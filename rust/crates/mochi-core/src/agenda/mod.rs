//! 日程待办。
//!
//! 两个并列、相互关联的模块：
//! - **事项**（[`Wish`] 愿望 / [`Goal`] 目标 / [`Task`] 任务）管理事情本身；
//! - **日程**（[`Entry`] 日程与时间块 / [`Routine`] 重复安排）管理时间。
//!
//! 时间块通过 `task_id` 引用同一份任务数据，从不复制任务。所有写操作见 [`ops`]，
//! 计算展示信息见 [`query`]，落盘见 [`store`]。

pub mod batch;
pub mod change;
pub mod legacy;
pub mod model;
pub mod ops;
pub mod parse;
pub mod planner;
pub mod proposal;
pub mod query;
pub mod recur;
pub mod store;
pub mod time;

#[cfg(test)]
mod tests;

pub use change::{ChangeSet, RecordChange};
pub use model::*;
pub use ops::{
    Clock, Editor, EntryDraft, FutureBlocks, GoalDraft, ProjectDraft, RoutineDraft, TaskDraft,
    WishDraft,
};
pub use proposal::Proposal;
pub use store::{with_write_lock, AgendaStore, LogEntry, Source};
