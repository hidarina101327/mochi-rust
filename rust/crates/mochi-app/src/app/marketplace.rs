//! 管理社区资源市场的状态、加载过程和页面绘制。
use super::*;
use crate::ui::marketplace::{self as ui, Hit};

#[derive(Default)]
pub(super) struct State {
    pub view: ui::State,
    pub layout: ui::Layout,
}
pub enum Event {
    Catalog(std::result::Result<mochi_core::marketplace::Catalog, String>),
    Download(std::result::Result<PathBuf, String>),
}
impl App {
    pub(super) fn open_marketplace(&mut self) {
        if !self.commit_title() || !self.commit_table_cell() {
            return;
        }
        self.open_auxiliary_page(WorkspaceView::Marketplace);
        self.focus = Focus::Main;
        self.invalidate_main();
        if !self.marketplace.view.loaded && !self.marketplace.view.loading {
            self.refresh_marketplace();
        }
    }
    fn refresh_marketplace(&mut self) {
        if self.marketplace.view.loading || self.marketplace.view.downloading {
            return;
        }
        self.marketplace.view.loading = true;
        self.marketplace.view.error.clear();
        self.file_jobs.submit(PathBuf::new(), self.hwnd_raw, || {
            Ok(crate::file_runtime::Payload::Marketplace(Event::Catalog(
                mochi_core::marketplace::fetch_catalog().map_err(|e| e.to_string()),
            )))
        });
    }
    pub(super) fn finish_marketplace(&mut self, event: Event) {
        let state = &mut self.marketplace.view;
        match event {
            Event::Catalog(result) => {
                state.loading = false;
                match result {
                    Ok(catalog) => {
                        state.catalog = catalog;
                        state.loaded = true;
                        state.selected = None;
                        state.page = 0;
                        state.scroll = 0.0;
                        state.focused = None;
                        state.error.clear();
                        state.notice.clear();
                    }
                    Err(error) => state.error = error,
                }
            }
            Event::Download(result) => {
                state.downloading = false;
                match result {
                    Ok(path) => {
                        state.notice = format!("已下载：{}", path.display());
                        state.error.clear();
                        platform::show_item_in_folder(&path.to_string_lossy());
                    }
                    Err(error) => state.error = error,
                }
            }
        }
    }
    pub(super) fn paint_marketplace(&mut self, area: Rect, p: &Palette) {
        let lay = ui::layout(&self.marketplace.view, area);
        self.marketplace.view.scroll = self.marketplace.view.scroll.clamp(0.0, lay.max_scroll);
        ui::paint(&mut self.list, &self.marketplace.view, &lay, p);
        self.marketplace.layout = lay;
    }
    pub(super) fn marketplace_click(&mut self, x: f32, y: f32) {
        if let Some(hit) = self.marketplace.layout.hit(x, y) {
            self.marketplace_action(hit);
        }
    }
    fn marketplace_action(&mut self, hit: Hit) {
        match hit {
            Hit::Refresh => self.refresh_marketplace(),
            Hit::Releases => {
                platform::open_external(mochi_core::marketplace::RELEASES_URL);
            }
            Hit::Category(category) => {
                self.marketplace.view.category = category;
                self.marketplace.view.page = 0;
                self.marketplace.view.scroll = 0.0;
            }
            Hit::Previous | Hit::Next => {
                let state = &mut self.marketplace.view;
                state.page = if hit == Hit::Previous {
                    state.page.saturating_sub(1)
                } else {
                    (state.page + 1).min(state.pages() - 1)
                };
                state.scroll = 0.0;
            }
            Hit::Item(i) => {
                self.marketplace.view.selected = Some(i);
                self.marketplace.view.scroll = 0.0;
                self.marketplace.view.focused = None;
            }
            Hit::Back => {
                self.marketplace.view.selected = None;
                self.marketplace.view.scroll = 0.0;
                self.marketplace.view.focused = None;
            }
            Hit::Download => {
                if self.marketplace.view.downloading {
                    return;
                }
                let Some(package) = self
                    .marketplace
                    .view
                    .selected
                    .and_then(|i| self.marketplace.view.catalog.packages.get(i))
                    .cloned()
                else {
                    return;
                };
                let Some(path) = platform::save_file(HWND(self.hwnd_raw as *mut _), &package.asset)
                else {
                    return;
                };
                self.marketplace.view.downloading = true;
                self.marketplace.view.error.clear();
                self.marketplace.view.notice.clear();
                self.file_jobs.submit(path.clone(), self.hwnd_raw, move || {
                    let result = mochi_core::marketplace::download(&package, &path)
                        .map(|_| path)
                        .map_err(|e| e.to_string());
                    Ok(crate::file_runtime::Payload::Marketplace(Event::Download(
                        result,
                    )))
                });
            }
            Hit::CopyCommand => {
                if let Some(command) = self
                    .marketplace
                    .view
                    .selected
                    .and_then(|i| self.marketplace.view.catalog.packages.get(i))
                    .and_then(|p| p.install_command())
                {
                    // 用 PowerShell 安全的引号方式，命令无需改动 PATH 即可运行，
                    // 安装目录/工作目录含空格也没问题。
                    let command = std::env::current_exe()
                        .ok()
                        .and_then(|exe| exe.parent().map(|dir| dir.join("mochi-community.exe")))
                        .map(|exe| {
                            let mut result = format!(
                                "& '{}' {}",
                                exe.to_string_lossy().replace('\'', "''"),
                                command.trim_start_matches("mochi-community ")
                            );
                            if let Some(workspace) = self.shell.workspace() {
                                result.push_str(&format!(
                                    " --workspace '{}'",
                                    workspace.root.to_string_lossy().replace('\'', "''")
                                ));
                            }
                            result
                        })
                        .unwrap_or(command);
                    if platform::copy_to_clipboard(&command) {
                        self.marketplace.view.notice =
                            "安装命令已复制，请在 PowerShell 中运行".into();
                    } else {
                        self.marketplace.view.error = "无法复制，请稍后重试".into();
                    }
                }
            }
        }
        self.marketplace.view.hover = None;
    }
    pub(super) fn marketplace_key(&mut self, key: u16, shift: bool) -> bool {
        if self.state.view != WorkspaceView::Marketplace
            || self.dialog.is_some()
            || self.settings_overlay.is_some()
            || self.menu.is_some()
            || self.search.is_some()
            || self.command.is_some()
        {
            return false;
        }
        match key {
            0x1b if self.marketplace.view.selected.is_some() => self.marketplace_action(Hit::Back),
            0x09 => {
                let entries = &self.marketplace.layout.entries;
                if entries.is_empty() {
                    return true;
                }
                let current = entries
                    .iter()
                    .position(|(_, h)| Some(*h) == self.marketplace.view.focused);
                let next = current
                    .map(|i| (i + if shift { entries.len() - 1 } else { 1 }) % entries.len())
                    .unwrap_or(if shift { entries.len() - 1 } else { 0 });
                let (rect, hit) = entries[next];
                self.marketplace.view.focused = Some(hit);
                if matches!(hit, Hit::Item(_)) {
                    let body = self.marketplace.layout.body;
                    if rect.top < body.top {
                        self.marketplace.view.scroll += rect.top - body.top;
                    }
                    if rect.bottom > body.bottom {
                        self.marketplace.view.scroll += rect.bottom - body.bottom;
                    }
                }
            }
            0x0d | 0x20 => {
                if let Some(hit) = self.marketplace.view.focused {
                    self.marketplace_action(hit);
                }
            }
            _ => return false,
        }
        true
    }
}
