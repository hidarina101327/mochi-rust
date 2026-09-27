//! 根据指针位置选择光标样式，并处理标题栏操作。
use super::*;

impl App {
    pub fn take_caption_action(&mut self) -> Option<CaptionAction> {
        self.caption_action.take()
    }

    /// 指针在拖动手柄上时换成左右箭头。不换的话用户根本看不出那里能拖。
    pub fn cursor_for(&mut self, x: f32, y: f32) -> Option<PCWSTR> {
        if let Some(pending) = self.global_import.as_ref() {
            return Some(
                if matches!(
                    pending.view.hit(self.renderer.viewport(), x, y),
                    crate::ui::import_picker::Hit::Inside | crate::ui::import_picker::Hit::Outside
                ) {
                    windows::Win32::UI::WindowsAndMessaging::IDC_ARROW
                } else {
                    windows::Win32::UI::WindowsAndMessaging::IDC_HAND
                },
            );
        }
        let desktop_capture = self.desktop_manager_captures_pointer(x, y);
        let desktop_area = self.desktop_manager_area();
        if let Some(panel) = self.desktop.panel.as_ref().filter(|_| desktop_capture) {
            let layout = panel.layout(desktop_area);
            return Some(match layout.hit(x, y) {
                Some(
                    crate::ui::desktop_cards::Hit::CardName
                    | crate::ui::desktop_cards::Hit::PageName,
                ) => windows::Win32::UI::WindowsAndMessaging::IDC_IBEAM,
                Some(_) => windows::Win32::UI::WindowsAndMessaging::IDC_HAND,
                None => windows::Win32::UI::WindowsAndMessaging::IDC_ARROW,
            });
        }
        if let Some(cursor) = self.ai_float_cursor(x, y) {
            return Some(cursor);
        }
        if self.navigation_library_drag_active() {
            return Some(windows::Win32::UI::WindowsAndMessaging::IDC_SIZEALL);
        }
        if self.workflows_active() && self.workflows.view.area.contains(x, y) {
            if self.workflows.view.hit(x, y) == Some(crate::ui::workflows::Hit::Resize) {
                return Some(windows::Win32::UI::WindowsAndMessaging::IDC_SIZEWE);
            }
            return Some(if self.workflows.view.field_rect.contains(x, y) {
                windows::Win32::UI::WindowsAndMessaging::IDC_IBEAM
            } else if self.workflows.view.hit(x, y).is_some() {
                windows::Win32::UI::WindowsAndMessaging::IDC_HAND
            } else {
                windows::Win32::UI::WindowsAndMessaging::IDC_SIZEALL
            });
        }
        if self.notification_open() {
            return Some(if self.notification_layout().hit(x, y).is_some() {
                windows::Win32::UI::WindowsAndMessaging::IDC_HAND
            } else {
                windows::Win32::UI::WindowsAndMessaging::IDC_ARROW
            });
        }
        if self.hit_is_titlebar_button(x, y) {
            return Some(windows::Win32::UI::WindowsAndMessaging::IDC_HAND);
        }
        if self.automation.panel.is_some() && self.dialog.is_none() {
            return Some(windows::Win32::UI::WindowsAndMessaging::IDC_ARROW);
        }
        if let Some(picker) = self.template_picker.as_ref() {
            return Some(
                if matches!(
                    picker.hit(self.renderer.viewport(), x, y),
                    crate::ui::template_picker::Hit::Blank
                        | crate::ui::template_picker::Hit::Template(_)
                        | crate::ui::template_picker::Hit::Cancel
                ) {
                    windows::Win32::UI::WindowsAndMessaging::IDC_HAND
                } else {
                    windows::Win32::UI::WindowsAndMessaging::IDC_ARROW
                },
            );
        }
        if let Some(dialog) = self.object_picker.as_ref() {
            let layout = crate::ui::object_picker::layout(&dialog.state, self.renderer.viewport());
            return Some(if layout.query.contains(x, y) {
                windows::Win32::UI::WindowsAndMessaging::IDC_IBEAM
            } else {
                windows::Win32::UI::WindowsAndMessaging::IDC_ARROW
            });
        }
        if let Some(dialog) = self.dialog.as_ref() {
            use windows::Win32::UI::WindowsAndMessaging::{IDC_ARROW, IDC_HAND, IDC_IBEAM};
            if dialog
                .note_link_at(self.renderer.viewport(), x, y)
                .is_some()
            {
                return Some(IDC_HAND);
            }
            return Some(match dialog.hit(self.renderer.viewport(), x, y) {
                DialogHit::Field => IDC_IBEAM,
                DialogHit::Close | DialogHit::Button(_) => IDC_HAND,
                _ => IDC_ARROW,
            });
        }
        if self.image_preview.is_some()
            || self.export_form.is_some()
            || self.dialog.is_some()
            || self.menu.is_some()
            || self.search.is_some()
            || self.command.is_some()
            || self.commands.review.is_some()
        {
            return None;
        }
        if self.settings_overlay.is_some() {
            if self.prefs.nav_layout.hit(x, y) == Some(settings::NavHit::Scrollbar) {
                return Some(windows::Win32::UI::WindowsAndMessaging::IDC_HAND);
            }
            if self.prefs.content_layout.search_rect.contains(x, y) {
                return Some(windows::Win32::UI::WindowsAndMessaging::IDC_IBEAM);
            }
            let provider = self
                .settings_tab()
                .is_some_and(|(tab, section)| tab == "ai" && section != "parameters");
            let text_hit = if provider {
                if self
                    .prefs
                    .provider_layout
                    .hit(x, y)
                    .is_some_and(|h| !matches!(h, providers::Hit::Field(_)))
                {
                    return Some(windows::Win32::UI::WindowsAndMessaging::IDC_HAND);
                }
                (0..4).any(|i| {
                    self.prefs
                        .provider_layout
                        .field(i)
                        .is_some_and(|r| r.contains(x, y))
                })
            } else {
                self.prefs
                    .content_layout
                    .hit(x, y)
                    .and_then(|i| app_settings::descriptors().get(i))
                    .is_some_and(|d| {
                        matches!(
                            d.value_type,
                            SettingValueType::Number | SettingValueType::String
                        )
                    })
            };
            return if text_hit {
                Some(windows::Win32::UI::WindowsAndMessaging::IDC_IBEAM)
            } else {
                self.window_hover_rect(x, y)
                    .map(|_| windows::Win32::UI::WindowsAndMessaging::IDC_HAND)
            };
        }
        if self.viewer_layout.body.contains(x, y) && self.canvas_input_active() {
            if let Some((_, viewer::Content::Canvas(state))) = self.viewer_tab() {
                use windows::Win32::UI::WindowsAndMessaging::{
                    IDC_ARROW, IDC_CROSS, IDC_IBEAM, IDC_SIZEALL, IDC_SIZENWSE,
                };
                return Some(
                    if self.viewer_layout.hit(x, y)
                        == Some(viewer::Hit::Canvas(canvas_view::Hit::Resize))
                    {
                        IDC_SIZENWSE
                    } else if state.editor.is_some() || state.tool == canvas_view::Tool::Text {
                        IDC_IBEAM
                    } else {
                        match state.tool {
                            canvas_view::Tool::Pen | canvas_view::Tool::Eraser => IDC_CROSS,
                            canvas_view::Tool::Hand => IDC_SIZEALL,
                            _ => IDC_ARROW,
                        }
                    },
                );
            }
        }
        if self.base_detail_open()
            && self.dialog.is_none()
            && self.menu.is_none()
            && self.search.is_none()
        {
            return None;
        }
        if self.scrollbar_hit(x, y) {
            return Some(windows::Win32::UI::WindowsAndMessaging::IDC_ARROW);
        }
        let navigation_row = (self.state.view == WorkspaceView::MochiAi
            && self
                .ai
                .layout
                .navigation_rect
                .is_some_and(|area| area.contains(x, y)))
            || (self.content() == MainContent::Document && self.outline_area.contains(x, y));
        if navigation_row && self.window_hover_rect(x, y).is_some() {
            return Some(windows::Win32::UI::WindowsAndMessaging::IDC_HAND);
        }
        let tab_chrome = self.build_chrome();
        if !tab_chrome.tree.is_hidden(tab_chrome.tab_bar) {
            if let Some(direction) =
                tab_bar::navigation_hit(tab_chrome.tree.rect(tab_chrome.tab_bar), x, y)
            {
                let enabled = self.tab_navigation_enabled(match direction {
                    tab_bar::Navigation::Back => NavigationDirection::Back,
                    tab_bar::Navigation::Forward => NavigationDirection::Forward,
                });
                return Some(if enabled {
                    windows::Win32::UI::WindowsAndMessaging::IDC_HAND
                } else {
                    windows::Win32::UI::WindowsAndMessaging::IDC_ARROW
                });
            }
        }
        if self.content() == MainContent::Home && self.home.hit(self.editor_area, x, y).is_some() {
            return Some(windows::Win32::UI::WindowsAndMessaging::IDC_HAND);
        }
        let workspace_action = match self.state.view {
            WorkspaceView::Marketplace => self.marketplace.layout.hit(x, y).is_some(),
            WorkspaceView::Templates => self
                .views
                .templates_layout
                .hit(x, y)
                .is_some_and(|h| h != crate::ui::templates::Hit::Blank),
            WorkspaceView::Inbox => self.views.inbox_layout.hit(x, y).is_some_and(|h| {
                !matches!(h, views::inbox::Hit::Blank | views::inbox::Hit::EditField)
            }),
            WorkspaceView::AgentConfig => {
                self.agent
                    .layout
                    .hit(x, y)
                    .is_some_and(|h| h != agent_config::Hit::Blank)
                    || self.agent.nav_layout.hit(x, y).is_some()
                    || self.agent.update_banner.contains(x, y)
            }
            WorkspaceView::Schedule => self.agenda_hand_cursor(x, y),
            _ => false,
        };
        if workspace_action {
            return Some(windows::Win32::UI::WindowsAndMessaging::IDC_HAND);
        }
        if !self.base_detail_open()
            && self.dialog.is_none()
            && self.menu.is_none()
            && self.search.is_none()
        {
            if let Some((_, viewer::Content::Base(state))) = self.viewer_tab() {
                let hit = base_view::layout(state, self.editor_area).hit(x, y);
                if state.scrollbar_dragging()
                    || matches!(
                        hit,
                        Some(base_view::Hit::ScrollTrack | base_view::Hit::ScrollThumb)
                    )
                {
                    return Some(windows::Win32::UI::WindowsAndMessaging::IDC_ARROW);
                }
                if state.dragging() || matches!(hit, Some(base_view::Hit::Resize(_))) {
                    return Some(platform::CURSOR_RESIZE_HORIZONTAL);
                }
            }
        }
        if self.content() == MainContent::Document
            && self
                .doc
                .image_handle(self.editor_area, self.shell.active_scroll(), x, y)
                .is_some_and(|(_, r)| r.contains(x, y))
        {
            return Some(platform::CURSOR_RESIZE_HORIZONTAL);
        }
        if self.content() == MainContent::Document
            && self
                .doc
                .table_column_handles(self.editor_area, self.shell.active_scroll())
                .iter()
                .any(|(_, _, r, _)| r.contains(x, y))
        {
            return Some(platform::CURSOR_RESIZE_HORIZONTAL);
        }
        if self.settings_tab().is_some()
            && self.prefs.nav_layout.hit(x, y) == Some(settings::NavHit::Scrollbar)
        {
            return Some(windows::Win32::UI::WindowsAndMessaging::IDC_HAND);
        }
        if self
            .drag
            .is_some_and(|drag| drag.target == DragTarget::SettingsNav)
        {
            return Some(windows::Win32::UI::WindowsAndMessaging::IDC_HAND);
        }
        if self.drag.is_some_and(|drag| {
            matches!(
                drag.target,
                DragTarget::Navigation
                    | DragTarget::Sidebar
                    | DragTarget::AiPanel
                    | DragTarget::Split
                    | DragTarget::EditorImage
                    | DragTarget::EditorTable
                    | DragTarget::EditorTableColumn
            )
        }) {
            return Some(platform::CURSOR_RESIZE_HORIZONTAL);
        }
        if self.split.divider_hit.contains(x, y) {
            return Some(platform::CURSOR_RESIZE_HORIZONTAL);
        }
        // 原生编辑器直接接管文本输入，指针落在当前编辑器正文上时窗口要主动
        // 选用文本光标。`editor_area` 是扣掉工具栏/大纲后的矩形，点击处理
        // 用的也是它；边界保持一致，光标和命中测试才不会打架，包括点击仍能
        // 放置光标并进入编辑状态的空行。
        if matches!(self.content(), MainContent::Document | MainContent::Source)
            && self.editor_area.contains(x, y)
        {
            return Some(windows::Win32::UI::WindowsAndMessaging::IDC_IBEAM);
        }
        let chrome = self.build_chrome();
        match chrome.hit(x, y) {
            Some(NodeKey::TitleBarBack | NodeKey::TitleBarThemeToggle) => {
                Some(windows::Win32::UI::WindowsAndMessaging::IDC_HAND)
            }
            Some(NodeKey::NavigationResize)
            | Some(NodeKey::SidebarResize)
            | Some(NodeKey::RightSidebarResize) => Some(platform::CURSOR_RESIZE_HORIZONTAL),
            _ => None,
        }
    }
}
