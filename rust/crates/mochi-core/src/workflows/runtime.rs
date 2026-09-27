//! 启动后台工作流运行任务，并跟踪执行、取消和定时状态。
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
    Arc,
};
use std::time::Duration;

pub struct Runtime {
    stop: Arc<AtomicBool>,
    pub events: Receiver<String>,
}
impl Runtime {
    pub fn start(store: Store, host: Arc<dyn NodeHost>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let halt = stop.clone();
        let (tx, events) = mpsc::channel();
        let scheduler_stop = stop.clone();
        let scheduler_store = store.clone();
        let scheduler_tx = tx.clone();
        std::thread::spawn(move || {
            while !scheduler_stop.load(Ordering::Relaxed) {
                if let Err(e) = schedule(&scheduler_store, now()) {
                    let _ = scheduler_tx.send(e);
                }
                for _ in 0..60 {
                    if scheduler_stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        });
        std::thread::spawn(move || {
            while !halt.load(Ordering::Relaxed) {
                match store.claim() {
                    Ok(Some(run)) => {
                        let id = run.id.clone();
                        let message =
                            match execute_monitored(&store, run, host.as_ref(), halt.clone()) {
                                Ok(_) => id,
                                Err(e) => e,
                            };
                        let _ = tx.send(message);
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(250)),
                    Err(e) => {
                        let _ = tx.send(e);
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
            }
        });
        Self { stop, events }
    }
}

pub fn execute_monitored(
    store: &Store,
    run: Run,
    host: &dyn NodeHost,
    stop: Arc<AtomicBool>,
) -> Result<Run> {
    let cancel = Arc::new(AtomicBool::new(false));
    let monitor_cancel = cancel.clone();
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let monitor_store = store.clone();
    let id = run.id.clone();
    let monitor = std::thread::spawn(move || {
        let mut beat = 0;
        while !finished.load(Ordering::Relaxed) {
            if stop.load(Ordering::Relaxed) || monitor_store.cancelled(&id) {
                monitor_cancel.store(true, Ordering::Relaxed);
            }
            if now() >= beat {
                let _ = monitor_store.heartbeat(&id);
                beat = now() + 10_000;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(store, run, host, cancel)
    }));
    done.store(true, Ordering::Relaxed);
    let _ = monitor.join();
    outcome.map_err(|_| "工作流执行线程异常，运行记录将在超时后标为中断".to_string())?
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn schedule(store: &Store, millis: i64) -> Result<()> {
    store.recover_stale()?;
    for summary in store.summaries()?.into_iter().filter(|w| w.enabled) {
        let saved = store.get(&summary.id)?;
        if let Some(slot) = due_slot(&saved.definition.trigger, millis) {
            store.enqueue(
                &saved.definition.id,
                serde_json::json!({}),
                "schedule",
                Some(&slot),
            )?;
        }
    }
    Ok(())
}
