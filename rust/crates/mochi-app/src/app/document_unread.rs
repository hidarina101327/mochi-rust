//! 维护文档未读状态，并更新文件树中的未读标记。
use super::*;
use mochi_core::document_unread::{key, Store};

impl App {
    pub(super) fn refresh_document_unread(&mut self) {
        let root = self.shell.workspace().map(|ws| ws.root.clone());
        if self.nav.unread_root != root {
            self.nav.unread = Default::default();
            self.nav.unread_root = root.clone();
        }
        if let Some(root) = root {
            if let Ok(snapshot) = Store::new(&root).snapshot() {
                self.nav.unread = snapshot;
            }
        }
    }

    pub(super) fn acknowledge_visible_documents(&mut self) {
        if self.state.view != WorkspaceView::Editor
            || self.settings_overlay.is_some()
            || self.dialog.is_some()
            || self.search.is_some()
            || self.command.is_some()
            || self.object_picker.is_some()
            || self.global_import.is_some()
            || self.image_preview.is_some()
            || self.notification_open()
        {
            return;
        }
        let Some(root) = self.nav.unread_root.clone() else {
            return;
        };
        let active = self
            .shell
            .active()
            .and_then(|tab| tab.path())
            .map(Path::to_path_buf);
        let other = self.split.other.clone();
        let mut changed = false;
        for path in active.into_iter().chain(other) {
            if let Some(token) = self.nav.unread.documents.get(&key(&path)) {
                let ready = self
                    .shell
                    .tabs()
                    .iter()
                    .find(|tab| tab.path().is_some_and(|p| key(p) == key(&path)))
                    .is_some_and(|tab| match &tab.kind {
                        TabKind::File { buffer, .. } => std::fs::read_to_string(&path)
                            .is_ok_and(|text| buffer.matches_saved(&text)),
                        TabKind::Viewer { content, .. } => match content {
                            crate::ui::viewer::Content::Pdf(state) => {
                                !state.loading
                                    && state.error.is_none()
                                    && state.ready.iter().any(Option::is_some)
                            }
                            crate::ui::viewer::Content::Spreadsheet(state) => {
                                !state.loading && state.error.is_empty()
                            }
                            crate::ui::viewer::Content::Image(state) => state.natural.is_some(),
                            crate::ui::viewer::Content::Unsupported { .. } => false,
                            _ => true,
                        },
                        _ => false,
                    });
                if !ready {
                    continue;
                }
                changed |= Store::new(&root).read(&path, token).unwrap_or(false);
            }
        }
        if changed {
            self.refresh_document_unread();
        }
    }

    pub(super) fn show_document_unread(&self) -> bool {
        crate::ui::settings_values::boolean("sidebar.showUnreadIndicators", true)
    }

    pub(super) fn paint_document_unread_tree(&mut self, layout: &SidebarLayout, p: &Palette) {
        if !self.show_document_unread() {
            return;
        }
        self.list.push_clip(layout.content);
        for (rect, hit) in &layout.entries {
            if let SidebarHit::Row(index) = hit {
                if self
                    .shell
                    .rows()
                    .get(*index)
                    .is_some_and(|row| self.nav.unread.has(&row.path))
                {
                    self.list.rounded_rect(
                        Rect::from_size(rect.right - 10.0, rect.top + 4.0, 6.0, 6.0),
                        3.0,
                        p.accent,
                    );
                }
            }
        }
        self.list.pop_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reading_only_visible_documents_clears_ancestors_and_hiding_badges_keeps_state() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        let temp = std::env::temp_dir().join(format!(
            "mochi-unread-ui-{}",
            mochi_core::paths::new_library_id("test")
        ));
        std::fs::create_dir_all(&temp).unwrap();
        let mut app = App::with_settings(Arc::new(SettingsService::new(Some(
            temp.join("settings.json"),
        ))))
        .unwrap();
        app.shell.open_workspace(&temp, || {}).unwrap();
        let index = app.shell.create_library("knowledge-base", "情报").unwrap();
        let folder =
            PathBuf::from(&app.shell.workspace().unwrap().libraries[index].path).join("行业");
        std::fs::create_dir(&folder).unwrap();
        let a = folder.join("a.md");
        let b = folder.join("b.md");
        std::fs::write(&a, "# A\n内容").unwrap();
        std::fs::write(&b, "# B\n内容").unwrap();
        Store::new(&temp).mark(&[a.clone(), b.clone()]).unwrap();
        app.refresh_document_unread();
        assert!(app.nav.unread.has(&folder));
        app.shell.open_file_with_mode(&b, true);
        app.shell.open_file_with_mode(&a, true);
        app.state.view = WorkspaceView::Editor;
        app.shell.select_library(index);
        app.shell.refresh_tree();
        app.shell.expand_all();
        for _ in 0..200 {
            app.shell.poll_tree_loads();
            if !app.shell.tree_loading_pending() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.sync_state();
        let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/unread-qa");
        std::fs::create_dir_all(&output).unwrap();
        let snapshot = app.renderer.prepare_snapshot(1200, 800, 96.0).unwrap();
        app.paint(HWND::default()).unwrap();
        app.renderer
            .save_snapshot(&snapshot, &output.join("unread.png"))
            .unwrap();
        let dots = |app: &App| {
            app.list.cmds().iter().filter(|cmd| matches!(cmd,crate::ui::draw::DrawCmd::RoundedRect{rect,radius,..} if rect.width()==6.0 && rect.height()==6.0 && *radius==3.0)).count()
        };
        assert!(
            dots(&app) >= 5,
            "未读文档、文件夹和打开的文档标签应显示提示"
        );
        app.state.view = WorkspaceView::Home;
        app.acknowledge_visible_documents();
        assert_eq!(app.nav.unread.documents.len(), 2);
        app.state.view = WorkspaceView::Editor;
        app.acknowledge_visible_documents();
        assert!(!app.nav.unread.has(&a));
        assert!(app.nav.unread.has(&b));
        assert!(app.nav.unread.has(&folder));
        app.paint(HWND::default()).unwrap();
        app.renderer
            .save_snapshot(&snapshot, &output.join("one-read.png"))
            .unwrap();
        std::fs::write(&a, "# A 新版").unwrap();
        Store::new(&temp).mark(&[a.clone()]).unwrap();
        app.refresh_document_unread();
        app.acknowledge_visible_documents();
        assert!(app.nav.unread.has(&a), "尚未显示新正文时不能清除未读");
        app.shell.open_file(&a);
        app.acknowledge_visible_documents();
        assert!(!app.nav.unread.has(&a));
        app.settings
            .set("app.sidebar.showUnreadIndicators", "false");
        app.load_chrome_settings();
        assert!(!app.show_document_unread());
        assert!(Store::new(&temp).snapshot().unwrap().has(&b));
        app.paint(HWND::default()).unwrap();
        assert_eq!(dots(&app), 0);
        app.renderer
            .save_snapshot(&snapshot, &output.join("hidden.png"))
            .unwrap();
        app.shell.open_file(&b);
        app.acknowledge_visible_documents();
        assert!(app.nav.unread.is_empty());
    }
}
