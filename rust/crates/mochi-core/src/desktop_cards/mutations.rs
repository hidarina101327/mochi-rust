//! 桌面卡片上的勾选：直接走日程待办的编辑器，和主界面、AI 共用一把写锁和一份日志。
//!
//! 勾选任务只改任务状态，未来的时间块原样保留（卡片上没有地方询问用户）；
//! 勾选日程只改这条日程自己的执行状态，不会顺带完成关联任务。

use anyhow::{bail, Result};
use std::path::Path;

use crate::agenda::{AgendaStore, EntryStatus, Kind, Source, TaskStatus};

/// 按 ID 切换一条日程待办的完成状态。
///
/// `id` 可以是任务 ID、日程 ID，或重复安排的发生键（`rtn-…@YYYY-MM-DD`，会先落地）。
/// 找到并更新返回 `true`；ID 不存在返回 `false`，什么都不写。
pub fn toggle_schedule_item(workspace: &Path, id: &str, done: bool) -> Result<bool> {
    let id = id.trim();
    if id.is_empty() {
        bail!("日程项 ID 不能为空");
    }
    let store = AgendaStore::new(workspace);
    let (found, _) = store.mutate(Source::Card, |ed| {
        if let Some((routine, _)) = id.split_once('@') {
            if ed.data.live_routine(routine).is_none() {
                return Ok(false);
            }
            ed.confirm(
                id,
                if done {
                    EntryStatus::Done
                } else {
                    EntryStatus::Planned
                },
            )?;
            return Ok(true);
        }
        match Kind::of_id(id) {
            Some(Kind::Task) if ed.data.task(id).is_some() => {
                ed.set_task_status(
                    id,
                    if done {
                        TaskStatus::Done
                    } else {
                        TaskStatus::Todo
                    },
                )?;
                Ok(true)
            }
            Some(Kind::Entry) if ed.data.entry(id).is_some() => {
                let status = if done {
                    EntryStatus::Done
                } else {
                    EntryStatus::Planned
                };
                ed.set_entry_status(id, status, None)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    })?;
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agenda::{EntryDraft, TaskDraft};
    use std::path::PathBuf;

    fn workspace(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "mochi-toggle-{tag}-{}",
            crate::paths::random_base36(8)
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn toggling_a_block_never_completes_its_task() {
        let root = workspace("block");
        let store = AgendaStore::new(&root);
        let ((task, block), _) = store
            .mutate(Source::User, |ed| {
                let task = ed.create_task(TaskDraft {
                    title: "写报告".into(),
                    ..Default::default()
                })?;
                let block = ed.schedule_task(&task, "2026-09-25T09:00", "2026-09-25T10:00")?;
                Ok((task, block))
            })
            .unwrap();
        assert!(toggle_schedule_item(&root, &block, true).unwrap());
        let data = store.load().unwrap();
        assert_eq!(data.entry(&block).unwrap().status, EntryStatus::Done);
        assert_eq!(data.task(&task).unwrap().status, TaskStatus::Todo);

        assert!(toggle_schedule_item(&root, &task, true).unwrap());
        assert!(toggle_schedule_item(&root, &task, false).unwrap());
        let data = store.load().unwrap();
        assert_eq!(data.task(&task).unwrap().status, TaskStatus::Todo);
        assert!(data.task(&task).unwrap().completed_at.is_none());
        assert_eq!(store.history(10)[0].source, Source::Card);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_item_returns_false_without_writing() {
        let root = workspace("missing");
        let store = AgendaStore::new(&root);
        store
            .mutate(Source::User, |ed| {
                ed.create_entry(EntryDraft {
                    title: "会".into(),
                    start: "2026-09-25T09:00".into(),
                    end: "2026-09-25T10:00".into(),
                    ..Default::default()
                })
            })
            .unwrap();
        let before = std::fs::read(store.data_path()).unwrap();
        assert!(!toggle_schedule_item(&root, "task-nope", true).unwrap());
        assert!(!toggle_schedule_item(&root, "rtn-nope@2026-09-25", true).unwrap());
        assert!(toggle_schedule_item(&root, " ", true).is_err());
        assert_eq!(std::fs::read(store.data_path()).unwrap(), before);
        let _ = std::fs::remove_dir_all(root);
    }
}
