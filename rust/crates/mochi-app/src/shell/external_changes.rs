//! 响应外部文件变更，并刷新或重载受影响的标签页。
use super::*;

impl Shell {
    pub fn forget_tabs_under(&mut self, path: &Path) {
        self.close_tabs_under(path);
    }

    pub fn accept_external_rename(&mut self, from: &Path, to: &Path) {
        let old_side = PathBuf::from(mochi_core::sub_documents::sidecar_path(
            &from.to_string_lossy(),
        ));
        let new_side = PathBuf::from(mochi_core::sub_documents::sidecar_path(
            &to.to_string_lossy(),
        ));
        for tab in &mut self.tabs {
            tab.history.remap(from, to, &old_side, &new_side);
            let Some(path) = tab.path() else { continue };
            let changed = if path == from {
                Some(to.to_path_buf())
            } else {
                path.strip_prefix(from)
                    .ok()
                    .map(|rest| to.join(rest))
                    .or_else(|| {
                        path.strip_prefix(&old_side)
                            .ok()
                            .map(|rest| new_side.join(rest))
                    })
            };
            if let Some(next) = changed {
                tab.title = next
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                match &mut tab.kind {
                    TabKind::File { path, .. } => *path = next,
                    TabKind::Viewer { path, content } => {
                        if !matches!(content, crate::ui::viewer::Content::Base(_)) {
                            *content =
                                crate::ui::viewer::load(&next, &crate::ui::viewer::detect(&next));
                        }
                        *path = next;
                    }
                    _ => {}
                }
            }
        }
        remap_expanded_paths(&mut self.expanded, from, to);
        remap_expanded_paths(&mut self.favorites_expanded, from, to);
        if let Some(warning) = self.update_base_references_for_rename(from, to) {
            self.status = warning;
        }
        if let Some(warning) = self.remap_favorites(from, to) {
            self.status = warning;
        }
        self.refresh_tree();
    }

    pub fn accept_external_delete(&mut self, path: &Path) {
        self.remove_favorites_under(path);
        remove_expanded_under(&mut self.expanded, path);
        remove_expanded_under(&mut self.favorites_expanded, path);
        let mut i = 0;
        while i < self.tabs.len() {
            let tab = &self.tabs[i];
            let affected = tab
                .path()
                .is_some_and(|open| native_path_is_same_or_under(open, path));
            if affected && !tab.dirty() && tab.buffer().is_none_or(|b| b.composition().is_none()) {
                self.discard_tab(i);
            } else {
                i += 1;
            }
        }
        self.refresh_tree();
    }

    pub fn git(&self) -> Option<Arc<GitService>> {
        self.workspace.as_ref().map(|ws| Arc::clone(&ws.git))
    }

    pub fn refresh_visible_files(&mut self, other: Option<&Path>) -> bool {
        let active = self.active_tab;
        let mut changed = false;
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if Some(i) != active
                && !tab
                    .path()
                    .zip(other)
                    .is_some_and(|(open, changed)| same_native_path(open, changed))
            {
                continue;
            }
            if let TabKind::Viewer {
                path,
                content: crate::ui::viewer::Content::Base(state),
            } = &mut tab.kind
            {
                if let Ok(raw) = std::fs::read_to_string(&*path) {
                    if raw != state.saved_raw {
                        if state.dirty {
                            self.status =
                                "磁盘多维表格有新版本，本地编辑已保留；请按 Ctrl+S 处理".into();
                        } else {
                            match crate::ui::base_view::State::parse(raw) {
                                Ok(next) => {
                                    *state = next;
                                    changed = true;
                                }
                                Err(error) => {
                                    state.error =
                                        format!("磁盘文件校验失败，本地内容已保留：{error}");
                                }
                            }
                        }
                    }
                }
                continue;
            }
            let TabKind::File { path, buffer, .. } = &mut tab.kind else {
                continue;
            };
            let Ok(disk) = std::fs::read_to_string(&*path) else {
                continue;
            };
            if buffer.matches_saved(&disk) {
                continue;
            }
            if buffer.dirty() || buffer.composition().is_some() {
                self.status =
                    "保存失败：磁盘文件有新版本，本地编辑未被覆盖；请按 Ctrl+S 处理".into();
                continue;
            }
            let cursor = buffer.cursor();
            let length = buffer.text().len();
            buffer.replace_range(0..length, &disk);
            buffer.set_cursor(cursor, false);
            buffer.mark_saved();
            changed = true;
        }
        changed
    }

    /// 磁盘内容变了（版本回溯、AI 写入）：让打开着的标签重新读盘。
    /// 光标与滚动位置尽量保住，未保存的编辑会被覆盖——回溯本来就是要覆盖。
    pub fn reload_file(&mut self, path: &Path) {
        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };
        for tab in &mut self.tabs {
            if let TabKind::Viewer {
                path: open,
                content: viewer,
            } = &mut tab.kind
            {
                if same_native_path(open, path) {
                    match viewer {
                        crate::ui::viewer::Content::Base(state) => {
                            match crate::ui::base_view::State::parse(content.clone()) {
                                Ok(next) => *state = next,
                                Err(error) => {
                                    state.error = format!("文件校验失败，本地内容已保留：{error}");
                                    self.status = state.error.clone();
                                }
                            }
                        }
                        // 画布由 AI 或外部操作写入时，已打开的标签此前不会重新读盘，
                        // 于是会一直显示打开瞬间的空卡片列表。
                        crate::ui::viewer::Content::Canvas(state) => {
                            match crate::ui::canvas_view::State::parse(&content) {
                                Ok(next) => *state = next,
                                Err(error) => {
                                    self.status = format!("画布校验失败，本地内容已保留：{error}");
                                }
                            }
                        }
                        _ => {}
                    }
                }
                continue;
            }
            if let TabKind::File {
                path: p, buffer, ..
            } = &mut tab.kind
            {
                if same_native_path(p, path) {
                    let cursor = buffer.cursor().min(content.len());
                    *buffer = crate::ui::editor::TextBuffer::new(content.clone());
                    buffer.set_cursor(cursor, false);
                }
            }
        }
    }
}
