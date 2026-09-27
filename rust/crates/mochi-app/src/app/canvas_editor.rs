//! 处理画布编辑、拖动、文本变更、撤销和保存。
use super::*;

mod agent;

#[cfg(test)]
mod tests;

impl App {
    pub(super) fn canvas_input_active(&self) -> bool {
        self.state.view == WorkspaceView::Editor
            && matches!(self.focus, Focus::Main | Focus::CanvasText)
            && self.dialog.is_none()
            && self.menu.is_none()
            && self.object_picker.is_none()
            && self.settings_overlay.is_none()
            && self.search.is_none()
            && self.command.is_none()
            && self.export_form.is_none()
            && self.commands.review.is_none()
            && self.image_preview.is_none()
            && self.template_picker.is_none()
            && self.automation.panel.is_none()
            && !self.notification_open()
            && self.mapped_folder.is_none()
            && self.link_create.is_none()
            && !self.desktop_manager_active()
            && matches!(self.viewer_tab(), Some((_, viewer::Content::Canvas(_))))
    }

    fn save_active_canvas(&mut self) {
        if let Some(path) = self.viewer_tab().and_then(|(path, content)| {
            matches!(content, viewer::Content::Canvas(_)).then(|| path.to_path_buf())
        }) {
            self.save_canvas(&path);
        }
    }

    pub(super) fn canvas_text_changed(&mut self) {
        if let Some(viewer::Content::Canvas(state)) = self.viewer_content_mut() {
            state.sync_text();
        }
        self.save_active_canvas();
    }

    pub(super) fn canvas_finish_editing(&mut self) {
        if let Some(viewer::Content::Canvas(state)) = self.viewer_content_mut() {
            state.finish_editing();
        }
        self.save_active_canvas();
        if self.focus == Focus::CanvasText {
            self.focus = Focus::Main;
        }
    }

    pub(super) fn canvas_release(&mut self) {
        if let Some(viewer::Content::Canvas(state)) = self.viewer_content_mut() {
            state.release();
        }
        self.save_active_canvas();
    }

    pub fn cancel_canvas_drag(&mut self) -> bool {
        if !self
            .drag
            .is_some_and(|d| matches!(d.target, DragTarget::CanvasCard | DragTarget::CanvasPan))
        {
            return false;
        }
        // 如果 Windows 在笔迹手势过程中收回鼠标捕获，仍要保留已完成的部分。
        self.canvas_release();
        self.drag = None;
        true
    }

    pub(super) fn canvas_click(&mut self, hit: canvas_view::Hit, x: f32, y: f32) {
        use canvas_view::{Element, Hit};
        if hit == Hit::AddReference {
            self.canvas_finish_editing();
            if let Some((path, _)) = self.viewer_tab() {
                self.open_object_picker(object_picker_host::Purpose::Canvas(path.to_path_buf()));
            }
            return;
        }
        let body = self.viewer_layout.body;
        let pan = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x20) < 0 };
        let Some(viewer::Content::Canvas(state)) = self.viewer_content_mut() else {
            return;
        };
        match hit {
            Hit::Surface => state.pointer_down(body, x, y, pan),
            Hit::Tool(tool) => state.set_tool(tool),
            Hit::Color(_) | Hit::Width(_) | Hit::FontSize(_) => state.apply_style(hit),
            Hit::Undo => {
                state.undo_redo(false);
            }
            Hit::Redo => {
                state.undo_redo(true);
            }
            Hit::Delete => state.delete_selected(),
            Hit::EditText => {
                if let Some(Element::Text(i)) = state.selected {
                    state.edit_text(i);
                }
            }
            Hit::Resize => state.begin_resize(),
            Hit::Grid => state.grid = !state.grid,
            Hit::ZoomOut | Hit::ZoomIn => {
                state.zoom_at(
                    body,
                    (body.left + body.right) / 2.0,
                    (body.top + body.bottom) / 2.0,
                    if hit == Hit::ZoomIn { 1.2 } else { 1.0 / 1.2 },
                );
            }
            Hit::ResetZoom => {
                state.reset_view();
            }
            Hit::FitContent => {
                state.fit_to_content(body);
            }
            Hit::AddReference => {}
        }
        self.focus = if state.editor.is_some() {
            Focus::CanvasText
        } else {
            Focus::Main
        };
        if matches!(hit, Hit::Surface | Hit::Resize) {
            self.drag = Some(Drag {
                target: DragTarget::CanvasCard,
                grab_offset: 0.0,
            });
        } else {
            self.save_active_canvas();
        }
        self.invalidate_main();
    }

    pub(super) fn canvas_key(&mut self, key: u16, shift: bool, ctrl: bool) -> bool {
        use canvas_view::{Element, Tool};
        if !self.canvas_input_active() {
            return false;
        }
        if ctrl && matches!(key, 0x09 | 0x57) {
            self.canvas_finish_editing();
            return false;
        }
        if ctrl && key == 0x53 {
            self.canvas_text_changed();
            return true;
        }
        let body = self.viewer_layout.body;
        let Some(viewer::Content::Canvas(state)) = self.viewer_content_mut() else {
            return false;
        };
        if state.editor.is_some() {
            if !state.text_key(key, shift, ctrl) {
                return false;
            }
        } else if ctrl {
            match key {
                0x5a | 0x59 => {
                    state.undo_redo(key == 0x59 || shift);
                }
                0x30 => {
                    state.reset_view();
                }
                0xbb | 0x6b | 0xbd | 0x6d => {
                    state.zoom_at(
                        body,
                        (body.left + body.right) / 2.0,
                        (body.top + body.bottom) / 2.0,
                        if matches!(key, 0xbb | 0x6b) {
                            1.2
                        } else {
                            1.0 / 1.2
                        },
                    );
                }
                _ => return false,
            }
        } else {
            match key {
                0x56 => state.set_tool(Tool::Select),
                0x50 | 0x42 => state.set_tool(Tool::Pen),
                0x54 => state.set_tool(Tool::Text),
                0x45 => state.set_tool(Tool::Eraser),
                0x48 => state.set_tool(Tool::Hand),
                0x2e | 0x08 => state.delete_selected(),
                0x1b => {
                    state.cancel_gesture();
                    state.set_tool(Tool::Select);
                    self.drag = None;
                }
                0x31 if shift => {
                    state.fit_to_content(body);
                }
                0x0d => {
                    if let Some(Element::Text(i)) = state.selected {
                        state.edit_text(i);
                    } else {
                        return false;
                    }
                }
                0x20 => return true,
                _ => return false,
            }
        }
        self.focus = if matches!(self.viewer_tab(), Some((_, viewer::Content::Canvas(s))) if s.editor.is_some())
        {
            Focus::CanvasText
        } else {
            Focus::Main
        };
        self.save_active_canvas();
        self.invalidate_main();
        true
    }

    pub(super) fn canvas_double_click(&mut self, x: f32, y: f32) -> bool {
        use canvas_view::{Element, Tool};
        if !self.canvas_input_active() || !self.viewer_layout.body.contains(x, y) {
            return false;
        }
        let body = self.viewer_layout.body;
        let Some(viewer::Content::Canvas(state)) = self.viewer_content_mut() else {
            return false;
        };
        if !matches!(state.tool, Tool::Select | Tool::Text) {
            return true;
        }
        state.release();
        let point = state.world(body, x, y);
        let mut target = None;
        match state.element_at(point) {
            Some(Element::Text(i)) => {
                if let Some(edit) = state.editor.as_mut().filter(|e| e.index == i) {
                    edit.field.buffer.select_word();
                } else {
                    state.edit_text(i);
                }
            }
            Some(Element::Card(i)) => target = Some(state.document.cards[i].target.clone()),
            Some(Element::Stroke(_)) => {}
            None => state.create_text(point),
        }
        self.focus = if state.editor.is_some() {
            Focus::CanvasText
        } else {
            Focus::Main
        };
        self.drag = None;
        self.save_active_canvas();
        match target {
            Some(mochi_core::canvas::Target::Mochi { url }) => self.open_link(&url),
            Some(mochi_core::canvas::Target::PdfAnnotation {
                path,
                annotation_id,
            }) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    let path = root.join(path);
                    if path.is_file() {
                        self.open_file_from_ui(&path);
                        self.pdf_locate_annotation(&annotation_id);
                    }
                }
            }
            None => {}
        }
        self.invalidate_main();
        true
    }

    pub(super) fn canvas_scroll(&mut self, notches: f32, horizontal: bool) {
        if let Some(viewer::Content::Canvas(state)) = self.viewer_content_mut() {
            let delta = f64::from(notches) * 90.0 / state.document.viewport.zoom;
            if horizontal {
                state.document.viewport.x -= delta;
            } else {
                state.document.viewport.y -= delta;
            }
        }
        self.save_active_canvas();
        self.invalidate_main();
    }

    /// 使用 FileService 的原子安全写入器保存精简后的 Canvas JSON。
    pub(super) fn save_canvas(&mut self, path: &Path) -> bool {
        let content = match self.viewer_content_for(path) {
            Some(viewer::Content::Canvas(state)) => state.serialized(),
            _ => return false,
        };
        match content.and_then(|content| {
            mochi_core::files::FileService::new().write_file_safe(path, &content)
        }) {
            Ok(()) => true,
            Err(error) => {
                self.show_global_notice(format!("保存画布失败：{error}"));
                false
            }
        }
    }

    pub(super) fn canvas_drag_to(&mut self, x: f32, y: f32) -> bool {
        let body = self.viewer_layout.body;
        let Some(viewer::Content::Canvas(state)) = self.viewer_content_mut() else {
            return false;
        };
        state.pointer_move(body, x, y)
    }

    pub(super) fn canvas_pan_to(&mut self, x: f32, y: f32) -> bool {
        self.canvas_drag_to(x, y)
    }

    pub(super) fn canvas_zoom_at(&mut self, x: f32, y: f32, factor: f64) {
        let body = self.viewer_layout.body;
        let path = self.viewer_tab().and_then(|(path, content)| {
            matches!(content, viewer::Content::Canvas(_)).then_some(path.to_path_buf())
        });
        let changed = match self.viewer_content_mut() {
            Some(viewer::Content::Canvas(state)) => state.zoom_at(body, x, y, factor),
            _ => false,
        };
        if changed {
            if let Some(path) = path {
                self.save_canvas(&path);
            }
            self.invalidate_main();
        }
    }

    pub(super) fn canvas_reset_view(&mut self) {
        let path = self.viewer_tab().and_then(|(path, content)| {
            matches!(content, viewer::Content::Canvas(_)).then_some(path.to_path_buf())
        });
        let changed = match self.viewer_content_mut() {
            Some(viewer::Content::Canvas(state)) => state.reset_view(),
            _ => false,
        };
        if changed {
            if let Some(path) = path {
                self.save_canvas(&path);
            }
            self.invalidate_main();
        }
    }

    pub(super) fn canvas_fit_to_content(&mut self) {
        let body = self.viewer_layout.body;
        let path = self.viewer_tab().and_then(|(path, content)| {
            matches!(content, viewer::Content::Canvas(_)).then_some(path.to_path_buf())
        });
        let changed = match self.viewer_content_mut() {
            Some(viewer::Content::Canvas(state)) => state.fit_to_content(body),
            _ => false,
        };
        if changed {
            if let Some(path) = path {
                self.save_canvas(&path);
            }
            self.invalidate_main();
        }
    }
}
