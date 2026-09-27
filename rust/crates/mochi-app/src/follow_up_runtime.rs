//! 追问独立请求：超时 10 秒、不重试、不注册工具；失败不影响主回答及其保存。

use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
    Arc,
};
use std::time::Duration;

use mochi_core::ai::{
    follow_up::validate_follow_up_response,
    models::{AiCompletionRequest, AiProvider},
    service::AiService,
};

pub const TIMEOUT_SECONDS: u64 = 10;

#[derive(Debug, Clone)]
pub struct FollowUpTarget {
    pub session_id: String,
    pub message_id: String,
    pub revision: String,
}

pub struct FollowUpJobResult {
    pub target: FollowUpTarget,
    pub outcome: Result<Vec<String>, String>,
}

pub struct FollowUpJob {
    pub target: FollowUpTarget,
    cancel: Arc<AtomicBool>,
    rx: Receiver<FollowUpJobResult>,
}

impl FollowUpJob {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn finish(&self) -> Option<FollowUpJobResult> {
        match self.rx.try_recv() {
            Ok(res) => Some(res),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(FollowUpJobResult {
                target: self.target.clone(),
                outcome: Err("追问生成任务已中断".into()),
            }),
        }
    }
}

impl Drop for FollowUpJob {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub fn spawn(
    target: FollowUpTarget,
    provider: AiProvider,
    request: AiCompletionRequest,
    wake: impl Fn() + Send + 'static,
) -> FollowUpJob {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let (tx, rx) = mpsc::channel();
    let worker_target = target.clone();

    std::thread::Builder::new()
        .name("mochi-follow-up".into())
        .spawn(move || {
            if flag.load(Ordering::Relaxed) {
                return;
            }
            let service = AiService::with_cancel_timeout(
                provider,
                flag.clone(),
                Duration::from_secs(TIMEOUT_SECONDS),
            );
            let outcome = match service.complete_once(&request) {
                Ok(resp) => validate_follow_up_response(&resp.content),
                Err(err) => Err(format!("追问请求失败: {err}")),
            };

            if !flag.load(Ordering::Relaxed) {
                let _ = tx.send(FollowUpJobResult {
                    target: worker_target,
                    outcome,
                });
                wake();
            }
        })
        .expect("failed to spawn follow-up worker thread");

    FollowUpJob { target, cancel, rx }
}
