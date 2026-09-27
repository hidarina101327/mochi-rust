//! 管理桌面卡片管理窗口的状态、布局和输入。
use super::*;

/// 卡片管理器持有模态输入时，原生弹出菜单也可以正常使用。
fn transfer_menu(owner: HWND, importing: bool) -> u32 {
    use windows::core::w;
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::*;
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return 0;
        };
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            1,
            if importing {
                w!("从文件导入…")
            } else {
                w!("导出到文件…")
            },
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            2,
            if importing {
                w!("粘贴 JSON 导入")
            } else {
                w!("复制 JSON")
            },
        );
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let selected = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            Some(0),
            owner,
            None,
        )
        .0 as u32;
        let _ = DestroyMenu(menu);
        selected
    }
}

impl App {
    pub fn desktop_icons_ready(&mut self) {
        self.desktop_take_target_results();
        self.invalidate_main();
    }
    pub(in crate::app) fn desktop_manager_active(&self) -> bool {
        self.desktop
            .panel
            .as_ref()
            .is_some_and(|p| !p.embedded || self.state.view == WorkspaceView::DesktopCards)
    }
    pub(in crate::app) fn desktop_manager_modal(&self) -> bool {
        self.desktop_manager_active()
            && self
                .desktop
                .panel
                .as_ref()
                .is_some_and(|p| !p.embedded || p.confirm_cancel || p.confirm_delete.is_some())
    }
    pub(in crate::app) fn desktop_manager_area(&self) -> Rect {
        if self.desktop.panel.as_ref().is_some_and(|p| p.embedded) {
            let chrome = Chrome::build(&self.state, self.renderer.viewport());
            chrome.tree.rect(chrome.editor)
        } else {
            self.renderer.viewport()
        }
    }
    pub(in crate::app) fn desktop_manager_input_active(&self) -> bool {
        self.desktop_manager_active()
            && self.object_picker.is_none()
            && !self.notification_open()
            && self.menu.is_none()
            && self.dialog.is_none()
            && self.settings_overlay.is_none()
            && self.search.is_none()
            && self.command.is_none()
            && self.global_import.is_none()
            && self.template_picker.is_none()
            && self.image_preview.is_none()
            && self.export_form.is_none()
            && self.commands.review.is_none()
            && self.mapped_folder.is_none()
            && self.link_create.is_none()
            && self.automation.panel.is_none()
            && self.table_picker.is_none()
            && matches!(self.focus, Focus::Main | Focus::Dialog)
    }
    pub(in crate::app) fn desktop_manager_captures_pointer(&self, x: f32, y: f32) -> bool {
        self.desktop_manager_active()
            && self.object_picker.is_none()
            && !self.notification_open()
            && self.menu.is_none()
            && self.dialog.is_none()
            && self.settings_overlay.is_none()
            && self.search.is_none()
            && self.command.is_none()
            && self.template_picker.is_none()
            && self.global_import.is_none()
            && self.image_preview.is_none()
            && self.export_form.is_none()
            && self.commands.review.is_none()
            && self.mapped_folder.is_none()
            && self.link_create.is_none()
            && self.automation.panel.is_none()
            && self.table_picker.is_none()
            && self.desktop.panel.as_ref().is_some_and(|p| {
                !p.embedded
                    || p.confirm_cancel
                    || p.confirm_delete.is_some()
                    || p.dragging()
                    || self.desktop_manager_area().contains(x, y)
            })
    }

    pub(in crate::app) fn open_desktop_manager(&mut self) {
        if !self.commit_title() || !self.commit_table_cell() {
            return;
        }
        self.close_notifications();
        self.close_search();
        self.command = None;
        self.menu = None;
        if self.desktop.panel.is_some() {
            self.open_auxiliary_page(WorkspaceView::DesktopCards);
            self.focus = Focus::Main;
            self.invalidate_main();
            return;
        }
        let mut panel = ui::State::workspace(&self.desktop.config);
        panel.error = if self.desktop.root.is_none() {
            Some("请先打开工作区，再创建桌面卡片。卡片随工作区保存。".into())
        } else if !self.desktop.loaded {
            Some(
                self.desktop
                    .error
                    .clone()
                    .unwrap_or_else(|| "正在读取工作区卡片布局…".into()),
            )
        } else {
            self.desktop.error.clone()
        };
        self.desktop.editing_base = Some(self.desktop.config.clone());
        self.desktop.panel = Some(panel);
        self.desktop_timer_state();
        self.open_auxiliary_page(WorkspaceView::DesktopCards);
        self.focus = Focus::Main;
        self.invalidate_main();
        if !self.desktop.loaded && self.desktop.job.is_none() {
            if let Some(root) = self.desktop.root.clone() {
                self.desktop_enqueue(runtime::Job::Load(root));
            }
        }
    }

    pub(in crate::app) fn desktop_manager_click(&mut self, x: f32, y: f32) {
        self.focus = Focus::Main;
        if self.desktop.saving_editor {
            return;
        }
        if !self.desktop.loaded && self.desktop.job.is_some() {
            return;
        }
        let viewport = self.desktop_manager_area();
        let action = self.desktop.panel.as_mut().and_then(|p| {
            let l = p.layout(viewport);
            if p.pointer_down(&l, x, y) {
                None
            } else {
                p.click(&l, x, y)
            }
        });
        self.desktop_manager_action(action);
    }
    pub(in crate::app) fn desktop_manager_key(
        &mut self,
        key: u16,
        shift: bool,
        ctrl: bool,
        alt: bool,
    ) -> bool {
        if self.desktop.saving_editor {
            return true;
        }
        if self.desktop.panel.as_ref().is_some_and(|p| p.embedded)
            && (alt
                || (ctrl && !matches!(key, 0x41 | 0x43 | 0x58 | 0x56 | 0x53 | 0x5a | 0x59 | 0x44)))
        {
            return false;
        }
        if alt && key == 0x73 {
            let action = self
                .desktop
                .panel
                .as_mut()
                .and_then(|p| p.key(0x1b, false, false));
            self.desktop_manager_action(action);
            return true;
        }
        if ctrl && matches!(key, 0x43 | 0x58 | 0x56) {
            if let Some(panel) = self.desktop.panel.as_mut() {
                let max_input = if matches!(panel.focus_field, Some(ui::Field::Studio(_))) {
                    4096
                } else if matches!(panel.focus_field, Some(ui::Field::FolderPath)) {
                    model::MAX_SOURCE_CHARS
                } else if matches!(panel.focus_field, Some(ui::Field::GroupRule(_))) {
                    1024
                } else {
                    model::MAX_TITLE_CHARS
                };
                if let Some(field) = panel.focused_field_mut() {
                    match key {
                        0x43 | 0x58 => {
                            if platform::copy_to_clipboard(field.buffer.selected_text())
                                && key == 0x58
                            {
                                field.buffer.insert("");
                            }
                        }
                        0x56 => {
                            if let Some(value) = platform::read_clipboard_text() {
                                let value: String = value
                                    .chars()
                                    .filter(|c| !c.is_control())
                                    .take(max_input)
                                    .collect();
                                field.buffer.insert(&value);
                            }
                        }
                        _ => {}
                    }
                }
                panel.commit_focused_field();
            }
            return true;
        }
        let action = self
            .desktop
            .panel
            .as_mut()
            .and_then(|p| p.key(key, shift, ctrl));
        self.desktop_manager_action(action);
        true
    }
    pub(in crate::app) fn desktop_manager_pointer(&mut self, x: f32, y: f32) -> bool {
        let viewport = self.desktop_manager_area();
        self.desktop.panel.as_mut().is_some_and(|p| {
            let l = p.layout(viewport);
            p.pointer(&l, x, y)
        })
    }
    pub(in crate::app) fn desktop_manager_wheel(&mut self, x: f32, y: f32, delta: i16) {
        let viewport = self.desktop_manager_area();
        if let Some(p) = self.desktop.panel.as_mut() {
            let l = p.layout(viewport);
            p.wheel(&l, x, y, delta as f32 / 120.0 * 60.0);
        }
    }
    pub(in crate::app) fn paint_desktop_manager(&mut self, palette: &Palette) {
        if !self.desktop_manager_active() {
            return;
        }
        let viewport = self.desktop_manager_area();
        desktop_window::shortcut_icon::watch(HWND(self.hwnd_raw as *mut _));
        if let Some(panel) = self.desktop.panel.as_mut() {
            panel.icon_workspace = self.desktop.root.clone().unwrap_or_default();
            let layout = panel.layout(viewport);
            self.list.clear_carets();
            panel.paint(&mut self.list, &layout, viewport, palette);
        }
    }

    fn desktop_manager_action(&mut self, action: Option<ui::Action>) {
        let Some(action) = action else {
            return;
        };
        if matches!(action, ui::Action::Cancel) {
            if self.desktop.panel.as_ref().is_some_and(|p| p.embedded) {
                self.desktop.panel = Some(ui::State::workspace(&self.desktop.config));
                self.desktop.editing_base = Some(self.desktop.config.clone());
            } else {
                self.desktop.panel = None;
            }
            self.focus = Focus::Main;
            return;
        }
        if !self.desktop.loaded
            && !(matches!(action, ui::Action::Import)
                && self.desktop.root.is_some()
                && self.desktop.job.is_none())
        {
            self.desktop_error(
                "布局尚未就绪，请打开工作区或等待读取完成；损坏的布局可通过导入恢复".into(),
            );
            return;
        }
        let owner = HWND(self.hwnd_raw as *mut _);
        let keep_open = matches!(action, ui::Action::SaveKeepOpen(_))
            || self.desktop.panel.as_ref().is_some_and(|p| p.embedded);
        match action {
            ui::Action::PickItemColor { background, item } => {
                if let Some(panel) = self.desktop.panel.as_mut() {
                    if let Some(page) = panel.selected_page_mut() {
                        let current = if let Some(key) = &item {
                            page.item_styles.get(key).and_then(|s| {
                                if background {
                                    s.background
                                } else {
                                    s.foreground
                                }
                            })
                        } else if background {
                            page.presentation.item_background
                        } else {
                            page.presentation.item_foreground
                        };
                        let swap =
                            |rgb: u32| (rgb & 0xff00) | ((rgb >> 16) & 0xff) | ((rgb & 0xff) << 16);
                        use windows::Win32::UI::Controls::Dialogs::*;
                        let mut colors = [windows::Win32::Foundation::COLORREF(0xffffff); 16];
                        let mut picker = CHOOSECOLORW {
                            lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
                            hwndOwner: owner,
                            rgbResult: windows::Win32::Foundation::COLORREF(swap(
                                current.unwrap_or(if background { 0xffffff } else { 0x20242b }),
                            )),
                            lpCustColors: colors.as_mut_ptr(),
                            Flags: CC_FULLOPEN | CC_RGBINIT,
                            ..Default::default()
                        };
                        if unsafe { ChooseColorW(&mut picker).as_bool() } {
                            let color = Some(swap(picker.rgbResult.0));
                            if let Some(key) = item {
                                let style = page.item_styles.entry(key).or_default();
                                if background {
                                    style.background = color
                                } else {
                                    style.foreground = color
                                }
                            } else if background {
                                page.presentation.item_background = color
                            } else {
                                page.presentation.item_foreground = color
                            }
                            panel.dirty = true;
                        }
                    }
                }
            }

            ui::Action::PickFontColor
            | ui::Action::PickBackgroundColor
            | ui::Action::PickStudioColor(_) => {
                let component = matches!(action, ui::Action::PickStudioColor(_));
                let background = matches!(
                    action,
                    ui::Action::PickBackgroundColor | ui::Action::PickStudioColor(true)
                );
                use windows::Win32::UI::Controls::Dialogs::*;
                if let Some(panel) = self.desktop.panel.as_mut() {
                    let current = if component {
                        panel.studio_color(background)
                    } else {
                        panel.selected_card_ref().and_then(|c| {
                            if background {
                                c.appearance.background_color
                            } else {
                                c.appearance.font_color
                            }
                        })
                    }
                    .unwrap_or(0x20242b);
                    let swap =
                        |rgb: u32| (rgb & 0xff00) | ((rgb >> 16) & 0xff) | ((rgb & 0xff) << 16);
                    let mut colors = [windows::Win32::Foundation::COLORREF(0xffffff); 16];
                    let mut picker = CHOOSECOLORW {
                        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
                        hwndOwner: owner,
                        rgbResult: windows::Win32::Foundation::COLORREF(swap(current)),
                        lpCustColors: colors.as_mut_ptr(),
                        Flags: CC_FULLOPEN | CC_RGBINIT,
                        ..Default::default()
                    };
                    if unsafe { ChooseColorW(&mut picker).as_bool() } {
                        if component {
                            panel.set_studio_color(background, swap(picker.rgbResult.0));
                        } else {
                            if let Some(card) = panel.selected_card_mut() {
                                if background {
                                    card.appearance.background_color =
                                        Some(swap(picker.rgbResult.0));
                                } else {
                                    card.appearance.font_color = Some(swap(picker.rgbResult.0));
                                }
                                panel.dirty = true;
                            }
                        }
                    }
                }
            }
            ui::Action::Save(mut config) | ui::Action::SaveKeepOpen(mut config) => {
                self.desktop.keep_editor_open = keep_open;
                // 草稿打开期间，用户仍可能拖动或隐藏桌面卡片。
                // 除非编辑器明确修改过，否则保留卡片当前的字段值。
                if let Some(base) = &self.desktop.editing_base {
                    for card in &mut config.cards {
                        if let (Some(before), Some(live)) = (
                            base.cards.iter().find(|c| c.id == card.id),
                            self.desktop.config.cards.iter().find(|c| c.id == card.id),
                        ) {
                            macro_rules! merge {($($field:ident),*)=>{$(if card.$field==before.$field{card.$field=live.$field.clone();})*};}
                            merge!(x, y, width, height, enabled, locked);
                            if card.appearance.edge_dock == before.appearance.edge_dock {
                                card.appearance.edge_dock = live.appearance.edge_dock;
                            }
                            if card.appearance.pinned == before.appearance.pinned {
                                card.appearance.pinned = live.appearance.pinned;
                            }
                            for page in &mut card.pages {
                                if let (Some(previous), Some(current)) = (
                                    before.pages.iter().find(|p| p.id == page.id),
                                    live.pages.iter().find(|p| p.id == page.id),
                                ) {
                                    if page.item_styles == previous.item_styles {
                                        page.item_styles = current.item_styles.clone();
                                    }
                                    if page.studio == previous.studio {
                                        page.studio = current.studio.clone();
                                    }
                                    if page.presentation.calendar_expanded
                                        == previous.presentation.calendar_expanded
                                    {
                                        page.presentation.calendar_expanded =
                                            current.presentation.calendar_expanded;
                                    }
                                }
                            }

                            if card.active_page == before.active_page
                                && card.pages.iter().any(|p| p.id == live.active_page)
                            {
                                card.active_page = live.active_page.clone();
                            }
                        }
                    }
                }
                if let Err(e) = config.validate() {
                    self.desktop_error(e.to_string());
                    return;
                }
                if self.desktop.job.is_some() || !self.desktop.pending.is_empty() {
                    self.desktop_error("卡片正在更新，请稍后点击应用".into());
                    return;
                }
                // 将新建的卡片依次错开放置，避免一批卡片全叠在同一位置。
                for (i, c) in config.cards.iter_mut().enumerate() {
                    if !self.desktop.config.cards.iter().any(|old| old.id == c.id)
                        && ((c.x == 0 && c.y == 0) || (c.x == 24 && c.y == 24))
                    {
                        c.x = 32 + (i as i32 % 5) * 32;
                        c.y = 64 + (i as i32 % 5) * 32;
                    }
                }
                if let Some(root) = self.desktop.root.clone() {
                    self.desktop.saving_editor = true;
                    self.desktop.save_at = None;
                    if let Some(p) = self.desktop.panel.as_mut() {
                        p.error = Some("正在保存并应用…".into());
                    }
                    let job = if self.desktop.recovery {
                        runtime::Job::Recover(root, config, self.desktop.revision + 1)
                    } else {
                        runtime::Job::Save(root, config, self.desktop.revision + 1, true)
                    };
                    self.desktop_enqueue(job);
                }
            }
            ui::Action::Import => {
                if self.desktop.job.is_some() {
                    self.desktop_error("后台操作进行中，请稍后导入".into());
                    return;
                }
                let job = match transfer_menu(owner, true) {
                    1 => platform::pick_file(owner).map(|path| (Some(path), String::new())),
                    2 => match platform::read_clipboard_text() {
                        Some(text) if !text.trim().is_empty() => Some((None, text)),
                        _ => {
                            self.desktop_error("剪贴板中没有文本，请先复制桌面卡片 JSON".into());
                            return;
                        }
                    },
                    _ => None,
                };
                if let Some((path, text)) = job {
                    if let Some(p) = self.desktop.panel.as_mut() {
                        p.commit_focused_field();
                        let draft = p.config.clone();
                        self.desktop.saving_editor = true;
                        p.error = Some("正在导入布局…".into());
                        self.desktop_enqueue(match path {
                            Some(path) => runtime::Job::Import(path, draft),
                            None => runtime::Job::ImportJson(text, draft),
                        });
                    }
                }
            }
            ui::Action::ExportAll | ui::Action::ExportCard(_) => {
                let Some(p) = self.desktop.panel.as_mut() else {
                    return;
                };
                p.commit_focused_field();
                let mut config = p.config.clone();
                if let ui::Action::ExportCard(id) = action {
                    config.cards.retain(|c| c.id == id);
                }
                match transfer_menu(owner, false) {
                    1 => {
                        if let Some(path) =
                            platform::save_file(owner, "墨池桌面卡片.mochi-cards.zip")
                        {
                            self.desktop_enqueue(runtime::Job::Export(path, config));
                        }
                    }
                    2 => match config.export_json() {
                        Ok(text) if platform::copy_to_clipboard(&text) => {
                            self.show_global_notice("桌面卡片 JSON 已复制");
                        }
                        Ok(_) => self.desktop_error("复制失败，请稍后重试".into()),
                        Err(error) => self.desktop_error(format!("导出失败：{error}")),
                    },
                    _ => {}
                }
            }
            ui::Action::ChooseFolder { card_id, page_id } => {
                if let Some(path) = platform::pick_folder(owner) {
                    if let Some(panel) = self.desktop.panel.as_mut() {
                        panel.set_folder_source(
                            &card_id,
                            &page_id,
                            path.to_string_lossy().into_owned(),
                        );
                    }
                }
            }
            ui::Action::ChooseSource { card_id, page_id } => {
                let module = self
                    .desktop
                    .panel
                    .as_ref()
                    .and_then(|p| p.config.cards.iter().find(|c| c.id == card_id))
                    .and_then(|c| c.pages.iter().find(|p| p.id == page_id))
                    .map_or(Module::Document, |p| p.module);
                self.open_object_picker(
                    super::super::object_picker_host::Purpose::DesktopSources {
                        card: card_id,
                        page: page_id,
                        module,
                    },
                );
            }

            ui::Action::LocateCard(id) => {
                if self
                    .desktop
                    .config
                    .cards
                    .iter()
                    .any(|c| c.id == id && c.enabled)
                    && !self.desktop.paused
                {
                    self.desktop.windows.locate(&id);
                } else {
                    self.desktop_error("请先应用并显示此卡片，再定位到桌面".into());
                }
            }
            ui::Action::OpenModule { module, .. } => {
                self.desktop_open_module(module);
                self.desktop.panel = None;
            }
            ui::Action::Cancel => {}
        }
    }
}
