//! 审批预览在测量和绘制时共用一套有界布局。
//! 完整的命令/输出仍保留在 PendingCard::raw 和评审队列里。
use super::*;

const PAD: f32 = 12.0;
const BODY_TOP: f32 = 35.0;
const SECTION_GAP: f32 = 4.0;

/// 定高 AI 标签绝不能把硬换行传给 DirectWrite——即使关闭了自动换行，
/// 它也会照着硬换行折行。
pub fn single_line_preview(value: &str, style: TextStyle, width: f32) -> String {
    let line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    text::ellipsize(&line, style, width.max(1.0))
}

fn preview_lines(value: &str, width: f32, style: TextStyle, limit: usize) -> Vec<String> {
    if limit == 0 || value.is_empty() {
        return Vec::new();
    }
    let normalized = value.replace("\r\n", "\n");
    let mut lines: Vec<String> = normalized
        .split([
            '\n', '\r', '\u{85}', '\u{2028}', '\u{2029}', '\u{b}', '\u{c}',
        ])
        .flat_map(|line| text::wrap_source(line, style, width.max(1.0)))
        .map(|runs| runs.into_iter().map(|run| run.text).collect::<String>())
        .take(limit + 1)
        .collect();
    if lines.len() > limit {
        lines.truncate(limit);
        if let Some(last) = lines.last_mut() {
            *last = text::ellipsize(&format!("{}…", last.trim_end()), style, width);
        }
    }
    lines
}

#[derive(Clone, Copy)]
enum Tone {
    Primary,
    Muted,
    Error,
}

struct PreviewLine {
    text: String,
    top: f32,
    style: TextStyle,
    tone: Tone,
}

struct PreviewLayout {
    lines: Vec<PreviewLine>,
    height: f32,
}

fn push_section(
    lines: &mut Vec<PreviewLine>,
    content: Vec<String>,
    y: &mut f32,
    style: TextStyle,
    tone: Tone,
) {
    if content.is_empty() {
        return;
    }
    for text in content {
        lines.push(PreviewLine {
            text,
            top: *y,
            style,
            tone,
        });
        *y += style.line_height();
    }
    *y += SECTION_GAP;
}

fn preview_layout(card: &PendingCard, width: f32) -> PreviewLayout {
    let width = (width - PAD * 2.0).max(1.0);
    let footer = if card.status == "pending" && card.review.is_available() {
        40.0
    } else {
        8.0
    };
    let error = card
        .error
        .as_deref()
        .map(|error| preview_lines(error, width, TextStyle::Tiny, 2))
        .unwrap_or_default();
    // 先给错误信息和评审控件留位，再分配详情预览。
    let error_height = if error.is_empty() {
        0.0
    } else {
        error.len() as f32 * TextStyle::Tiny.line_height() + SECTION_GAP
    };
    let body_end = PENDING_CARD_MAX_HEIGHT - footer - error_height;
    let mut lines = Vec::new();
    let mut y = BODY_TOP;
    push_section(
        &mut lines,
        preview_lines(&card.summary, width, TextStyle::Small, 3),
        &mut y,
        TextStyle::Small,
        Tone::Primary,
    );
    for (label, value) in &card.details {
        let remaining = ((body_end - y - SECTION_GAP) / TextStyle::Tiny.line_height())
            .floor()
            .max(0.0) as usize;
        if remaining == 0 {
            if let Some(last) = lines.last_mut() {
                last.text = text::ellipsize(
                    &format!("{}…", last.text.trim_end_matches('…')),
                    last.style,
                    width,
                );
            }
            break;
        }
        let content = preview_lines(
            &format!("{label}：{value}"),
            width,
            TextStyle::Tiny,
            remaining.min(3),
        );
        push_section(&mut lines, content, &mut y, TextStyle::Tiny, Tone::Muted);
    }
    push_section(&mut lines, error, &mut y, TextStyle::Tiny, Tone::Error);
    PreviewLayout {
        lines,
        height: (y + footer).clamp(PENDING_CARD_MIN_HEIGHT, PENDING_CARD_MAX_HEIGHT),
    }
}

pub fn pending_card_height(card: &PendingCard, width: f32) -> f32 {
    preview_layout(card, width).height
}

/// 卡片只负责打开评审；批准和拒绝仍由队列 UI 处理。
pub fn pending_card_review_rect(card: &PendingCard, rect: Rect) -> Option<Rect> {
    (card.status == "pending" && card.review.is_available()).then(|| {
        Rect::new(
            rect.left + PAD,
            rect.bottom - 32.0,
            rect.right - PAD,
            rect.bottom - 8.0,
        )
    })
}

pub fn paint_pending_card(
    list: &mut DrawList,
    rect: Rect,
    card: &PendingCard,
    p: &Palette,
) -> Option<Rect> {
    if rect.is_empty() {
        return None;
    }
    let preview = preview_layout(card, rect.width());
    let pending = card.status == "pending";
    let border = if pending {
        theme::mix(p.accent, p.border, 0.2)
    } else {
        p.border
    };
    let radius = super::super::card_radius();
    list.rounded_rect(rect, radius, theme::mix(p.surface_muted, p.surface, 0.5));
    list.rounded_border(rect, radius, border);
    list.push_clip(rect);
    let header_height = TextStyle::Label
        .line_height()
        .max(TextStyle::Small.line_height());
    let title = Rect::new(
        rect.left + PAD,
        rect.top + 8.0,
        rect.right - 92.0,
        rect.top + 8.0 + header_height,
    );
    list.push_clip(title);
    list.text(
        title,
        single_line_preview(
            &format!("{} · {}", card.kind.label(), card.title),
            TextStyle::Label,
            title.width(),
        ),
        TextStyle::Label,
        p.foreground,
    );
    list.pop_clip();
    list.text_aligned(
        Rect::new(rect.right - 88.0, title.top, rect.right - PAD, title.bottom),
        card.status_label(),
        TextStyle::Small,
        if pending { p.muted } else { p.accent },
        Align::Trailing,
    );
    let review = pending_card_review_rect(card, rect);
    let content_bottom = review.map_or(rect.bottom - 8.0, |button| button.top - 8.0);
    let content = Rect::new(
        rect.left + PAD,
        rect.top + BODY_TOP,
        rect.right - PAD,
        content_bottom,
    );
    list.push_clip(content);
    for line in preview.lines {
        let row = Rect::new(
            content.left,
            rect.top + line.top,
            content.right,
            rect.top + line.top + line.style.line_height(),
        );
        if row.bottom > content.bottom {
            break;
        }
        list.text(
            row,
            line.text,
            line.style,
            match line.tone {
                Tone::Primary => p.foreground,
                Tone::Muted => p.muted,
                Tone::Error => p.danger,
            },
        );
    }
    list.pop_clip();
    if let Some(button) = review {
        list.rounded_rect(button, 5.0, p.background);
        list.rounded_border(button, 5.0, p.border);
        list.text_aligned(
            button,
            card.review_label(),
            TextStyle::Small,
            p.foreground,
            Align::Center,
        );
    }
    list.pop_clip();
    review
}
