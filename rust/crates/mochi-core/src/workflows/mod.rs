//! 导出工作流模块，并提供结果、时间和 ID 等通用辅助功能。
pub mod catalog;
mod engine;
mod model;
pub mod native_host;
mod nodes;
mod references;
pub mod runtime;
mod scheduler;
mod store;
pub mod templates;
mod validation;

pub use engine::{execute, NodeHost};
pub use model::*;
pub use references::resolve;
pub use scheduler::due_slot;
pub use store::{RunSummary, Store, WorkflowFolder, WorkflowOperation, WorkflowSummary};
pub use validation::{fingerprint, validate};

pub type Result<T> = std::result::Result<T, String>;
pub fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
pub fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!(
        "{prefix}_{}_{}_{}",
        now(),
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests;
