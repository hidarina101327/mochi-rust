//! Editor.tsx 的文件名标题区；与 Markdown 正文独立，不进入源码或大纲。
use super::{
    draw::{DrawList, TextStyle},
    layout::Rect,
    settings_values,
    theme::Palette,
};
// 与 Editor.tsx 的布局一致：顶部 32、标题上边距 mt-12、标题高 45/py-1、pb-6、边框和 mb-8。
// document::layout 已经提供了外侧 32px 的顶部留白。
pub const BODY_OFFSET: f32 = 158.0;
pub fn body_offset() -> f32 {
    BODY_OFFSET + TextStyle::DocumentTitle.line_height() - 45.0
}
pub fn rect(area: Rect, scroll: f32) -> Rect {
    let left = area.left
        + super::editor_preferences::current().padding_left
        + settings_values::number("editorLayout.titleMarginLeft", 0.0);
    let top = area.top + super::editor_preferences::current().padding_top + 52.0 - scroll;
    Rect::new(
        left,
        top,
        area.left
            + super::editor_preferences::current().padding_left
            + super::document::content_width(area),
        top + TextStyle::DocumentTitle.line_height(),
    )
}
pub fn paint(list: &mut DrawList, area: Rect, scroll: f32, title: &str, hidden: bool, p: &Palette) {
    let r = rect(area, scroll);
    list.push_clip(area);
    if !hidden {
        list.text(
            r,
            super::text::ellipsize(title, TextStyle::DocumentTitle, r.width()),
            TextStyle::DocumentTitle,
            p.foreground,
        );
    }
    let x = area.left + super::editor_preferences::current().padding_left;
    list.hline(
        x,
        x + super::document::content_width(area),
        area.top + super::editor_preferences::current().padding_top + body_offset() - 33.0 - scroll,
        p.border,
    );
    list.pop_clip();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn title_section_scrolls_without_changing_source_coordinates() {
        let a = Rect::new(20.0, 50.0, 800.0, 700.0);
        let r = rect(a, 10.0);
        assert_eq!(r.top, 124.0);
        assert_eq!(r.height(), 45.0);
    }
}
