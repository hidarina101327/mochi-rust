//! 提供工作流的命令行管理、校验、运行和状态查询入口。
use mochi_core::{
    settings::SettingsService,
    workflows::{self, native_host::NativeHost, Store, Workflow},
};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

fn read_json(path: &str) -> Result<Value, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > 524288 {
        return Err("JSON 文件超过 512 KiB".into());
    }
    serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{}", json!({"ok":false,"error":error}));
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" {
        println!("mochi-workflow <workspace> <list|get ID|import FILE|validate FILE|save FILE [REVISION]|run ID [INPUT.json]|enable ID|pause ID|history ID|result RUN_ID|cancel RUN_ID|tick|worker|catalog>\nSaved and imported workflows run as trusted scripts without additional approval. enable/pause control scheduling. worker runs until stopped; tick enqueues due schedules and drains the queue.");
        return Ok(());
    }
    let root = Path::new(&args[0]);
    if !root.is_dir() {
        return Err("工作区不存在".into());
    }
    let command = args.get(1).map(String::as_str).unwrap_or("list");
    let arg = || {
        args.get(2)
            .map(String::as_str)
            .ok_or("缺少参数".to_string())
    };
    let store = Store::open(root)?;
    let settings = Arc::new(SettingsService::new(
        std::env::var_os("MOCHI_SETTINGS_FILE").map(Into::into),
    ));
    let output = match command {
        "catalog" => workflows::catalog::describe(),
        "list" => json!(store.summaries()?),
        "get" => json!(store.get(arg()?)?),
        "import" => json!(store.import(&read_json(arg()?)?.to_string())?),
        "validate" => {
            let flow: Workflow =
                serde_json::from_value(read_json(arg()?)?).map_err(|e| e.to_string())?;
            json!({"valid":true,"order":workflows::validate(&flow)?})
        }
        "save" => {
            let flow: Workflow =
                serde_json::from_value(read_json(arg()?)?).map_err(|e| e.to_string())?;
            let revision = args
                .get(3)
                .map(|s| s.parse::<i64>().map_err(|_| "revision 无效".to_string()))
                .transpose()?;
            json!(store.save(&flow, revision)?)
        }
        "run" => {
            let input = args
                .get(3)
                .map(|p| read_json(p))
                .transpose()?
                .unwrap_or(json!({}));
            let run = store
                .enqueue(arg()?, input, "cli", None)?
                .ok_or("工作流已在执行")?;
            json!({"runId":run.id,"status":"queued","next":"Run worker or keep the Mochi application open"})
        }
        "enable" => {
            let saved = store.get(arg()?)?;
            store.set_schedule(arg()?, saved.revision, true)?;
            json!({"enabled":store.get(arg()?)?.enabled})
        }
        "pause" => {
            store.pause(arg()?)?;
            json!({"enabled":false})
        }
        "history" => json!(store.history(arg()?)?),
        "result" => json!(store.run(arg()?)?),
        "cancel" => {
            store.cancel(arg()?)?;
            json!({"cancellationRequested":true})
        }
        "worker" => {
            let worker = workflows::runtime::Runtime::start(
                store,
                Arc::new(NativeHost::new(root, settings)?),
            );
            loop {
                if let Ok(message) = worker.events.recv_timeout(Duration::from_secs(30)) {
                    println!("{}", json!({"event":message}));
                }
            }
        }
        "tick" => {
            workflows::runtime::schedule(&store, workflows::now())?;
            let host = NativeHost::new(root, settings)?;
            let mut runs = Vec::new();
            while let Some(run) = store.claim()? {
                let done = workflows::runtime::execute_monitored(
                    &store,
                    run,
                    &host,
                    Arc::new(AtomicBool::new(false)),
                )?;
                runs.push(json!({"runId":done.id,"status":done.status}));
            }
            json!({"runs":runs})
        }
        _ => return Err("未知命令；使用 --help 查看帮助".into()),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({"ok":true,"data":output}))
            .map_err(|e| e.to_string())?
    );
    Ok(())
}
