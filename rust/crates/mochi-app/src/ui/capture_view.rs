//! 原生快速操作面板及其 EDIT 控件共用的几何信息。
use super::{
    draw::{DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
};
pub const PAGES: [&str; 5] = ["收件箱", "最近文档", "收藏", "日程待办", "AI 聊天"];
pub struct Layout {
    pub input: Rect,
    pub save: Rect,
    pub card: Rect,
    pub status: Rect,
    pub close: Rect,
    pub tabs: [Rect; 5],
}
pub fn layout(area: Rect) -> Layout {
    let card = Rect::new(12.0, 12.0, area.right - 12.0, area.bottom - 12.0);
    let tabs = std::array::from_fn(|i| {
        let width = (card.width() - 40.0) / 5.0;
        Rect::new(
            card.left + 20.0 + i as f32 * width,
            card.top + 68.0,
            card.left + 20.0 + (i + 1) as f32 * width - 6.0,
            card.top + 106.0,
        )
    });
    Layout {
        input: Rect::new(
            card.left + 24.0,
            card.top + 126.0,
            card.right - 24.0,
            card.bottom - 78.0,
        ),
        save: Rect::new(
            card.right - 160.0,
            card.bottom - 56.0,
            card.right - 24.0,
            card.bottom - 18.0,
        ),
        status: Rect::new(
            card.left + 24.0,
            card.bottom - 56.0,
            card.right - 174.0,
            card.bottom - 14.0,
        ),
        close: Rect::new(
            card.right - 54.0,
            card.top + 16.0,
            card.right - 18.0,
            card.top + 52.0,
        ),
        card,
        tabs,
    }
}
pub fn capacity(area: Rect) -> usize {
    ((layout(area).input.height() / 62.0) as usize).max(1)
}
pub fn row_rect(area: Rect, row: usize) -> Rect {
    let input = layout(area).input;
    Rect::new(
        input.left,
        input.top + row as f32 * 62.0,
        input.right,
        input.top + row as f32 * 62.0 + 56.0,
    )
}
pub fn paint(
    list: &mut DrawList,
    area: Rect,
    name: Option<&str>,
    error: &str,
    dark: bool,
    page: usize,
    entries: &[crate::capture_window::Entry],
    selected: usize,
) {
    let l = layout(area);
    let bg = if dark { 0x20242b } else { 0xffffff };
    let fg = if dark { 0xe7ebf0 } else { 0x202b3c };
    let muted = if dark { 0xa1abba } else { 0x65748a };
    let border = if dark { 0x353d48 } else { 0xe3e8ef };
    let surface = if dark { 0x282e38 } else { 0xf4f7fb };
    let palette = super::theme::configured_palette(dark);
    let accent = palette.accent;
    let large_title_height = TextStyle::Large.line_height().max(33.0);
    list.rounded_rect(l.card, 18.0, bg);
    list.rounded_border(l.card, 18.0, border);
    list.text(
        Rect::new(
            l.card.left + 24.0,
            l.card.top + 16.0,
            l.card.right - 280.0,
            l.card.top + 16.0 + large_title_height,
        ),
        "快捷空间",
        TextStyle::Large,
        fg,
    );
    list.text(
        Rect::new(
            l.card.right - 278.0,
            l.card.top + 22.0,
            l.card.right - 65.0,
            l.card.top + 48.0,
        ),
        name.unwrap_or("尚未打开工作区"),
        TextStyle::Caption,
        muted,
    );
    list.icon_centered(l.close, Icon::X, 18.0, muted);
    for (i, tab) in l.tabs.iter().enumerate() {
        if i == page {
            list.rounded_rect(*tab, 9.0, surface);
        }
        let color = if i == page { accent } else { muted };
        let icons = [
            Icon::INBOX,
            Icon::CLOCK,
            Icon::STAR,
            Icon::CALENDAR_DAYS,
            Icon::MESSAGE_SQUARE,
        ];
        list.icon_centered(
            Rect::new(tab.left + 10.0, tab.top, tab.left + 32.0, tab.bottom),
            icons[i],
            17.0,
            color,
        );
        list.text(
            Rect::new(tab.left + 40.0, tab.top, tab.right, tab.bottom),
            PAGES[i],
            TextStyle::Caption,
            if i == page { accent } else { muted },
        );
    }
    if page == 0 {
        list.rounded_rect(l.input, 12.0, surface);
        list.rounded_border(l.input, 12.0, border);
        list.text(
            Rect::new(
                l.input.left + 14.0,
                l.input.top + 10.0,
                l.input.right - 14.0,
                l.input.top + 34.0,
            ),
            "随手记下想法，稍后在收件箱整理",
            TextStyle::Caption,
            muted,
        );
    } else if entries.is_empty() {
        list.text(
            Rect::new(
                l.input.left + 20.0,
                l.input.top + 48.0,
                l.input.right - 20.0,
                l.input.top + 48.0 + TextStyle::Large.line_height().max(32.0),
            ),
            if name.is_none() {
                "请先打开工作区"
            } else {
                "这里还没有内容"
            },
            TextStyle::Large,
            muted,
        );
    } else {
        let count = capacity(area);
        let offset = selected / count * count;
        for (row, entry) in entries.iter().skip(offset).take(count).enumerate() {
            let r = row_rect(area, row);
            if offset + row == selected {
                list.rounded_rect(r, 9.0, surface);
            }
            list.text(
                Rect::new(r.left + 14.0, r.top + 5.0, r.right - 16.0, r.top + 29.0),
                &entry.title,
                TextStyle::Body,
                fg,
            );
            list.text(
                Rect::new(r.left + 14.0, r.top + 30.0, r.right - 16.0, r.bottom),
                &entry.detail,
                TextStyle::Caption,
                muted,
            );
        }
    }
    let hint = if !error.is_empty() {
        error.to_owned()
    } else if page == 0 {
        "Ctrl+Enter 存入 · Alt+←/→ 切页\nEsc / Ctrl+Shift+空格 关闭".into()
    } else {
        format!(
            "←/→ 切页 · ↑/↓ 选择 · Enter 打开   {}/{}\nEsc / Ctrl+Shift+空格 关闭",
            if entries.is_empty() { 0 } else { selected + 1 },
            entries.len()
        )
    };
    list.text(
        l.status,
        hint,
        TextStyle::Caption,
        if error.is_empty() { muted } else { 0xc0392b },
    );
    list.glass_button(l.save, 9.0, &palette, false);
    list.text(
        Rect::new(l.save.left + 16.0, l.save.top, l.save.right, l.save.bottom),
        if page == 0 {
            "存入收件箱"
        } else if page == 4 {
            "新建 AI 聊天"
        } else {
            "打开完整页面"
        },
        TextStyle::Caption,
        palette.button_foreground(),
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn card_keeps_editor_tabs_and_footer_separate() {
        for (w, h) in [(800.0, 520.0), (640.0, 420.0)] {
            let area = Rect::new(0.0, 0.0, w, h);
            let l = layout(area);
            assert!(l.input.height() >= 180.0);
            assert!(l.tabs[0].bottom < l.input.top);
            assert!(l.input.bottom < l.save.top);
            assert!(row_rect(area, capacity(area) - 1).bottom <= l.input.bottom);
        }
    }
}
