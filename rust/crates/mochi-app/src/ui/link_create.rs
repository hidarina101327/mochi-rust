//! 链接需要名称和 URL 两个字段，通用 Dialog 只支持单字段。

use std::path::PathBuf;

use crate::ui::draw::{Align, DrawList, TextStyle};
use crate::ui::icons::Icon;
use crate::ui::layout::Rect;
use crate::ui::theme::Palette;
use crate::ui::widgets::{FieldLook, TextField};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Url,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Close,
    Outside,
    Name,
    Url,
    Cancel,
    Create,
    Inside,
}

fn large_title_extra() -> f32 {
    (TextStyle::Large.line_height() - 28.0).max(0.0)
}

#[derive(Debug, Clone)]
pub struct State {
    pub parent: PathBuf,
    pub name: TextField,
    pub url: TextField,
    pub active: Field,
    pub error: String,
    pub hover: Option<Hit>,
}

impl State {
    pub fn new(parent: PathBuf) -> Self {
        Self {
            parent,
            name: TextField::new("例如：Mochi 官网"),
            url: TextField::new("https://"),
            active: Field::Name,
            error: String::new(),
            hover: None,
        }
    }

    pub fn rect(&self, viewport: Rect) -> Rect {
        let width = 440.0;
        let height = (if self.error.is_empty() { 284.0 } else { 304.0 }) + large_title_extra();
        Rect::from_size(
            ((viewport.left + viewport.right - width) / 2.0).round(),
            ((viewport.top + viewport.bottom - height) / 2.0).round(),
            width,
            height,
        )
    }

    fn parts(&self, viewport: Rect) -> (Rect, Rect, Rect, Rect, Rect, Rect) {
        let r = self.rect(viewport);
        let title_extra = large_title_extra();
        let name = Rect::new(
            r.left + 24.0,
            r.top + 96.0 + title_extra,
            r.right - 24.0,
            r.top + 134.0 + title_extra,
        );
        let url = Rect::new(
            r.left + 24.0,
            r.top + 174.0 + title_extra,
            r.right - 24.0,
            r.top + 212.0 + title_extra,
        );
        let bottom = r.bottom - 24.0;
        let create = Rect::new(r.right - 24.0 - 76.0, bottom - 36.0, r.right - 24.0, bottom);
        let cancel = Rect::new(create.left - 84.0, bottom - 36.0, create.left - 8.0, bottom);
        let close = Rect::from_size(r.right - 40.0, r.top + 16.0, 24.0, 24.0);
        (name, url, cancel, create, close, r)
    }

    pub fn hit(&self, viewport: Rect, x: f32, y: f32) -> Hit {
        let (name, url, cancel, create, close, r) = self.parts(viewport);
        if !r.contains(x, y) {
            return Hit::Outside;
        }
        if close.contains(x, y) {
            return Hit::Close;
        }
        if name.contains(x, y) {
            return Hit::Name;
        }
        if url.contains(x, y) {
            return Hit::Url;
        }
        if cancel.contains(x, y) {
            return Hit::Cancel;
        }
        if create.contains(x, y) {
            return Hit::Create;
        }
        Hit::Inside
    }

    pub fn set_hover(&mut self, viewport: Rect, x: f32, y: f32) -> bool {
        let hit = self.hit(viewport, x, y);
        let hover = matches!(hit, Hit::Cancel | Hit::Create).then_some(hit);
        let changed = self.hover != hover;
        self.hover = hover;
        changed
    }

    pub fn paint(&mut self, list: &mut DrawList, viewport: Rect, p: &Palette) {
        list.rect_alpha(viewport, 0x000000, 0.5);
        let (name, url, cancel, create, close, r) = self.parts(viewport);
        list.rounded_rect(r, 8.0, p.surface);
        list.rounded_border(r, 8.0, p.border);
        list.text(
            Rect::new(
                r.left + 24.0,
                r.top + 24.0,
                r.right - 56.0,
                r.top + 52.0 + large_title_extra(),
            ),
            "新建链接",
            TextStyle::Large,
            p.foreground,
        );
        list.icon_centered(close, Icon::X, 16.0, p.muted);
        list.text(
            Rect::new(name.left, name.top - 24.0, name.right, name.top - 4.0),
            "显示名称",
            TextStyle::Label,
            p.muted,
        );
        list.text(
            Rect::new(url.left, url.top - 24.0, url.right, url.top - 4.0),
            "链接",
            TextStyle::Label,
            p.muted,
        );
        self.name.style = TextStyle::Label;
        self.url.style = TextStyle::Label;
        self.name.paint(
            list,
            name,
            self.active == Field::Name,
            p,
            FieldLook::dialog(p),
        );
        self.url.paint(
            list,
            url,
            self.active == Field::Url,
            p,
            FieldLook::dialog(p),
        );
        if !self.error.is_empty() {
            list.text(
                Rect::new(url.left, url.bottom + 4.0, url.right, url.bottom + 20.0),
                self.error.clone(),
                TextStyle::Caption,
                p.danger,
            );
        }
        if self.hover == Some(Hit::Cancel) {
            list.rounded_rect(cancel, 8.0, p.background);
        }
        list.text_aligned(
            cancel,
            "取消",
            TextStyle::Label,
            p.foreground,
            Align::Center,
        );
        list.glass_button(create, 8.0, p, self.hover == Some(Hit::Create));
        list.text_aligned(
            create,
            "创建",
            TextStyle::Label,
            p.button_foreground(),
            Align::Center,
        );
    }

    pub fn field_text_left(&self, viewport: Rect, field: Field) -> f32 {
        let (name, url, _, _, _, _) = self.parts(viewport);
        match field {
            Field::Name => name.left + 12.0,
            Field::Url => url.left + 12.0,
        }
    }
}
