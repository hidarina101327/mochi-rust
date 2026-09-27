//! 公式对话框的几何信息和预览，共用文档的原生渲染器。
use super::{
    draw::{DrawList, TextStyle},
    layout::Rect,
    math_layout,
    theme::Palette,
    widgets::TextField,
};

pub struct Layout {
    pub dialog: Rect,
    pub title: Rect,
    pub editor: Rect,
    pub preview: Rect,
    pub close: Rect,
    pub footer: f32,
}

pub fn layout(viewport: Rect) -> Layout {
    let width = (viewport.width() - 40.0).clamp(280.0, 1040.0);
    let height = (viewport.height() - 48.0).clamp(300.0, 580.0);
    let dialog = Rect::from_size(
        (viewport.left + (viewport.width() - width) / 2.0).round(),
        (viewport.top + (viewport.height() - height) / 2.0).round(),
        width,
        height,
    );
    let title = Rect::from_size(
        dialog.left + 24.0,
        dialog.top + 24.0,
        width - 80.0,
        TextStyle::Large.line_height().max(28.0),
    );
    let footer = dialog.bottom - 60.0;
    let content = Rect::new(
        dialog.left + 24.0,
        dialog.top + 120.0,
        dialog.right - 24.0,
        footer - 32.0,
    );
    let (editor, preview) = if width >= 700.0 {
        let mid = (content.left + content.right) / 2.0;
        (
            Rect::new(content.left, content.top, mid - 10.0, content.bottom),
            Rect::new(mid + 10.0, content.top, content.right, content.bottom),
        )
    } else {
        let mid = (content.top + content.bottom) / 2.0;
        (
            Rect::new(content.left, content.top, content.right, mid - 20.0),
            Rect::new(content.left, mid + 20.0, content.right, content.bottom),
        )
    };
    Layout {
        dialog,
        title,
        editor,
        preview,
        footer,
        close: Rect::from_size(dialog.right - 40.0, dialog.top + 16.0, 24.0, 24.0),
    }
}

pub fn paint(list: &mut DrawList, viewport: Rect, field: &mut TextField, p: &Palette) {
    let layout = layout(viewport);
    for (rect, label) in [(layout.editor, "LaTeX 源码"), (layout.preview, "实时预览")] {
        list.text(
            Rect::new(rect.left, rect.top - 26.0, rect.right, rect.top - 6.0),
            label,
            TextStyle::Label,
            p.muted,
        );
    }
    field.paint_multiline(list, layout.editor, true, p);
    let preview = layout.preview;
    list.rounded_rect(preview, 6.0, p.background);
    list.rounded_border(preview, 6.0, p.border);
    let inner = Rect::new(
        preview.left + 16.0,
        preview.top + 16.0,
        preview.right - 16.0,
        preview.bottom - 16.0,
    );
    list.push_clip(inner);
    let tex = field.text().trim();
    if tex.is_empty() {
        list.text(inner, "输入公式后在这里预览", TextStyle::Label, p.muted);
    } else if let Some((width, height)) = math_layout::size_wrapped(tex, 22.0, inner.width()) {
        let scale = (inner.height() / height.max(1.0))
            .min(inner.width() / width.max(1.0))
            .min(1.0);
        let rect = Rect::from_size(
            inner.left + (inner.width() - width * scale) / 2.0,
            inner.top + (inner.height() - height * scale) / 2.0,
            width * scale,
            height * scale,
        );
        list.math_wrapped(rect, tex, 22.0 * scale, p.foreground, inner.width() * scale);
    } else {
        list.text(
            inner,
            "暂时无法渲染，请检查 LaTeX 语法",
            TextStyle::Label,
            p.danger,
        );
    }
    list.pop_clip();
    list.text(
        Rect::new(
            layout.editor.left,
            layout.footer + 8.0,
            layout.dialog.right - 180.0,
            layout.footer + 28.0,
        ),
        "Enter 换行 · Ctrl+Enter 保存",
        TextStyle::Caption,
        p.muted,
    );
}
