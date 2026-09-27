//! 处理工作流节点的添加、删除和撤销操作。
use super::*;

impl State {
    pub fn checkpoint(&mut self) {
        if let Some(draft) = &self.draft {
            self.undo.push(draft.clone());
            if self.undo.len() > 50 {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.dirty = true;
        }
    }
    pub fn undo(&mut self, redo: bool) {
        self.selected_binding = None;
        if self.run.is_some() {
            return;
        }
        let next = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        if let Some(next) = next {
            self.group.clear();
            self.selected_edge = None;
            if let Some(old) = self.draft.replace(next) {
                if redo {
                    self.undo.push(old)
                } else {
                    self.redo.push(old)
                }
            }
            self.dirty = true;
            self.editor = None;
            self.selected_node = None;
        }
    }

    pub fn add_node(&mut self, kind: &str) {
        if self.run.is_some() {
            return;
        }
        if self.insert_edge.is_some() && matches!(kind, "start" | "end") {
            self.error = "连线中间请插入有输入和输出的节点".into();
            return;
        }
        self.checkpoint();
        let x = (self.canvas.width() * 0.35 - self.pan.0) / self.zoom;
        let y = (self.canvas.height() * 0.4 - self.pan.1) / self.zoom;
        if let Some(d) = &mut self.draft {
            if d.nodes.len() >= 100 {
                self.error = "最多 100 个节点".into();
                return;
            }
            let id = mochi_core::workflows::new_id(kind);
            d.nodes.push(Node::new(&id, kind, x, y));
            if let Some(edge) = self.insert_edge.take().and_then(|i| d.edges.get_mut(i)) {
                let target = std::mem::replace(&mut edge.target, id.clone());
                d.edges.push(Edge {
                    id: mochi_core::workflows::new_id("edge"),
                    source: id,
                    target,
                    source_handle: if kind == "condition" {
                        Some("true".into())
                    } else {
                        None
                    },
                });
            }
            let i = d.nodes.len() - 1;
            self.select_node(i);
        }
        self.palette = self.area.width() >= 1100.;
    }
    pub fn remove_selected(&mut self) {
        if let Some((target, key)) = self.selected_binding.clone() {
            self.remove_binding(target, &key);
            return;
        }
        if self.run.is_some() {
            return;
        }
        self.checkpoint();
        if !self.group.is_empty() {
            if let Some(g) = &mut self.draft {
                let ids: std::collections::HashSet<_> = self
                    .group
                    .iter()
                    .filter_map(|i| g.nodes.get(*i).map(|n| n.id.clone()))
                    .collect();
                g.nodes.retain(|n| !ids.contains(&n.id));
                g.edges
                    .retain(|e| !ids.contains(&e.source) && !ids.contains(&e.target));
            }
            self.group.clear();
            self.selected_node = None;
            self.selected_edge = None;
            self.editor = None;
            return;
        }
        if let Some(d) = &mut self.draft {
            if let Some(i) = self.selected_node.take() {
                if i < d.nodes.len() {
                    let id = d.nodes.remove(i).id;
                    d.edges.retain(|e| e.source != id && e.target != id);
                }
            } else if let Some(i) = self.selected_edge.take() {
                if i < d.edges.len() {
                    d.edges.remove(i);
                }
            }
        }
        self.editor = None;
    }
}
