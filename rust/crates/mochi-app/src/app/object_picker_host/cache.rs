//! 缓存对象选择器的扫描结果，并管理异步刷新。
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};

use mochi_core::object_reference::ObjectCandidate;

use crate::ui::object_picker::reconcile_candidates;

type ScanResult = std::io::Result<Vec<ObjectCandidate>>;

#[derive(Default)]
pub(crate) struct Cache {
    // 限制内存占用，同时在近期工作区之间切换时保留快照。
    entries: VecDeque<Entry>,
}

struct Entry {
    workspace: PathBuf,
    candidates: Option<Vec<ObjectCandidate>>,
    pending: Option<Receiver<ScanResult>>,
}

impl Cache {
    pub(super) fn get(&self, workspace: &Path) -> Option<&Vec<ObjectCandidate>> {
        self.entries
            .iter()
            .find(|entry| entry.workspace == workspace)?
            .candidates
            .as_ref()
    }

    /// 每个工作区最多一个工作线程，包括关闭/重开的情形。
    pub(super) fn begin_refresh(&mut self, workspace: &Path) -> Option<Sender<ScanResult>> {
        let mut entry = self
            .entries
            .iter()
            .position(|entry| entry.workspace == workspace)
            .and_then(|index| self.entries.remove(index))
            .unwrap_or_else(|| Entry {
                workspace: workspace.to_path_buf(),
                candidates: None,
                pending: None,
            });
        let sender = if entry.pending.is_none() {
            let (tx, rx) = channel();
            entry.pending = Some(rx);
            Some(tx)
        } else {
            None
        };
        self.entries.push_back(entry);
        while self.entries.len() > 4 {
            self.entries.pop_front();
        }
        sender
    }

    pub(super) fn poll(&mut self) -> Vec<(PathBuf, bool)> {
        let mut updates = Vec::new();
        for entry in &mut self.entries {
            let Some(receiver) = &entry.pending else {
                continue;
            };
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => continue,
                Err(TryRecvError::Disconnected) => Err(std::io::Error::other("对象扫描已中断")),
            };
            entry.pending = None;
            let success = result.is_ok();
            if let Ok(candidates) = result {
                reconcile_candidates(entry.candidates.get_or_insert_with(Vec::new), candidates);
            }
            updates.push((entry.workspace.clone(), success));
        }
        updates
    }
}

#[cfg(test)]
mod tests;
