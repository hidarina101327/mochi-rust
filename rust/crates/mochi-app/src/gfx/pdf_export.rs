//! 可检索的光栅 PDF：D2D 是视觉上的权威；DirectWrite 提供的
//! Unicode 簇盒子嵌入为「只度量、不绘字」的 Type 3 文本层。
use super::*;
use mochi_core::exports::{PdfPage, PdfText};
use windows::Win32::Graphics::DirectWrite::{DWRITE_CLUSTER_METRICS, DWRITE_HIT_TEST_METRICS};

pub(super) fn export(
    source: &str,
    output: &std::path::Path,
    base_dir: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    struct ComGuard(bool);
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    windows::Win32::System::Com::CoUninitialize();
                }
            }
        }
    }
    // 尊重已存在的 STA。只对本线程成功的调用做配对释放。
    let _com = ComGuard(
        unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            )
        }
        .is_ok(),
    );
    let mut renderer = Renderer::new()?;
    let page = renderer.prepare_snapshot(1190, 1684, 144.0)?;
    let viewport = renderer.viewport();
    let area = Rect::new(32.0, 32.0, viewport.right - 32.0, viewport.bottom - 32.0);
    let parsed = crate::ui::document::parse_ranged(source);
    let images = |src: &str| {
        crate::ui::imginfo::dimensions(std::path::Path::new(
            &crate::ui::document::resolve_image_src(src, base_dir),
        ))
    };
    let layout = crate::ui::document::layout_live(
        &parsed.blocks,
        source,
        None,
        crate::ui::document::content_width(area),
        &images,
    );
    let mut pages = Vec::new();
    let mut pixels = vec![0u8; 1190 * 1684 * 4];
    for (start, end) in page_ranges(&layout, area.height())? {
        let mut list = DrawList::new();
        let page_area = Rect::new(area.left, area.top, area.right, area.top + end - start);
        crate::ui::document::paint_export_in(
            &mut list,
            page_area,
            &layout,
            start,
            base_dir,
            theme::tokens().palette(false),
        );
        renderer.present(HWND(std::ptr::null_mut()), 0xffffff, &list)?;
        unsafe {
            page.0.CopyPixels(std::ptr::null(), 1190 * 4, &mut pixels)?;
        }
        let text = renderer.pdf_text(&list, viewport)?;
        pages.push(PdfPage::from_bgra(1190, 1684, &pixels)?.with_text(text)?);
    }
    let bytes = mochi_core::exports::pdf(&pages)?;
    mochi_core::files::FileService::new().write_bytes_safe(output, &bytes)
}

fn page_ranges(
    layout: &crate::ui::document::Layout,
    height: f32,
) -> anyhow::Result<Vec<(f32, f32)>> {
    let mut result = Vec::new();
    let mut start = 0.0;
    let keep_together = atomic_blocks(layout);
    loop {
        if result.len() >= 200 {
            anyhow::bail!("PDF 导出超过 200 页限制")
        }
        let mut end = (start + height).min(layout.height.max(1.0));
        // 与 Electron 的 print break-inside:avoid 一致，适用于能在一页内
        // 放下的块。放不下的块仍按完整视觉行推进。
        for &(top, bottom) in &keep_together {
            if bottom - top <= height && top > start && top < end && bottom > end {
                end = top;
            }
        }
        // 视觉行是原子的。超大的图片/行保留既有的续页行为；
        // 普通文字绝不会被拦腰截断。
        for line in &layout.lines {
            if line.y >= start
                && line.y < end
                && line.y + line.height > end
                && line.height <= height
            {
                end = end.min(line.y);
            }
        }
        if end <= start {
            end = (start + height).min(layout.height.max(1.0));
        }
        result.push((start, end));
        if end >= layout.height {
            break;
        }
        start = end;
    }
    Ok(result)
}

fn atomic_blocks(layout: &crate::ui::document::Layout) -> Vec<(f32, f32)> {
    use crate::ui::document::{code_card_height, Decoration};
    let mut groups = std::collections::BTreeMap::<usize, (f32, f32, bool)>::new();
    let mut code_ends = std::collections::BTreeMap::<usize, (f32, f32)>::new();
    for line in &layout.lines {
        let bottom = line.y
            + match line.decoration {
                Decoration::CodeHeader {
                    body_lines,
                    collapsed: false,
                } => code_card_height(body_lines),
                _ => line.height,
            };
        let keep = matches!(
            line.decoration,
            Decoration::CodeHeader { .. } | Decoration::TableRow { .. } | Decoration::QuoteBar
        );
        let entry = groups.entry(line.block).or_insert((line.y, bottom, false));
        entry.0 = entry.0.min(line.y);
        entry.1 = entry.1.max(bottom);
        entry.2 |= keep;
        if matches!(line.decoration, Decoration::CodeBackground { .. }) {
            let ends = code_ends
                .entry(line.block)
                .or_insert((line.y + line.height, line.y));
            ends.1 = line.y;
        }
    }
    let mut ranges = groups
        .values()
        .filter_map(|&(top, bottom, keep)| keep.then_some((top, bottom)))
        .collect::<Vec<_>>();
    // 即使代码卡跨页，头部也始终与第一行正文同页，
    // 底部内边距/边框与最后一行同页，绝不出现在空页上。
    for (block, (first_bottom, last_top)) in code_ends {
        if let Some(&(top, bottom, _)) = groups.get(&block) {
            ranges.push((top, first_bottom));
            ranges.push((last_top, bottom));
        }
    }
    ranges.sort_by(|a, b| a.0.total_cmp(&b.0));
    ranges
}

impl Renderer {
    fn pdf_text(&self, list: &DrawList, viewport: Rect) -> anyhow::Result<Vec<PdfText>> {
        let mut result = Vec::new();
        let mut clips = vec![viewport];
        for cmd in list.cmds() {
            match cmd {
                DrawCmd::PushClip { rect } => {
                    clips.push(intersection(*clips.last().unwrap(), *rect))
                }
                DrawCmd::PopClip => {
                    if clips.len() > 1 {
                        clips.pop();
                    }
                }
                DrawCmd::Text {
                    rect,
                    text,
                    style,
                    align,
                    emphasis,
                    ..
                } => {
                    let style = if emphasis.base() == Emphasis::Code && *style == TextStyle::Body {
                        TextStyle::Mono
                    } else {
                        *style
                    };
                    let Some(format) = self.formats.get(&(style, *align, emphasis.base())) else {
                        anyhow::bail!("PDF 文字格式不可用")
                    };
                    if text.is_empty() || rect.is_empty() {
                        continue;
                    }
                    let utf16 = text.encode_utf16().collect::<Vec<_>>();
                    let layout = unsafe {
                        self.dwrite
                            .CreateTextLayout(&utf16, format, rect.width(), rect.height())?
                    };
                    let mut clusters = vec![DWRITE_CLUSTER_METRICS::default(); utf16.len()];
                    let mut count = 0;
                    unsafe {
                        layout.GetClusterMetrics(Some(&mut clusters), &mut count)?;
                    }
                    let mut offset = 0usize;
                    let mut shaped: Vec<(String, Rect)> = Vec::new();
                    let mut prefix = String::new();
                    for cluster in &clusters[..count as usize] {
                        let end = offset + cluster.length as usize;
                        let mut x = 0.0;
                        let mut y = 0.0;
                        let mut metric = DWRITE_HIT_TEST_METRICS::default();
                        unsafe {
                            layout.HitTestTextPosition(
                                offset as u32,
                                false,
                                &mut x,
                                &mut y,
                                &mut metric,
                            )?;
                        }
                        let mut value = String::from_utf16(&utf16[offset..end])?;
                        let bounds = Rect::new(
                            rect.left + metric.left,
                            rect.top + metric.top,
                            rect.left + metric.left + metric.width,
                            rect.top + metric.top + metric.height,
                        );
                        // 有些回退字体把组合标记报成独立的零宽度簇。
                        // 它们必须与宿主文字留在同一片。
                        if metric.width <= 0.0 {
                            if let Some((prior, _)) = shaped.last_mut() {
                                prior.push_str(&value)
                            } else {
                                prefix.push_str(&value)
                            }
                        } else {
                            if !prefix.is_empty() {
                                value = std::mem::take(&mut prefix) + &value;
                            }
                            shaped.push((value, bounds));
                        }
                        offset = end;
                    }
                    for (value, bounds) in shaped {
                        push_cluster(&mut result, value, bounds, *clips.last().unwrap(), viewport);
                    }
                }
                DrawCmd::Math { rect, tex, .. } => {
                    // 复制公式得到的是原始 TeX，而不是分式/矩阵的
                    // 有损 Unicode 近似。
                    let chars = tex.chars().collect::<Vec<_>>();
                    let advance = rect.width() / chars.len().max(1) as f32;
                    for (i, c) in chars.iter().enumerate() {
                        if c.is_control() {
                            continue;
                        }
                        let left = rect.left + i as f32 * advance;
                        push_cluster(
                            &mut result,
                            c.to_string(),
                            Rect::new(left, rect.top, left + advance, rect.bottom),
                            *clips.last().unwrap(),
                            viewport,
                        );
                    }
                }
                _ => {}
            }
        }
        Ok(result)
    }
}

fn intersection(a: Rect, b: Rect) -> Rect {
    Rect::new(
        a.left.max(b.left),
        a.top.max(b.top),
        a.right.min(b.right),
        a.bottom.min(b.bottom),
    )
}

fn push_cluster(out: &mut Vec<PdfText>, text: String, bounds: Rect, clip: Rect, viewport: Rect) {
    // 部分可见的簇只归属到一个竖直切片。被编辑器/表格裁剪矩形
    // 完全遮住的文字绝不暴露。
    let center = (bounds.top + bounds.bottom) / 2.0;
    if center < clip.top || center >= clip.bottom {
        return;
    }
    let visible = intersection(bounds, clip);
    if visible.is_empty() || text.chars().all(char::is_control) {
        return;
    }
    out.push(PdfText {
        text,
        left: visible.left * 595.0 / viewport.width(),
        top: visible.top * 842.0 / viewport.height(),
        width: visible.width() * 595.0 / viewport.width(),
        height: visible.height() * 842.0 / viewport.height(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fitting_code_table_and_quote_blocks_stay_on_one_page() {
        let source = format!(
            "{}\n```rust\nlet a = 1;\nlet b = 2;\n```\n\n| A | B |\n| - | - |\n| c | d |\n\n> {}\n",
            "前文。\n\n".repeat(5),
            "连续引用内容。".repeat(20)
        );
        let parsed = crate::ui::document::parse_ranged(&source);
        let layout =
            crate::ui::document::layout_live(&parsed.blocks, &source, None, 300.0, &|_| None);
        let blocks = atomic_blocks(&layout);
        assert_eq!(blocks.len(), 5); // 三个内容块和代码卡片两侧的边界。
        let ranges = page_ranges(&layout, 240.0).unwrap();
        for (top, bottom) in blocks {
            assert!(bottom - top <= 240.0);
            assert!(
                ranges.iter().any(|&(a, b)| a <= top && bottom <= b),
                "split {top}..{bottom}: {ranges:?}"
            );
        }
    }
    #[test]
    fn print_hides_controls_and_paints_continuation_code_background() {
        let source = format!("```rust\n{}```", "let value = 42;\n".repeat(60));
        let parsed = crate::ui::document::parse_ranged(&source);
        let layout =
            crate::ui::document::layout_live(&parsed.blocks, &source, None, 500.0, &|_| None);
        let area = Rect::new(0.0, 0.0, 560.0, 240.0);
        let palette = theme::tokens().palette(false);
        let mut editor = DrawList::new();
        crate::ui::document::paint_in(&mut editor, area, &layout, 0.0, None, palette);
        assert!(editor
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Icon { .. })));
        let ranges = page_ranges(&layout, area.height()).unwrap();
        assert!(ranges.len() > 3);
        for (top, bottom) in atomic_blocks(&layout)
            .into_iter()
            .filter(|(top, bottom)| bottom - top <= area.height())
        {
            assert!(
                ranges.iter().any(|&(a, b)| a <= top && bottom <= b),
                "code edge split: {top}..{bottom}"
            );
        }
        for (start, end) in ranges {
            let mut list = DrawList::new();
            crate::ui::document::paint_export_in(
                &mut list,
                Rect::new(0.0, 0.0, 560.0, end - start),
                &layout,
                start,
                None,
                palette,
            );
            assert!(!list
                .cmds()
                .iter()
                .any(|c| matches!(c, DrawCmd::Icon { .. })));
            assert!(!list
                .cmds()
                .iter()
                .any(|c| matches!(c,DrawCmd::Text{text,..} if text.contains("代码块名称"))));
            assert!(list.cmds().iter().any(|c|matches!(c,DrawCmd::RoundedRect{rect,color,..} if *color==crate::ui::document::CODE_BG && rect.bottom>0.0)));
        }
    }
    #[test]
    fn directwrite_clusters_keep_unicode_and_alignment() {
        let renderer = Renderer::new().unwrap();
        let viewport = Rect::new(0.0, 0.0, 595.0, 842.0);
        let mut list = DrawList::new();
        list.text_aligned(
            Rect::new(20.0, 20.0, 300.0, 50.0),
            "中文 A😀e\u{301} ffi",
            TextStyle::Document,
            0,
            Align::Trailing,
        );
        let text = renderer.pdf_text(&list, viewport).unwrap();
        assert_eq!(
            text.iter().map(|c| c.text.as_str()).collect::<String>(),
            "中文 A😀e\u{301} ffi"
        );
        assert!(text[0].left > 100.0);
        assert!(text
            .iter()
            .all(|c| c.top >= 20.0 && c.top + c.height <= 50.0 && c.width > 0.0));
        assert!(text.iter().any(|c| c.text == "😀"));
    }
    #[test]
    fn page_breaks_never_split_normal_visual_rows() {
        let source = (0..100)
            .map(|i| format!("段落 {i} 中文测试。\n\n"))
            .collect::<String>();
        let parsed = crate::ui::document::parse_ranged(&source);
        let layout =
            crate::ui::document::layout_live(&parsed.blocks, &source, None, 600.0, &|_| None);
        let ranges = page_ranges(&layout, 240.0).unwrap();
        assert!(ranges.len() > 2);
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].1, pair[1].0);
        }
        for (_, end) in ranges.iter().take(ranges.len() - 1) {
            assert!(!layout
                .lines
                .iter()
                .any(|l| l.y < *end && l.y + l.height > *end));
        }
        assert_eq!(ranges.last().unwrap().1, layout.height);
    }
    #[test]
    fn nested_clipping_excludes_hidden_text() {
        let renderer = Renderer::new().unwrap();
        let viewport = Rect::new(0.0, 0.0, 595.0, 842.0);
        let mut list = DrawList::new();
        list.push_clip(Rect::new(0.0, 0.0, 300.0, 50.0));
        list.text(
            Rect::new(20.0, 60.0, 300.0, 90.0),
            "hidden",
            TextStyle::Document,
            0,
        );
        list.pop_clip();
        list.text(
            Rect::new(20.0, 60.0, 300.0, 90.0),
            "visible",
            TextStyle::Document,
            0,
        );
        let text = renderer.pdf_text(&list, viewport).unwrap();
        assert_eq!(
            text.iter().map(|c| c.text.as_str()).collect::<String>(),
            "visible"
        );
    }
}
