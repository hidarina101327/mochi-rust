//! 计算 AI 附件图片的布局、预览区域和滚动范围。
use super::{assistant::State, layout::Rect};
use std::collections::HashMap;
pub fn file_rects(state: &State, area: Rect, top: f32) -> Vec<Rect> {
    let mut items = Vec::new();
    let mut x = area.left + 44.0;
    for path in &state.pending_files {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let w = (super::text::measure(&name, super::draw::TextStyle::Caption).min(200.0) + 46.0)
            .max(64.0);
        items.push(Rect::from_size(x, top + 12.0, w, 32.0));
        x += w + 8.0;
    }
    let max = (x - 8.0 - (area.right - 44.0)).max(0.0);
    let offset = state.files_scroll.clamp(0.0, max);
    for r in &mut items {
        r.left -= offset;
        r.right -= offset;
    }
    items
}
pub fn files_height(state: &State, _area: Rect) -> f32 {
    if state.pending_files.is_empty() {
        0.0
    } else {
        52.0
    }
}
pub fn files_max_scroll(state: &State, area: Rect) -> f32 {
    let widths: f32 = state
        .pending_files
        .iter()
        .map(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            (super::text::measure(&name, super::draw::TextStyle::Caption).min(200.0) + 46.0)
                .max(64.0)
                + 8.0
        })
        .sum();
    (widths - 8.0 - (area.width() - 88.0).max(1.0)).max(0.0)
}
pub fn remove_placeholder(text: &str, placeholder: &str) -> String {
    let text = text.replacen(placeholder, "", 1);
    let space = |c: char| c.is_whitespace() || c == '\u{feff}';
    let mut out = String::new();
    let mut whitespace = String::new();
    let flush = |out: &mut String, whitespace: &mut String| {
        if whitespace.chars().count() > 1 {
            out.push(' ')
        } else {
            out.push_str(whitespace)
        }
        whitespace.clear();
    };
    for c in text.chars() {
        if space(c) {
            whitespace.push(c)
        } else {
            flush(&mut out, &mut whitespace);
            out.push(c)
        }
    }
    flush(&mut out, &mut whitespace);
    out.trim_matches(space).into()
}
#[derive(Debug, Clone)]
pub struct ImagePreview {
    pub path: String,
    pub width: f32,
    pub height: f32,
}
fn columns(area: Rect) -> usize {
    (((area.width() - 40.0) + 8.0) / 64.0).floor().max(1.0) as usize
}
pub fn pending_height(state: &State, area: Rect) -> f32 {
    if state.pending_images.is_empty() {
        0.0
    } else {
        state.pending_images.len().div_ceil(columns(area)) as f32 * 64.0 + 4.0
    }
}
pub fn pending_rects(state: &State, area: Rect, top: f32) -> Vec<Rect> {
    let cols = columns(area);
    state
        .pending_images
        .iter()
        .enumerate()
        .map(|(i, _)| {
            Rect::from_size(
                area.left + 20.0 + (i % cols) as f32 * 64.0,
                top + 8.0 + (i / cols) as f32 * 64.0,
                56.0,
                56.0,
            )
        })
        .collect()
}
pub fn image_layout(
    refs: &[String],
    previews: &HashMap<String, ImagePreview>,
    width: f32,
    start: f32,
) -> (Vec<(Rect, String)>, f32) {
    let mut items = Vec::new();
    let mut x = 0.0;
    let mut y = start;
    let mut row_h = 0.0_f32;
    for reference in refs {
        let (w, h) = previews
            .get(reference)
            .map(|p| (p.width, p.height))
            .unwrap_or((160.0, 120.0));
        let scale = (width.clamp(1.0, 220.0) / w.max(1.0))
            .min(220.0 / h.max(1.0))
            .min(1.0);
        let (w, h) = (w * scale, h * scale);
        if x > 0.0 && x + w > width {
            y += row_h + 8.0;
            x = 0.0;
            row_h = 0.0;
        }
        items.push((Rect::from_size(x, y, w, h), reference.clone()));
        x += w + 8.0;
        row_h = row_h.max(h);
    }
    let height = if items.is_empty() {
        0.0
    } else {
        y + row_h - start + 8.0
    };
    (items, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn many_files_keep_one_row_and_clipped_hit_targets_follow_scroll() {
        use super::super::assistant::{self, Hit};
        let mut state = State::default();
        state.pending_files = (0..20)
            .map(|i| std::path::PathBuf::from(format!("C:/fixture/很长的文件名-{i}.md")))
            .collect();
        let area = Rect::from_size(0.0, 0.0, 360.0, 800.0);
        let start = assistant::layout(&state, area);
        assert_eq!(files_height(&state, area), 52.0);
        assert!(start.files_max_scroll > 0.0);
        assert!(start.rect_of(Hit::RemoveFile(19)).is_none());
        state.files_scroll = f32::MAX;
        let end = assistant::layout(&state, area);
        let close = end.rect_of(Hit::RemoveFile(19)).unwrap();
        let clip = end.files_viewport.unwrap();
        assert!(close.left >= clip.left && close.right <= clip.right);
        assert_eq!(
            end.hit(
                (close.left + close.right) / 2.0,
                (close.top + close.bottom) / 2.0
            ),
            Some(Hit::RemoveFile(19))
        );
        assert!(end.rect_of(Hit::RemoveFile(0)).is_none());
        assert!(end.rect_of(Hit::Input).unwrap().top > clip.bottom);
        assert_eq!(start.messages_rect, end.messages_rect);
    }
    #[test]
    fn removing_placeholder_matches_composer_whitespace_rules() {
        assert_eq!(
            remove_placeholder("one\nline [image 1] [image 2]", "[image 1]"),
            "one\nline [image 2]"
        );
        assert_eq!(
            remove_placeholder("  one\n\n [image 1]  two ", "[image 1]"),
            "one two"
        );
    }
    #[test]
    fn pending_rows_and_message_images_keep_css_limits() {
        let mut state = State::default();
        state.pending_images = vec!["a".into(); 6];
        let area = Rect::from_size(0.0, 0.0, 300.0, 800.0);
        assert_eq!(pending_height(&state, area), 132.0);
        let boxes = pending_rects(&state, area, 400.0);
        assert_eq!(boxes[0].width(), 56.0);
        assert_eq!(boxes[4].top, 472.0);
        let mut images = HashMap::new();
        images.insert(
            "a".into(),
            ImagePreview {
                path: String::new(),
                width: 1200.0,
                height: 600.0,
            },
        );
        let (items, height) = image_layout(&["a".into(), "a".into()], &images, 300.0, 8.0);
        assert_eq!(items[0].0.width(), 220.0);
        assert_eq!(items[0].0.height(), 110.0);
        assert!(items[1].0.top > items[0].0.bottom);
        assert!(height > 220.0);
    }
}
