//! 打开文档中的链接，并处理链接导航。
use super::*;

impl App {
    /// 链接去向分流（`services/mochiLink.ts` 的 `openLinkHref`）：Mochi 链接在应用内开标签/定位目录，
    /// http/https/mailto/tel 交给系统，维基链接按文件名在工作区里找；其它协议一律不理。
    pub(super) fn open_link(&mut self, target: &str) {
        if self.open_block_object_url(target) {
            return;
        }
        let url = target.trim();
        if let Some(reference) = mochi_core::object_reference::ObjectReference::parse(url) {
            if reference.kind == mochi_core::object_reference::ObjectKind::AiSession {
                if let Some(id) = reference.session_id {
                    self.ai_open_session(&id);
                    self.state.ai_panel_open = true;
                    self.set_right_panel(RightPanel::Assistant);
                    self.invalidate_main();
                }
                return;
            }
        }
        if mochi_core::ai::locator::Locator::parse(url).is_some() {
            self.ai_locate(url);
            return;
        }
        if url.is_empty() {
            return;
        }
        if let Some(r) = mochi_url::parse_mochi_resource_url(url) {
            let root = self
                .shell
                .workspace()
                .map(|ws| ws.root.to_string_lossy().into_owned());
            let Some(abs) = mochi_url::resolve_workspace_path(&r.path, root.as_deref()) else {
                return;
            };
            let path = PathBuf::from(abs);
            if matches!(
                r.kind,
                ResourceKind::Task | ResourceKind::Event | ResourceKind::Project
            ) {
                if let Some(id) = r.item_id.as_deref() {
                    self.open_base_schedule_reference(r.kind, id, &path);
                }
                return;
            }
            if !path.exists() {
                self.state.status_text =
                    format!("无法打开链接：找不到 {}，可能已被移动或删除", r.path);
                return;
            }
            if r.kind == mochi_url::ResourceKind::Directory {
                self.shell.expand(&path);
            } else {
                if !self.open_link_file_from_ui(&path) {
                    return;
                }
                self.state.view = WorkspaceView::Editor;
                if let Some(table_id) = r.table_id {
                    self.reveal_base(mochi_core::base::BaseLocation {
                        table_id,
                        record_id: r.record_id,
                        field_id: r.field_id,
                    });
                }
                self.invalidate_main();
            }
            self.sync_state();
            return;
        }
        let lower = url.to_ascii_lowercase();
        if ["http:", "https:", "mailto:", "tel:"]
            .iter()
            .any(|s| lower.starts_with(s))
        {
            if !platform::open_external(url) {
                self.state.status_text = format!("无法打开外部链接：{url}");
            }
            return;
        }
        // 维基链接 / 相对路径：先按当前文件所在目录解析，再按文件名在文件树里找
        let current_dir = self
            .shell
            .active()
            .and_then(|t| t.path())
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let mut candidates = Vec::new();
        if let Some(dir) = &current_dir {
            for name in [url.to_owned(), format!("{url}.md"), format!("{url}.mc")] {
                candidates.push(dir.join(name));
            }
        }
        if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
            if !self.open_link_file_from_ui(&found) {
                return;
            }
            self.invalidate_main();
            self.sync_state();
            return;
        }
        let stem = url.rsplit('/').next().unwrap_or(url);
        let by_name = self
            .shell
            .rows()
            .iter()
            .find(|r| {
                !r.is_dir
                    && r.path
                        .file_stem()
                        .map(|s| s.to_string_lossy() == stem)
                        .unwrap_or(false)
            })
            .map(|r| r.path.clone());
        match by_name {
            Some(p) => {
                if !self.open_link_file_from_ui(&p) {
                    return;
                }
                self.invalidate_main();
                self.sync_state();
            }
            None => self.state.status_text = format!("找不到链接目标：{url}"),
        }
    }
}
