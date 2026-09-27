//! 打开块对象链接，并从日程或文档位置创建链接。
use super::*;

impl App {
    /// 打开持久化的块 URL，并在正式编辑器中选中对应的精确源码区间。对每个
    /// 语法合法的块 URL 都返回 `true`，可以让 `App::open_link` 在通用
    /// `mochi://open` 兜底之前停下；失败时报告在状态栏，而不是只打开所属
    /// 文档、丢掉块的身份。
    pub(super) fn open_block_object_url(&mut self, target: &str) -> bool {
        let Some(reference) = mochi_core::object_reference::ObjectReference::parse(target) else {
            return false;
        };
        if reference.kind != mochi_core::object_reference::ObjectKind::Block {
            return false;
        }
        let Some(workspace) = self
            .shell
            .workspace()
            .map(|workspace| workspace.root.clone())
        else {
            self.state.status_text = "无法打开文档块：没有工作区".into();
            return true;
        };
        let Some(path) = reference.resolve_path(Some(&workspace)) else {
            self.state.status_text = "无法打开文档块：路径无效".into();
            return true;
        };
        let root = workspace.canonicalize().unwrap_or(workspace.clone());
        let existing = match path.canonicalize() {
            Ok(path) if path.is_file() && mochi_core::paths::path_is_within(&root, &path) => path,
            _ => {
                self.state.status_text = format!(
                    "无法打开文档块：找不到工作区内的文档 {}",
                    reference.path.as_deref().unwrap_or_default()
                );
                return true;
            }
        };
        // `canonicalize` 可能加上 Windows 的 `\\?\` 前缀。只把它用于工作区
        // 边界检查；Shell 的标签去重会保留已打开路径的原有写法，若直接打开
        // 规范化路径会多建一个标签页并丢掉脏缓冲区。
        let open_path = self
            .shell
            .tabs()
            .iter()
            .filter_map(|tab| tab.path())
            .find(|open| open.canonicalize().ok().as_deref() == Some(existing.as_path()))
            .map(Path::to_path_buf)
            .unwrap_or(path);
        if !self.open_file_from_ui(&open_path) {
            self.state.status_text = "无法打开文档块所在文档".into();
            return true;
        }
        self.state.view = WorkspaceView::Editor;
        self.editor_engaged = true;
        let source_path = self.active_file_path().unwrap_or(open_path);

        let range = self
            .shell
            .active()
            .and_then(|tab| tab.buffer())
            .and_then(|buffer| {
                let document =
                    mochi_core::document_blocks::document_from_source(&source_path, buffer.text())
                        .ok()?;
                let id = reference.block_id.as_deref()?;
                let block = document.find_block(id).ok()?;
                block.marker_span?;
                Some((block.source_span.start, block.source_span.end))
            });
        let Some((start, end)) = range else {
            self.state.status_text = format!(
                "无法定位文档块 {}：可能尚未转换或已被删除",
                reference.block_id.as_deref().unwrap_or_default()
            );
            self.invalidate_main();
            self.sync_state();
            return true;
        };
        if let Some(buffer) = self.shell.active_buffer_mut() {
            buffer.set_cursor(start, false);
            buffer.set_cursor(end, true);
        }
        self.focus = Focus::Main;
        self.after_doc_selection_change(false);
        self.state.status_text = format!(
            "已定位到文档块 {}",
            reference.block_id.as_deref().unwrap_or_default()
        );
        self.invalidate_main();
        self.sync_state();
        true
    }

    pub(super) fn schedule_link_at(&self, x: f32, y: f32) -> Option<String> {
        if self.state.view != WorkspaceView::Schedule {
            return None;
        }
        use crate::ui::agenda::Hit;
        let id = match self.sched.layout.hit(x, y)? {
            Hit::Slot(id) | Hit::Todo(id) | Hit::Open(id) | Hit::Check(id) => id,
            _ => return None,
        };
        let data = self.sched.view.data.as_ref()?;
        let id = match data.entry(&id).and_then(|e| e.task_id.clone()) {
            Some(task) => task,
            None if id.contains('@') => id.split('@').next()?.to_owned(),
            None => id,
        };
        let kind = match mochi_core::agenda::Kind::of_id(&id)? {
            mochi_core::agenda::Kind::Task => "task",
            mochi_core::agenda::Kind::Project => "project",
            _ => "event",
        };
        let title = data.title_of(&id)?;
        Some(format!(
            "mochi://open?path=agenda&kind={kind}&item={}&label={}",
            mochi_url::form_encode(&id),
            mochi_url::form_encode(&title)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 打开块链接必须对照活动标签页来解析身份。尤其是等价的路径写法
    /// （这里是 `sub/../note.md`）必须复用现有的脏标签页，而不是把规范化的
    /// `\\?\` 路径丢给 Shell 再多开一份。
    #[test]
    fn block_url_reuses_dirty_tab_and_selects_the_live_block_range() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let root = std::env::temp_dir().join(format!(
            "mochi-block-link-app-{}",
            mochi_core::paths::random_base36(12)
        ));
        std::fs::create_dir_all(&root).unwrap();
        {
            let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
                root.join("settings.json"),
            ))))
            .unwrap();
            app.shell.open_workspace(&root, || {}).unwrap();

            // 先准备好真实落盘的源码，生成的 ID 与编辑器转换流程写出的
            // 完全一致。
            let path = root.join("note.md");
            let conversion = mochi_core::document_blocks::prepare_conversion(
                &path,
                "第一块（磁盘版本）\n\n第二块（磁盘版本）\n",
            )
            .unwrap();
            std::fs::write(&path, &conversion.converted_source).unwrap();
            let persisted = mochi_core::document_blocks::document_from_source(
                &path,
                &conversion.converted_source,
            )
            .unwrap();
            let target = persisted.blocks[0].id.to_string();

            // 等工作区迁移跑完再建别名，这样它才是真正的等价路径写法，
            // 而不是有待迁移的知识库。
            let subdir = root.join("sub");
            std::fs::create_dir_all(&subdir).unwrap();
            let alias = subdir.join("..").join("note.md");
            assert!(app.shell.open_file(&alias));
            let dirty = conversion
                .converted_source
                .replace("第一块（磁盘版本）", "第一块（未保存的当前版本😀）");
            let buffer = app.shell.active_buffer_mut().unwrap();
            buffer.replace_range(0..buffer.text().len(), &dirty);
            assert!(buffer.dirty());
            let before_text = buffer.text().to_owned();
            let before_tabs = app.shell.tabs().len();
            let url = mochi_core::object_reference::build_block_reference(
                &path,
                &target,
                Some(&root),
                Some("当前块"),
            )
            .unwrap();

            assert!(app.open_block_object_url(&url));
            assert_eq!(app.shell.tabs().len(), before_tabs);
            let active = app.shell.active().unwrap();
            assert_eq!(active.path(), Some(alias.as_path()));
            let buffer = active.buffer().unwrap();
            assert!(buffer.dirty());
            assert_eq!(buffer.text(), before_text);

            let live =
                mochi_core::document_blocks::document_from_source(&alias, &before_text).unwrap();
            let block = live.find_block(&target).unwrap();
            let expected = (block.source_span.start, block.source_span.end);
            assert_eq!(buffer.selection(), expected);
            assert_eq!(buffer.selected_text(), &before_text[expected.0..expected.1]);
            assert!(buffer.selected_text().contains("未保存的当前版本😀"));
            assert!(!buffer.selected_text().contains("磁盘版本"));
        }
        let resolved = root.canonicalize().unwrap();
        let temp = std::env::temp_dir().canonicalize().unwrap();
        assert!(
            resolved.starts_with(&temp)
                && resolved.file_name().is_some_and(|name| name
                    .to_string_lossy()
                    .starts_with("mochi-block-link-app-"))
        );
        std::fs::remove_dir_all(resolved).unwrap();
    }
}
