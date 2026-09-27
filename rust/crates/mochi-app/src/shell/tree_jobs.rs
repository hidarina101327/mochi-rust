//! 浅层目录读取在后台线程执行。加载到的子项直接挂在树上；
//! 折叠时保留，整树替换时丢弃所有未完成的结果。
use super::*;
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Default)]
pub(super) struct TreeLoads {
    pub(super) expand_all: bool,
    pub(super) queued: VecDeque<PathBuf>,
    pub(super) running: HashMap<PathBuf, Receiver<Result<Vec<FileNode>>>>,
}

pub(super) fn expandable_paths(nodes: &[FileNode]) -> Vec<PathBuf> {
    fn walk(nodes: &[FileNode], paths: &mut Vec<PathBuf>) {
        for node in nodes {
            if node.lazy
                || node
                    .children
                    .as_ref()
                    .is_some_and(|children| !children.is_empty())
            {
                paths.push(PathBuf::from(&node.path));
            }
            if let Some(children) = &node.children {
                walk(children, paths);
            }
        }
    }
    let mut paths = Vec::new();
    walk(nodes, &mut paths);
    paths
}

pub(super) fn find_node_mut<'a>(
    nodes: &'a mut [FileNode],
    path: &Path,
) -> Option<&'a mut FileNode> {
    for node in nodes {
        if Path::new(&node.path) == path {
            return Some(node);
        }
        if let Some(children) = &mut node.children {
            if let Some(found) = find_node_mut(children, path) {
                return Some(found);
            }
        }
    }
    None
}

impl Shell {
    pub(super) fn expand_tree_paths(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            self.active_expanded_mut().insert(path.clone());
            self.request_tree_children(path);
        }
    }
    pub(super) fn tree_node_is_lazy(&mut self, path: &Path) -> bool {
        self.workspace
            .as_mut()
            .and_then(|ws| find_node_mut(&mut ws.tree, path))
            .is_some_and(|node| node.lazy)
    }

    pub(super) fn cache_tree_children(&mut self, path: &Path, children: Vec<FileNode>) -> bool {
        let Some(node) = self
            .workspace
            .as_mut()
            .and_then(|ws| find_node_mut(&mut ws.tree, path))
        else {
            return false;
        };
        if !node.lazy {
            return false;
        }
        node.children = Some(children);
        node.lazy = false;
        true
    }

    pub(super) fn request_tree_children(&mut self, path: PathBuf) {
        if !self.tree_node_is_lazy(&path)
            || self.tree_loads.running.contains_key(&path)
            || self.tree_loads.queued.contains(&path)
        {
            return;
        }
        self.tree_loads.queued.push_back(path);
        self.start_tree_loads();
    }

    fn start_tree_loads(&mut self) {
        let Some(root) = self.workspace.as_ref().map(|ws| ws.root.clone()) else {
            return;
        };
        // 限制并发工作线程的数量，避免快速展开很多文件夹时不断创建线程。
        while self.tree_loads.running.len() < 2 {
            let Some(path) = self.tree_loads.queued.pop_front() else {
                break;
            };
            if !self.tree_node_is_lazy(&path) {
                continue;
            }
            let (tx, rx) = mpsc::channel();
            let failure_tx = tx.clone();
            let worker_root = root.clone();
            let worker_path = path.clone();
            match std::thread::Builder::new()
                .name("mochi-tree-read".into())
                .spawn(move || {
                    let _ = tx.send(tree_loading::read_children(&worker_root, &worker_path));
                }) {
                Ok(_) => {}
                Err(error) => {
                    let _ = failure_tx.send(Err(error.into()));
                }
            }
            self.tree_loads.running.insert(path, rx);
        }
    }

    pub fn tree_loading_pending(&self) -> bool {
        !self.tree_loads.running.is_empty() || !self.tree_loads.queued.is_empty()
    }

    pub fn tree_loading_paths(&self) -> impl Iterator<Item = &Path> {
        self.tree_loads
            .running
            .keys()
            .chain(self.tree_loads.queued.iter())
            .map(PathBuf::as_path)
    }

    /// 绝不阻塞等磁盘。旧树的读取结果已经没有接收方了。
    pub fn poll_tree_loads(&mut self) -> bool {
        let ready = self
            .tree_loads
            .running
            .iter()
            .filter_map(|(path, rx)| match rx.try_recv() {
                Ok(result) => Some((path.clone(), result)),
                Err(TryRecvError::Disconnected) => {
                    Some((path.clone(), Err(anyhow::anyhow!("目录读取中断"))))
                }
                Err(TryRecvError::Empty) => None,
            })
            .collect::<Vec<_>>();
        let selected_path = self
            .selected
            .and_then(|i| self.rows.get(i))
            .map(|r| r.path.clone());
        let mut changed = false;
        for (path, result) in ready {
            self.tree_loads.running.remove(&path);
            match result {
                Ok(children) => {
                    let expand = if self.tree_loads.expand_all {
                        expandable_paths(&children)
                    } else {
                        Vec::new()
                    };
                    if self.cache_tree_children(&path, children) {
                        changed = true;
                        self.expand_tree_paths(expand);
                    }
                }
                Err(error) => {
                    if self.active_expanded_mut().remove(&path) {
                        self.status = format!("无法读取目录 {}：{error}", path.display());
                        changed = true;
                    }
                }
            }
        }
        self.start_tree_loads();
        if changed {
            self.rebuild_rows();
            self.selected = selected_path.and_then(|p| self.rows.iter().position(|r| r.path == p));
        }
        changed
    }

    #[cfg(test)]
    pub fn toggle_loaded(&mut self, row: usize) {
        self.toggle(row);
        self.wait_for_tree_loads();
    }

    #[cfg(test)]
    pub fn wait_for_tree_loads(&mut self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.tree_loading_pending() {
            assert!(std::time::Instant::now() < deadline, "tree read timed out");
            self.poll_tree_loads();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}
