//! 处理画布元素选择、AI 编辑、变更记录和撤销重做。
use super::super::{editor, widgets::FieldKey};
use super::*;

impl State {
    pub fn ai_edit_ready(&self) -> bool {
        self.editor.is_none() && self.gesture.is_none() && self.checkpoint.is_none()
    }

    /// 把一整个工具批次作为一个撤销步骤应用。App 会先持久化一份克隆。
    pub fn apply_ai_edit(
        &mut self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if !self.ai_edit_ready() {
            return Err("画布正在手动编辑，请结束笔迹或文字输入后重试".into());
        }
        let (mut next, mut result) =
            mochi_core::ai::tools::canvas_tools::edit(&self.document, name, args)?;
        for text in &mut next.texts {
            if !self
                .document
                .texts
                .iter()
                .any(|before| before.id == text.id)
            {
                update_height(text);
            }
        }
        next.validate().map_err(|e| e.to_string())?;
        result["revision"] = serde_json::json!(mochi_core::canvas::drawing::revision(&next));
        self.begin_change();
        self.document = next;
        self.selected = None;
        self.finish_change();
        Ok(result)
    }
    pub(super) fn begin_change(&mut self) {
        if self.checkpoint.is_none() {
            self.checkpoint = Some(self.document.clone());
        }
    }
    pub(super) fn finish_change(&mut self) {
        let Some(mut before) = self.checkpoint.take() else {
            return;
        };
        before.viewport = self.document.viewport.clone();
        if before != self.document {
            if self.undo.len() == 32 {
                self.undo.remove(0);
            }
            self.undo.push(before);
            self.redo.clear();
        }
    }
    pub fn undo_redo(&mut self, redo: bool) -> bool {
        self.finish_editing();
        self.release();
        let source = if redo { &mut self.redo } else { &mut self.undo };
        let Some(mut previous) = source.pop() else {
            return false;
        };
        previous.viewport = self.document.viewport.clone();
        let current = std::mem::replace(&mut self.document, previous);
        if redo {
            self.undo.push(current);
        } else {
            self.redo.push(current);
        }
        self.selected = None;
        true
    }
    pub fn element_at(&self, point: Point) -> Option<Element> {
        self.document
            .texts
            .iter()
            .rposition(|t| t.bounds().contains(point, 0.0))
            .map(Element::Text)
            .or_else(|| {
                self.document
                    .strokes
                    .iter()
                    .rposition(|s| s.hit(point, 6.0 / self.document.viewport.zoom))
                    .map(Element::Stroke)
            })
            .or_else(|| {
                self.document
                    .cards
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(i, _)| self.bounds(Element::Card(*i)).unwrap().contains(point, 0.0))
                    .map(|(i, _)| Element::Card(i))
            })
    }
    pub fn set_tool(&mut self, tool: Tool) {
        self.finish_editing();
        self.release();
        self.tool = tool;
        self.selected = None;
    }
    pub fn pointer_down(&mut self, body: Rect, x: f32, y: f32, pan: bool) {
        let point = self.world(body, x, y);
        if pan || self.tool == Tool::Hand {
            self.finish_editing();
            self.gesture = Some(Gesture::Pan {
                pointer: (x, y),
                viewport: (self.document.viewport.x, self.document.viewport.y),
            });
            return;
        }
        if let Some(index) = self.editor.as_ref().map(|e| e.index) {
            if self.document.texts[index].bounds().contains(point, 0.0) {
                self.text_click(body, x, y, false);
                self.gesture = Some(Gesture::TextSelect);
                return;
            }
        }
        self.finish_editing();
        self.selected = None;
        match self.tool {
            Tool::Pen => {
                self.begin_change();
                self.draft = Some(Stroke {
                    id: self.document.next_id("stroke"),
                    points: vec![point],
                    color: COLORS[self.color],
                    width: WIDTHS[self.width],
                });
                self.gesture = Some(Gesture::Draw);
            }
            Tool::Eraser => {
                self.begin_change();
                self.erase_at(point);
                self.gesture = Some(Gesture::Erase(point));
            }
            Tool::Text => {
                if let Some(Element::Text(index)) = self.element_at(point) {
                    self.edit_text(index);
                } else {
                    self.create_text(point);
                }
            }
            Tool::Select => {
                self.selected = self.element_at(point);
                self.sync_selected_style();
                if self.selected.is_some() {
                    self.begin_change();
                    self.gesture = Some(Gesture::Move(point));
                } else {
                    self.gesture = Some(Gesture::Pan {
                        pointer: (x, y),
                        viewport: (self.document.viewport.x, self.document.viewport.y),
                    });
                }
            }
            Tool::Hand => {}
        }
    }
    pub fn pointer_move(&mut self, body: Rect, x: f32, y: f32) -> bool {
        let Some(gesture) = self.gesture else {
            return false;
        };
        let point = self.world(body, x, y);
        match gesture {
            Gesture::Pan { pointer, viewport } => {
                self.document.viewport.x =
                    viewport.0 - f64::from(x - pointer.0) / self.document.viewport.zoom;
                self.document.viewport.y =
                    viewport.1 - f64::from(y - pointer.1) / self.document.viewport.zoom;
            }
            Gesture::Draw => {
                if !body.contains(x, y) {
                    return false;
                }
                if let Some(stroke) = &mut self.draft {
                    if stroke.points.len() < 100_000
                        && stroke.points.last().is_none_or(|last| {
                            (last.x - point.x).hypot(last.y - point.y) * self.document.viewport.zoom
                                >= 0.6
                        })
                    {
                        stroke.points.push(point);
                    }
                }
            }
            Gesture::Erase(previous) => {
                if !body.contains(x, y) {
                    return false;
                }
                let distance = (point.x - previous.x).hypot(point.y - previous.y);
                let steps = (distance * self.document.viewport.zoom / 4.0)
                    .ceil()
                    .clamp(1.0, 2000.0) as usize;
                for step in 1..=steps {
                    let t = step as f64 / steps as f64;
                    self.erase_at(Point {
                        x: previous.x + (point.x - previous.x) * t,
                        y: previous.y + (point.y - previous.y) * t,
                    });
                }
                self.gesture = Some(Gesture::Erase(point));
            }
            Gesture::Move(previous) => {
                let (dx, dy) = (point.x - previous.x, point.y - previous.y);
                match self.selected {
                    Some(Element::Card(i)) => {
                        let c = &mut self.document.cards[i];
                        c.x += dx;
                        c.y += dy;
                    }
                    Some(Element::Text(i)) => {
                        let t = &mut self.document.texts[i];
                        t.x += dx;
                        t.y += dy;
                    }
                    Some(Element::Stroke(i)) => {
                        for p in &mut self.document.strokes[i].points {
                            p.x += dx;
                            p.y += dy;
                        }
                    }
                    None => {}
                }
                self.gesture = Some(Gesture::Move(point));
            }
            Gesture::Resize => {
                if let Some(Element::Text(i)) = self.selected {
                    let text = &mut self.document.texts[i];
                    text.width = (point.x - text.x).clamp(80.0, 4000.0);
                    update_height(text);
                }
            }
            Gesture::TextSelect => self.text_click(body, x, y, true),
        }
        true
    }
    pub fn release(&mut self) {
        self.gesture = None;
        if let Some(stroke) = self.draft.take() {
            self.document.strokes.push(stroke);
        }
        if self.editor.is_none() {
            self.finish_change();
        }
    }
    pub fn cancel_gesture(&mut self) {
        self.draft = None;
        self.gesture = None;
        if let Some(mut before) = self.checkpoint.take() {
            before.viewport = self.document.viewport.clone();
            self.document = before;
        }
    }
    fn erase_at(&mut self, point: Point) {
        let tolerance = 8.0 / self.document.viewport.zoom;
        self.document.strokes.retain(|s| !s.hit(point, tolerance));
    }
    pub fn delete_selected(&mut self) {
        self.finish_editing();
        let Some(element) = self.selected.take() else {
            return;
        };
        self.begin_change();
        match element {
            Element::Card(i) => {
                self.document.cards.remove(i);
            }
            Element::Text(i) => {
                self.document.texts.remove(i);
            }
            Element::Stroke(i) => {
                self.document.strokes.remove(i);
            }
        }
        self.finish_change();
    }
    pub fn begin_resize(&mut self) {
        self.finish_editing();
        self.begin_change();
        self.gesture = Some(Gesture::Resize);
    }
    pub fn create_text(&mut self, point: Point) {
        self.finish_editing();
        self.begin_change();
        let index = self.document.texts.len();
        self.document.texts.push(TextNote {
            id: self.document.next_id("text"),
            x: point.x,
            y: point.y,
            width: 320.0,
            height: 64.0,
            text: String::new(),
            color: COLORS[self.color],
            font_size: FONT_SIZES[self.font_size],
        });
        self.editor = Some(TextEditor {
            index,
            field: TextField::new("写下想法…"),
        });
        self.selected = Some(Element::Text(index));
    }
    pub fn edit_text(&mut self, index: usize) {
        if self.editor.as_ref().is_some_and(|e| e.index == index) {
            return;
        }
        self.finish_editing();
        self.begin_change();
        let Some(text) = self.document.texts.get(index) else {
            return;
        };
        self.editor = Some(TextEditor {
            index,
            field: TextField::new("写下想法…").with_text(&text.text),
        });
        self.selected = Some(Element::Text(index));
        self.sync_selected_style();
    }
    pub fn sync_text(&mut self) {
        if let Some(editor) = &self.editor {
            if let Some(text) = self.document.texts.get_mut(editor.index) {
                text.text = editor.field.text().to_owned();
                update_height(text);
            }
        }
    }
    pub fn finish_editing(&mut self) {
        self.sync_text();
        if let Some(edit) = self.editor.take() {
            if self.document.texts[edit.index].text.trim().is_empty() {
                self.document.texts.remove(edit.index);
                self.selected = None;
            }
            self.finish_change();
        }
    }
    pub fn text_key(&mut self, key: u16, shift: bool, ctrl: bool) -> bool {
        if key == 0x1b || (ctrl && key == 0x0d) {
            self.finish_editing();
            return true;
        }
        let Some(e) = &mut self.editor else {
            return false;
        };
        let rect = logical_text_rect(&self.document.texts[e.index]);
        if ctrl && matches!(key, 0x5a | 0x59) {
            if key == 0x59 || shift {
                e.field.buffer.redo();
            } else {
                e.field.buffer.undo();
            }
        } else if key == 0x0d {
            e.field.buffer.insert("\n");
        } else if e.field.multiline_key(rect, key, shift, ctrl) == FieldKey::Ignored {
            return false;
        }
        self.sync_text();
        true
    }
    pub fn text_click(&mut self, body: Rect, x: f32, y: f32, extend: bool) {
        let point = self.world(body, x, y);
        let Some(e) = &mut self.editor else {
            return;
        };
        let text = &self.document.texts[e.index];
        let scale = text.font_size as f32 / TextStyle::Body.font_size();
        let (a, b) = e.field.buffer.selection();
        let anchor = if e.field.buffer.cursor() == a { b } else { a };
        e.field.multiline_click(
            logical_text_rect(text),
            ((point.x - text.x) as f32) / scale,
            ((point.y - text.y) as f32) / scale,
        );
        if extend {
            let next = e.field.buffer.cursor();
            e.field.buffer.set_cursor(anchor, false);
            e.field.buffer.set_cursor(next, true);
        }
    }
    pub fn apply_style(&mut self, hit: Hit) {
        let editing = self.editor.is_some();
        self.sync_text();
        self.begin_change();
        match hit {
            Hit::Color(i) => {
                self.color = i;
                match self.selected {
                    Some(Element::Text(index)) => self.document.texts[index].color = COLORS[i],
                    Some(Element::Stroke(index)) => self.document.strokes[index].color = COLORS[i],
                    _ => {}
                }
            }
            Hit::Width(i) => {
                self.width = i;
                if let Some(Element::Stroke(index)) = self.selected {
                    self.document.strokes[index].width = WIDTHS[i];
                }
            }
            Hit::FontSize(i) => {
                self.font_size = i;
                if let Some(Element::Text(index)) = self.selected {
                    let text = &mut self.document.texts[index];
                    text.font_size = FONT_SIZES[i];
                    update_height(text);
                }
            }
            _ => {}
        }
        if !editing {
            self.finish_change();
        }
    }

    fn sync_selected_style(&mut self) {
        match self.selected {
            Some(Element::Text(i)) => {
                let text = &self.document.texts[i];
                if let Some(index) = COLORS.iter().position(|c| *c == text.color) {
                    self.color = index;
                }
                if let Some(index) = FONT_SIZES.iter().position(|s| *s == text.font_size) {
                    self.font_size = index;
                }
            }
            Some(Element::Stroke(i)) => {
                let stroke = &self.document.strokes[i];
                if let Some(index) = COLORS.iter().position(|c| *c == stroke.color) {
                    self.color = index;
                }
                if let Some(index) = WIDTHS.iter().position(|w| *w == stroke.width) {
                    self.width = index;
                }
            }
            _ => {}
        }
    }
}

pub(super) fn logical_text_rect(text: &TextNote) -> Rect {
    let scale = text.font_size as f32 / TextStyle::Body.font_size();
    Rect::from_size(
        0.0,
        0.0,
        text.width as f32 / scale,
        text.height as f32 / scale,
    )
}
fn update_height(text: &mut TextNote) {
    let scale = text.font_size as f32 / TextStyle::Body.font_size();
    let layout = editor::layout(
        &text.text,
        TextStyle::Body,
        (text.width as f32 / scale - 24.0).max(40.0),
    );
    text.height = f64::from((layout.height + 24.0) * scale).clamp(48.0, 1_000_000.0);
}
