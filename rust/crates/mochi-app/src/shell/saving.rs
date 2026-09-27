//! 保存当前标签页内容，并处理磁盘冲突和写入后的状态更新。
use super::*;

impl Shell {
    /// 把当前标签写回磁盘。写成功才清掉脏标记——
    /// 失败还清的话，用户会以为已经保存了。
    pub fn save_active(&mut self) -> bool {
        let Some(index) = self.active_tab else {
            return false;
        };
        self.save_tab(index)
    }

    pub fn save_tab(&mut self, index: usize) -> bool {
        self.save_tab_inner(index, false)
    }

    pub fn force_save_file(&mut self, path: &Path) -> bool {
        let Some(index) = self
            .tabs
            .iter()
            .position(|t| t.path().is_some_and(|open| same_native_path(open, path)))
        else {
            return false;
        };
        self.save_tab_inner(index, true)
    }

    pub fn has_disk_conflict(&self, path: &Path) -> bool {
        self.tabs
            .iter()
            .find(|t| t.path().is_some_and(|open| same_native_path(open, path)))
            .is_some_and(|t| match &t.kind {
                TabKind::Viewer {
                    content: crate::ui::viewer::Content::Base(s),
                    ..
                } => std::fs::read_to_string(path)
                    .map(|raw| raw != s.saved_raw)
                    .unwrap_or(true),
                _ => t.buffer().is_some_and(|b| {
                    std::fs::read_to_string(path)
                        .map(|s| !b.matches_saved(&s))
                        .unwrap_or(true)
                }),
            })
    }

    pub(super) fn save_tab_inner(&mut self, index: usize, force: bool) -> bool {
        self.save_tab_content(index, force, true)
    }

    pub(super) fn update_file_buffer_after_write(&mut self, index: usize, content: &str) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        if let TabKind::File { buffer, .. } = &mut tab.kind {
            if buffer.text() != content {
                let cursor = buffer.cursor().min(content.len());
                let length = buffer.text().len();
                buffer.replace_range(0..length, content);
                buffer.set_cursor(cursor, false);
            }
            buffer.mark_saved();
        }
        self.sync_clean_navigation_copies(index, content);
    }

    // 内链维护保留被引用文档原有的格式。
    // 用户保存可以触发格式升级；重命名则不能递归改掉其他文件。
    pub(super) fn save_tab_content(
        &mut self,
        index: usize,
        force: bool,
        upgrade_format: bool,
    ) -> bool {
        let git = self.workspace.as_ref().map(|ws| Arc::clone(&ws.git));
        let Some(tab) = self.tabs.get_mut(index) else {
            return false;
        };
        if let TabKind::Viewer {
            path,
            content: crate::ui::viewer::Content::Base(state),
        } = &mut tab.kind
        {
            if !force {
                match std::fs::read_to_string(&*path) {
                    Ok(raw) if raw != state.saved_raw => {
                        if state.dirty {
                            self.status = "保存失败：磁盘文件已变化，多维表格本地编辑已保留；请按 Ctrl+S 选择版本".into();
                            return false;
                        }
                        match crate::ui::base_view::State::parse(raw) {
                            Ok(next) => {
                                *state = next;
                                return true;
                            }
                            Err(error) => {
                                self.status =
                                    format!("磁盘多维表格格式无效，本地内容已保留：{error}");
                                return false;
                            }
                        }
                    }
                    Err(error) => {
                        self.status = format!("保存失败：文件已移动、删除或不可读：{error}");
                        return false;
                    }
                    _ => {}
                }
            }
            let raw = match mochi_core::base::serialize_base_document(&state.document) {
                Ok(raw) => raw,
                Err(error) => {
                    self.status = format!("多维表格校验失败：{error}");
                    return false;
                }
            };
            match FileService::new().write_file_safe(&*path, &raw) {
                Ok(()) => {
                    state.saved_raw = raw.clone();
                    state.dirty = false;
                    state.error.clear();
                    if let Some(git) = git {
                        git.mark_write(path.clone());
                    }
                    self.sync_clean_navigation_copies(index, &raw);
                    return true;
                }
                Err(error) => {
                    state.error = format!("保存失败：{error}");
                    self.status = state.error.clone();
                    return false;
                }
            }
        }

        let TabKind::File { path, buffer, .. } = &mut tab.kind else {
            return false;
        };
        if !force {
            match std::fs::read_to_string(&*path) {
                Ok(disk) if !buffer.matches_saved(&disk) => {
                    if buffer.dirty() || buffer.composition().is_some() {
                        self.status =
                            "保存失败：磁盘文件已变化，本地编辑已保留；请按 Ctrl+S 选择版本".into();
                        return false;
                    }
                    let cursor = buffer.cursor();
                    let length = buffer.text().len();
                    buffer.replace_range(0..length, &disk);
                    buffer.set_cursor(cursor, false);
                    buffer.mark_saved();
                    return true;
                }
                Err(error) => {
                    self.status =
                        format!("保存失败：文件已移动、删除或不可读，请按 Ctrl+S 处理：{error}");
                    return false;
                }
                _ => {}
            }
        }

        // 在调用 move_path 之前把路径和文本先拷出来。该操作会更新所有
        // 打开的页签、还可能保存干净的引用页签，因此必须在本页签
        // 可变借用结束之后执行。
        //
        // 文档源码总是原样保存。块身份过去曾持久化为
        // `<!-- mochi:block ... -->` 注释，既污染用户的 Markdown，
        // 又可能被一次无关的保存重新写进去。
        let path = path.clone();
        let content = buffer.text().to_owned();
        let should_upgrade = upgrade_format
            && path.extension().is_some_and(|extension| {
                extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("txt")
            })
            && mochi_core::document_format::requires_mochi_format(&content);

        if should_upgrade {
            let mut upgraded_path = path.clone();
            upgraded_path.set_extension("mc");

            // 写源文件之前先做这个检查。已存在的目标绝不覆盖，
            // 编辑器缓冲保持脏状态。
            if upgraded_path.exists() {
                self.status = format!("保存失败：目标文件已存在：{}", upgraded_path.display());
                return false;
            }
            if let Some(conflict) = companion_target_conflict(&path, &upgraded_path) {
                self.status = format!("保存失败：伴生数据目标已存在：{}", conflict.display());
                return false;
            }

            if let Err(error) = FileService::new().write_file_safe(&path, &content) {
                self.status = format!("保存失败：{error}");
                return false;
            }
            self.update_file_buffer_after_write(index, &content);
            if let Some(buffer) = self.tabs.get_mut(index).and_then(OpenTab::buffer_mut) {
                buffer.mark_written_pending_migration();
            }

            if let Err(error) = self.move_path(&path, &upgraded_path) {
                // 源文件写入成功，但路径迁移没有；保持缓冲为脏，
                // 用户可以重试或自行处理。
                self.status = format!("保存失败：无法升级文档格式：{error}");
                return false;
            }

            self.document_format_changes.push((path, upgraded_path));
            // 重命名可能更新了缓冲里的自链接或子引用。
            // 在清除脏状态之前，先把最终文字持久化到新路径。
            return self.save_tab_content(index, false, false);
        }

        match FileService::new().write_file_safe(&path, &content) {
            Ok(()) => {
                self.update_file_buffer_after_write(index, &content);
                // 登记到版本历史的待提交队列；真正的提交由自动提交或「记录版本」完成
                if let Some(git) = git {
                    git.mark_write(path);
                }
                true
            }
            Err(error) => {
                self.status = format!("保存失败：{error}");
                false
            }
        }
    }

    pub fn accept_written_text(&mut self, path: &Path, content: &str) {
        for tab in &mut self.tabs {
            if let TabKind::Viewer {
                path: open,
                content: crate::ui::viewer::Content::Base(state),
            } = &mut tab.kind
            {
                if same_native_path(open, path) {
                    match crate::ui::base_view::State::parse(content.to_owned()) {
                        Ok(next) => *state = next,
                        Err(error) => {
                            state.error = format!("文件校验失败，本地内容已保留：{error}")
                        }
                    }
                }
                continue;
            }
            if let TabKind::File {
                path: open, buffer, ..
            } = &mut tab.kind
            {
                if same_native_path(open, path) {
                    let cursor = buffer.cursor();
                    let length = buffer.text().len();
                    buffer.replace_range(0..length, content);
                    buffer.set_cursor(cursor, false);
                    buffer.mark_saved();
                }
            }
        }
        if let Some(git) = self.git() {
            git.mark_write(path.to_path_buf());
        }
    }

    /// 自动保存：把所有有未保存修改的文件标签写盘。任一失败即返回错误（其余仍会尝试）。
    pub fn save_dirty_tabs(&mut self) -> std::io::Result<()> {
        let mut first_err = None;
        let indices = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.dirty())
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        for i in indices {
            if !self.save_tab(i) {
                first_err.get_or_insert(std::io::Error::other(self.status.clone()));
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}
