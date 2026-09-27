//! 判断段落和引用块的连续关系，并确定后续可见内容。
use super::*;

/// 判断两块之间的源码间隔是否恰好是一个物理换行。
///
/// `parse_lines` 刻意按物理 Markdown 行各存一个带范围的块，保证源码偏移
/// 无损。但渲染出来的编辑器里，普通段落或引用仍可能跨这些行：Electron
/// 的 Markdown 渲染器会把这样的行放进同一个 `<p>`/`<blockquote>`，行间
/// 不插块间距。查看原始间隔就能让布局保留这一区分，而不必改动解析器的
/// 源码映射。
fn is_single_line_break(source: &str, start: usize, end: usize) -> bool {
    matches!(source.get(start..end), Some("\n" | "\r\n"))
}

fn paragraph_continues_before(blocks: &[RangedBlock], source: &str, bi: usize) -> bool {
    bi > 0
        && matches!(&blocks[bi - 1].block, Block::Paragraph(_))
        && is_single_line_break(source, blocks[bi - 1].end, blocks[bi].start)
}

fn paragraph_continues_after(blocks: &[RangedBlock], source: &str, bi: usize) -> bool {
    bi + 1 < blocks.len()
        && matches!(&blocks[bi + 1].block, Block::Paragraph(_))
        && is_single_line_break(source, blocks[bi].end, blocks[bi + 1].start)
}

fn quote_continues_before(blocks: &[RangedBlock], source: &str, bi: usize) -> bool {
    bi > 0
        && matches!(&blocks[bi - 1].block, Block::Quote(_))
        && is_single_line_break(source, blocks[bi - 1].end, blocks[bi].start)
}

fn quote_continues_after(blocks: &[RangedBlock], source: &str, bi: usize) -> bool {
    bi + 1 < blocks.len()
        && matches!(&blocks[bi + 1].block, Block::Quote(_))
        && is_single_line_break(source, blocks[bi].end, blocks[bi + 1].start)
}

/// Markdown 空行是分隔符。判断列表是否延续时忽略它们，让 `- a\n\n- b`
/// 仍是一个视觉列表，与 Electron/Tiptap 的 HTML 输出一致。这里也会跳过
/// 单独成行的持久化块标记，因为它们本来就不会成为 `blocks` 的条目。
fn next_visible_block(blocks: &[RangedBlock], from: usize) -> Option<usize> {
    (from..blocks.len()).find(|&index| !matches!(&blocks[index].block, Block::Blank))
}

fn list_continues_after(blocks: &[RangedBlock], bi: usize) -> bool {
    next_visible_block(blocks, bi + 1)
        .is_some_and(|next| matches!(&blocks[next].block, Block::ListItem { .. }))
}

/// 阅读/导出布局：Markdown 分隔行会被折叠。
/// `active` 可选地暴露某一个块的源码。
pub fn layout_live(
    blocks: &[RangedBlock],
    source: &str,
    active: Option<usize>,
    width: f32,
    images: ImageSizer,
) -> Layout {
    layout_document(blocks, source, active, width, images, false)
}

/// 编辑布局为源码空行保留一个稳定、可命中的行，让光标能进入空文档，
/// 连续按 Enter 产生的空行也能逐行导航。
pub fn layout_editor(
    blocks: &[RangedBlock],
    source: &str,
    active: Option<usize>,
    width: f32,
    images: ImageSizer,
) -> Layout {
    layout_document(blocks, source, active, width, images, true)
}

/// 编辑器布局的可续跑状态。`y` 是布局循环使用的绝对文档游标（已含顶部
/// 内边距）；当外部对已折叠代码的调整改变了已生成内容的高度时，调用方
/// 可以在分段之间平移它。
#[derive(Debug, Clone)]
pub struct LayoutProgress {
    pub next_block: usize,
    pub y: f32,
    pub previous_bottom: f32,
    pub has_rendered_block: bool,
    pub(super) started: bool,
    pub(super) finished: bool,
    pub(super) prepared: bool,
    pub(super) has_content: bool,
    pub(super) panel_events: Vec<PanelEvent>,
    pub(super) panel_event_cursor: usize,
    pub(super) panel_depth: usize,
    pub(super) closed_panels: usize,
}

#[derive(Debug, Clone)]
pub(super) struct PanelEvent {
    pub(super) offset: usize,
    pub(super) starts: bool,
    pub(super) closed: bool,
}

impl Default for LayoutProgress {
    fn default() -> Self {
        Self {
            next_block: 0,
            y: padding_top(),
            previous_bottom: 0.0,
            has_rendered_block: false,
            started: false,
            finished: false,
            prepared: false,
            has_content: false,
            panel_events: Vec::new(),
            panel_event_cursor: 0,
            panel_depth: 0,
            closed_panels: 0,
        }
    }
}

impl LayoutProgress {
    pub fn shift_y(&mut self, delta: f32) {
        self.y += delta;
    }

    pub fn is_complete(&self, block_count: usize) -> bool {
        self.next_block >= block_count && self.finished
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// 可编辑文档窗格使用的缓存变体。`layout_editor` 仍是小而免分配的
/// API，供测试和导出助手使用。
pub fn layout_editor_cached(
    blocks: &[RangedBlock],
    source: &str,
    active: Option<usize>,
    width: f32,
    images: ImageSizer,
    cache: &mut LayoutCache,
) -> Layout {
    cache.prepare();
    let layout =
        layout_document_with_cache(blocks, source, active, width, images, true, Some(cache));
    cache.finish_generation();
    layout
}

/// 在 `budget` 内尽量完整地布局块，只返回新产出的行和标题。
/// 返回的坐标是绝对文档坐标，调用方可直接追加到现有 `Layout`。
/// 块总是原子产出；截止时间只在块与块之间检查，绝不截断某块的行。
pub fn layout_editor_chunk(
    blocks: &[RangedBlock],
    source: &str,
    width: f32,
    images: ImageSizer,
    cache: &mut LayoutCache,
    progress: &mut LayoutProgress,
    budget: Duration,
) -> Layout {
    if progress.finished {
        return Layout::default();
    }
    if !progress.started {
        cache.prepare();
        progress.started = true;
    }
    let deadline = Instant::now() + budget;
    let layout = layout_document_chunk_with_cache(
        blocks,
        source,
        None,
        width,
        images,
        true,
        Some(cache),
        progress,
        Some(deadline),
    );
    if progress.next_block >= blocks.len() {
        cache.finish_generation();
        progress.finished = true;
    }
    layout
}

fn layout_document(
    blocks: &[RangedBlock],
    source: &str,
    active: Option<usize>,
    width: f32,
    images: ImageSizer,
    editable: bool,
) -> Layout {
    layout_document_with_cache(blocks, source, active, width, images, editable, None)
}

fn layout_document_with_cache(
    blocks: &[RangedBlock],
    source: &str,
    active: Option<usize>,
    width: f32,
    images: ImageSizer,
    editable: bool,
    mut cache: Option<&mut LayoutCache>,
) -> Layout {
    let mut progress = LayoutProgress::default();
    let mut layout = layout_document_chunk_with_cache(
        blocks,
        source,
        active,
        width,
        images,
        editable,
        cache.as_deref_mut(),
        &mut progress,
        None,
    );
    if let Some(active) = active.filter(|&index| {
        crate::ui::editor_preferences::current().live_line_source
            && !matches!(blocks[index].block, Block::Blank)
    }) {
        // 复用窗格局部的换行缓存：移动光标不能变成对整个文档的
        // 又一次无缓存遍历。
        let stable = layout_document_with_cache(
            blocks,
            source,
            None,
            width,
            images,
            editable,
            cache.as_deref_mut(),
        );
        stabilize_single_line_source_block(&mut layout, &stable, active);
    }
    layout
}

fn layout_document_chunk_with_cache(
    blocks: &[RangedBlock],
    source: &str,
    active: Option<usize>,
    width: f32,
    images: ImageSizer,
    editable: bool,
    mut cache: Option<&mut LayoutCache>,
    progress: &mut LayoutProgress,
    deadline: Option<Instant>,
) -> Layout {
    let text_width = width.min(max_line_width()).max(40.0);
    let mut lines = Vec::new();
    let mut headings = Vec::new();
    let mut y = progress.y;
    let mut previous_bottom = progress.previous_bottom;
    let mut has_rendered_block = progress.has_rendered_block;
    if !progress.prepared {
        progress.has_content = blocks.iter().any(|b| !matches!(b.block, Block::Blank));
        progress.panel_events.clear();
        for block in blocks {
            let Block::Container(panel) = &block.block else {
                continue;
            };
            if panel.body.start >= panel.body.end {
                continue;
            }
            progress.panel_events.push(PanelEvent {
                offset: panel.body.start,
                starts: true,
                closed: !panel.open,
            });
            progress.panel_events.push(PanelEvent {
                offset: panel.body.end,
                starts: false,
                closed: !panel.open,
            });
        }
        // 正文区间是左闭右开的。相邻边界处，前一个面板必须先退出，
        // 后一个才进入；这也让空正文完全落在激活区间之外。
        progress.panel_events.sort_unstable_by(|a, b| {
            a.offset
                .cmp(&b.offset)
                .then_with(|| a.starts.cmp(&b.starts))
        });
        progress.prepared = true;
    }
    let has_content = progress.has_content;
    // Electron/ProseMirror 把空段落当作真实、稳定的编辑器行。
    // 所有可编辑布局（包括缓存的生产路径）都照做。
    // 阅读/导出布局仍会折叠 Markdown 分隔。这一点不能依赖焦点：
    // 挪动光标绝不能让先前的行消失、把文档其余部分上提。
    let preserve_all_blank_rows = editable;
    let start_block = progress.next_block.min(blocks.len());
    progress.next_block = start_block;
    for (bi, rb) in blocks.iter().enumerate().skip(start_block) {
        if bi > start_block && deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        while progress.panel_event_cursor < progress.panel_events.len()
            && progress.panel_events[progress.panel_event_cursor].offset <= rb.start
        {
            let (starts, closed) = {
                let event = &progress.panel_events[progress.panel_event_cursor];
                (event.starts, event.closed)
            };
            if starts {
                progress.panel_depth += 1;
                if closed {
                    progress.closed_panels += 1;
                }
            } else {
                progress.panel_depth = progress.panel_depth.saturating_sub(1);
                if closed {
                    progress.closed_panels = progress.closed_panels.saturating_sub(1);
                }
            }
            progress.panel_event_cursor += 1;
        }
        if editable && progress.closed_panels > 0 {
            progress.next_block = bi + 1;
            continue;
        }
        let indent = progress.panel_depth as f32 * crate::ui::containers::INDENT;
        let text_width = (text_width - indent * 2.0).max(40.0);
        let first_line = lines.len();
        let block = &rb.block;
        if active == Some(bi) || (preserve_all_blank_rows && matches!(block, Block::Blank)) {
            let trailing = match block {
                Block::Heading { level, .. } => {
                    let prefs = crate::ui::editor_preferences::current();
                    let index = level.saturating_sub(1).min(5) as usize;
                    if has_rendered_block {
                        y += (prefs.heading_top[index] - previous_bottom).max(0.0);
                    }
                    prefs.heading_bottom[index]
                }
                Block::Paragraph(_) | Block::Math(_) => {
                    let gap = crate::ui::editor_preferences::current().paragraph_spacing;
                    if has_rendered_block {
                        y += (gap - previous_bottom).max(0.0);
                    }
                    gap
                }
                Block::Code { .. } => code_margin(),
                Block::Table { .. } => table_margin(),
                Block::Image { .. } => block_gap(),
                _ => 0.0,
            };
            y = push_raw(&mut lines, rb, bi, source, text_width, y);
            if let Block::Heading { level, text } = block {
                headings.push(Heading {
                    level: *level,
                    text: text.clone(),
                    y,
                });
            }
            y += trailing;
            previous_bottom = trailing;
            has_rendered_block = true;
            for line in &mut lines[first_line..] {
                line.x += indent;
            }
            progress.next_block = bi + 1;
            continue;
        }
        // Markdown 分隔线属于结构性空白。只有被显式保留的源码行
        //（活动块或上方源码编辑布局）才会进入渲染器；
        // 普通富文本布局在这里折叠它们。
        if matches!(block, Block::Blank) && has_content {
            progress.next_block = bi + 1;
            continue;
        }
        match block {
            Block::Container(panel) => {
                y += block_gap();
                let (background, border) = panel.colors();
                lines.push(LaidOutLine {
                    runs: vec![Run::plain(&panel.title)],
                    style: TextStyle::Title,
                    x: 0.0,
                    y,
                    height: crate::ui::containers::HEADER_HEIGHT,
                    block: bi,
                    source: None,
                    visible_start: usize::MAX,
                    tokens: Vec::new(),
                    decoration: Decoration::Container {
                        end: blocks.partition_point(|b| b.start < panel.body.end),
                        details: panel.kind == crate::ui::containers::Kind::Details,
                        collapsed: editable && !panel.open,
                        background,
                        border,
                    },
                });
                y += crate::ui::containers::HEADER_HEIGHT;
            }
            Block::AiLocator(locator) => {
                y += (12.0 - previous_bottom).max(0.0);
                let factor = crate::ui::editor_preferences::current().line_height;
                let height = 22.0
                    + 13.0 * factor
                    + if locator.snippet.is_empty() {
                        0.0
                    } else {
                        2.0 + 12.0 * factor
                    };
                lines.push(LaidOutLine {
                    runs: vec![Run::plain(&locator.title), Run::plain(&locator.snippet)],
                    style: TextStyle::Title,
                    x: 0.0,
                    y,
                    height,
                    decoration: Decoration::AiLocator,
                    block: bi,
                    source: None,
                    visible_start: 0,
                    tokens: Vec::new(),
                });
                y += height + 12.0;
            }
            Block::ObjectReference(reference) => {
                y += (8.0 - previous_bottom).max(0.0);
                let height = TextStyle::Document.line_height()
                    + if reference.text_style {
                        8.0
                    } else {
                        TextStyle::Small.line_height() + 22.0
                    };
                lines.push(LaidOutLine {
                    runs: vec![
                        Run::plain(&reference.label),
                        Run::plain(&reference.kind),
                        Run::plain(&reference.detail),
                    ],
                    style: TextStyle::Document,
                    x: 0.0,
                    y,
                    height,
                    decoration: Decoration::ObjectReference {
                        text_style: reference.text_style,
                    },
                    block: bi,
                    source: None,
                    visible_start: 0,
                    tokens: Vec::new(),
                });
                y += height + 8.0;
            }
            Block::Blank => {
                y += TextStyle::Document.line_height() * 0.5;
            }
            Block::Divider => {
                y += block_gap();
                lines.push(LaidOutLine {
                    runs: Vec::new(),
                    style: TextStyle::Document,
                    x: 0.0,
                    y,
                    height: 1.0,
                    decoration: Decoration::Divider,
                    block: bi,
                    source: None,
                    visible_start: 0,
                    tokens: Vec::new(),
                });
                y += block_gap() + 1.0;
            }
            Block::Heading { level, text } => {
                let style = match level {
                    1 => TextStyle::Heading1,
                    2 => TextStyle::Heading2,
                    3 => TextStyle::Heading3,
                    4 => TextStyle::Heading4,
                    5 => TextStyle::Heading5,
                    _ => TextStyle::Heading6,
                };
                // 标题上方留白比下方大，与 CSS 的 `margin-top` 惯例一致
                let prefs = crate::ui::editor_preferences::current();
                let hi = level.saturating_sub(1).min(5) as usize;
                let x = prefs.heading_left[hi];
                if has_rendered_block {
                    y += (prefs.heading_top[hi] - previous_bottom).max(0.0);
                }
                headings.push(Heading {
                    level: *level,
                    text: text.clone(),
                    y,
                });
                y = push_wrapped_cached(
                    &mut lines,
                    text,
                    style,
                    x,
                    (text_width - x).max(40.0),
                    y,
                    Decoration::None,
                    bi,
                    &mut cache,
                );
                if crate::ui::editor_preferences::current().heading_underline
                    [level.saturating_sub(1).min(5) as usize]
                {
                    if let Some(line) = lines.last_mut() {
                        line.decoration = Decoration::HeadingUnderline;
                        line.height += 9.0;
                        y += 9.0;
                    }
                }
                y += prefs.heading_bottom[hi];
            }
            Block::Paragraph(t) => {
                let prefs = crate::ui::editor_preferences::current();
                let continuation_before = paragraph_continues_before(blocks, source, bi);
                if has_rendered_block && !continuation_before {
                    y += (prefs.paragraph_spacing - previous_bottom).max(0.0);
                }
                y = push_wrapped_cached(
                    &mut lines,
                    t,
                    TextStyle::Document,
                    0.0,
                    text_width,
                    y,
                    Decoration::None,
                    bi,
                    &mut cache,
                );
                if !paragraph_continues_after(blocks, source, bi) {
                    y += prefs.paragraph_spacing;
                }
            }
            Block::Aligned { align, level, text } => {
                let style = match level {
                    1 => TextStyle::Heading1,
                    2 => TextStyle::Heading2,
                    3 => TextStyle::Heading3,
                    4 => TextStyle::Heading4,
                    5 => TextStyle::Heading5,
                    6 => TextStyle::Heading6,
                    _ => TextStyle::Document,
                };
                if *level > 0 {
                    y += block_gap() * 1.6;
                    headings.push(Heading {
                        level: *level,
                        text: text.clone(),
                        y,
                    });
                }
                let start = lines.len();
                y = push_wrapped_cached(
                    &mut lines,
                    text,
                    style,
                    0.0,
                    text_width,
                    y,
                    Decoration::None,
                    bi,
                    &mut cache,
                );
                for line in &mut lines[start..] {
                    let w = crate::ui::text::measure_runs(&line.runs, style);
                    line.x = match align {
                        crate::ui::draw::Align::Center => ((text_width - w) / 2.0).max(0.0),
                        crate::ui::draw::Align::Trailing => (text_width - w).max(0.0),
                        _ => 0.0,
                    };
                }
                if *level > 0 {
                    y += block_gap() * 0.4;
                }
            }
            Block::Math(tex) => {
                let gap = crate::ui::editor_preferences::current().paragraph_spacing;
                if has_rendered_block {
                    y += (gap - previous_bottom).max(0.0);
                }
                let font_size = TextStyle::Document.font_size() * text::MATH_SCALE;
                let height = crate::ui::math_layout::size(tex, font_size)
                    .map(|(w, h)| h * (text_width / w.max(1.0)).min(1.0) + 4.0)
                    .unwrap_or(TextStyle::Document.line_height())
                    .max(TextStyle::Document.line_height());
                lines.push(LaidOutLine {
                    runs: vec![Run::plain(tex)],
                    style: TextStyle::Document,
                    x: 0.0,
                    y,
                    height,
                    decoration: Decoration::Math,
                    block: bi,
                    source: None,
                    visible_start: 0,
                    tokens: Vec::new(),
                });
                y += height;
                y += gap;
            }
            Block::Quote(t) => {
                let continuation_before = quote_continues_before(blocks, source, bi);
                if has_rendered_block && !continuation_before {
                    y += (block_gap() - previous_bottom).max(0.0);
                }
                let x = quote_bar() + quote_indent();
                y = push_wrapped_cached(
                    &mut lines,
                    t,
                    TextStyle::Document,
                    x,
                    text_width - x,
                    y,
                    Decoration::QuoteBar,
                    bi,
                    &mut cache,
                );
                if !quote_continues_after(blocks, source, bi) {
                    y += block_gap();
                }
            }
            Block::ListItem {
                marker,
                text,
                depth,
            } => {
                let indent = *depth as f32 * list_indent();
                let style = TextStyle::Document;
                // 项目符号与首行同一条基线；后续行对齐到文字而不是符号
                lines.push(LaidOutLine {
                    runs: vec![Run {
                        text: marker.clone(),
                        emphasis: Emphasis::None,
                    }],
                    style,
                    x: indent,
                    y,
                    height: style.line_height(),
                    decoration: Decoration::None,
                    block: bi,
                    source: None,
                    visible_start: usize::MAX,
                    tokens: Vec::new(),
                });
                // 多位数编号可能超出配置的列表缩进。
                // 按实测宽度加上间隙预留，大字号下也一样。
                let marker_width = text::measure(marker, style);
                let marker_gap = style.font_size() * 0.35;
                let text_x = indent + list_indent().max(marker_width + marker_gap);
                y = push_wrapped_cached(
                    &mut lines,
                    text,
                    style,
                    text_x,
                    text_width - text_x,
                    y,
                    Decoration::None,
                    bi,
                    &mut cache,
                );
                let prefs = crate::ui::editor_preferences::current();
                let item_spacing = prefs.list_spacing;
                y += item_spacing;
                if !list_continues_after(blocks, bi) {
                    // 列表容器的下边距与最后一条的边距会折叠。
                    // 只补差的那部分，两个圆点之间的空分隔
                    // 才不会多出一行视觉行。
                    y += (prefs.paragraph_spacing - item_spacing).max(0.0);
                }
            }
            Block::Code { lang, lines: body } => {
                let style = TextStyle::DocumentMono;
                if has_rendered_block {
                    y += (code_margin() - previous_bottom).max(0.0);
                } else {
                    y += code_margin();
                }
                // 卡片头部：语言名（没有就留空）；它同时负责画整张卡片的底色与边框
                lines.push(LaidOutLine {
                    runs: vec![
                        Run::plain(lang.clone()),
                        Run::plain(crate::ui::code_blocks::metadata(source, rb.start).1.title),
                    ],
                    style: TextStyle::Caption,
                    x: 0.0,
                    y,
                    height: code_header_height(),
                    decoration: Decoration::CodeHeader {
                        body_lines: body.len(),
                        collapsed: false,
                    },
                    block: bi,
                    source: None,
                    visible_start: usize::MAX,
                    tokens: Vec::new(),
                });
                let header_index = lines.len() - 1;
                y += code_header_height() + code_pad_y();
                // 代码行保留原始字节范围；默认软换行，必要时仍可切换为横向滚动。
                let mut src = rb.start
                    + source
                        .get(rb.start..rb.end)
                        .and_then(|s| s.find('\n'))
                        .map(|i| i + 1)
                        .unwrap_or(0);
                let colored = highlight::highlight(lang, body);
                let line_number_width = code_line_number_width(body.len());
                let show_line_numbers =
                    crate::ui::editor_preferences::current().code_show_line_numbers;
                let code_width = (text_width - code_pad_x() * 2.0 - line_number_width).max(1.0);
                let mut visual_lines = 0usize;
                for (li, raw) in body.iter().enumerate() {
                    let wrapped = if crate::ui::editor_preferences::current().code_wrap {
                        text::wrap_source(raw, style, code_width)
                    } else {
                        vec![vec![Run::plain(raw)]]
                    };
                    let mut local = 0usize;
                    for (visual, runs) in wrapped.into_iter().enumerate() {
                        let len: usize = runs.iter().map(|run| run.text.len()).sum();
                        let exact = (source.get(src + local..src + local + len)
                            == raw.get(local..local + len))
                        .then_some((src + local, src + local + len));
                        lines.push(LaidOutLine {
                            runs,
                            style,
                            x: code_pad_x() + line_number_width,
                            y,
                            height: style.line_height(),
                            decoration: Decoration::CodeBackground {
                                line_number: (visual == 0 && show_line_numbers).then_some(li + 1),
                            },
                            block: bi,
                            source: exact,
                            visible_start: 0,
                            tokens: code_tokens_in_range(
                                colored.get(li).map(Vec::as_slice).unwrap_or_default(),
                                local,
                                local + len,
                            ),
                        });
                        local += len;
                        y += style.line_height();
                        visual_lines += 1;
                    }
                    src += raw.len()
                        + if source
                            .get(src + raw.len()..)
                            .is_some_and(|s| s.starts_with("\r\n"))
                        {
                            2
                        } else {
                            1
                        };
                }
                if body.is_empty() {
                    lines.push(LaidOutLine {
                        runs: vec![Run::plain("")],
                        style,
                        x: code_pad_x() + line_number_width,
                        y,
                        height: style.line_height(),
                        decoration: Decoration::CodeBackground {
                            line_number: show_line_numbers.then_some(1),
                        },
                        block: bi,
                        source: Some((src, src)),
                        visible_start: 0,
                        tokens: Vec::new(),
                    });
                    y += style.line_height();
                    visual_lines = 1;
                }
                lines[header_index].decoration = Decoration::CodeHeader {
                    body_lines: visual_lines,
                    collapsed: false,
                };
                y += code_pad_y() + code_scrollbar_height() + 1.0 + code_margin();
            }
            Block::Table { rows, header } => {
                let cols = rows.first().map(Vec::len).unwrap_or(0);
                if has_rendered_block {
                    y += (table_margin() - previous_bottom).max(0.0);
                } else {
                    y += table_margin();
                }
                for (ri, row) in rows.iter().enumerate() {
                    lines.push(LaidOutLine {
                        runs: row.iter().map(|c| Run::plain(c.clone())).collect(),
                        style: TextStyle::Table,
                        x: 0.0,
                        y,
                        height: table_row_height(),
                        decoration: Decoration::TableRow {
                            cols,
                            header: *header && ri == 0,
                        },
                        block: bi,
                        source: None,
                        visible_start: usize::MAX,
                        tokens: Vec::new(),
                    });
                    y += table_row_height();
                }
                y += table_margin();
            }
            Block::Image { alt, src, width } => {
                let requested_width = width
                    .as_ref()
                    .and_then(|w| crate::ui::editor_images::parse_width(w, text_width))
                    .unwrap_or(text_width);
                // `max-width: 100%`，高度随图；1 像素 = 1 DIP。读不到尺寸就用占位高
                // 这里的 shown_width 必须同时供绘制、命中与拖拽手柄使用。此前无
                // width 属性时高度用了原图宽度，绘制却用了整行宽度，造成拉伸和手柄
                // 偏离视觉右下角。
                let (shown_width, height) = match images(src) {
                    Some((w, h)) if w > 0 && h > 0 => {
                        let shown_w = if width.is_some() {
                            requested_width
                        } else {
                            (w as f32).min(requested_width)
                        };
                        (shown_w, (h as f32 * shown_w / w as f32).max(24.0).round())
                    }
                    _ => (requested_width, IMAGE_PLACEHOLDER_HEIGHT),
                };
                if has_rendered_block {
                    y += (block_gap() - previous_bottom).max(0.0);
                } else {
                    y += block_gap();
                }
                lines.push(LaidOutLine {
                    runs: vec![
                        Run::plain(alt.clone()),
                        Run::plain(src.clone()),
                        Run::plain(shown_width.to_string()),
                    ],
                    style: TextStyle::Caption,
                    x: 0.0,
                    y,
                    height,
                    decoration: Decoration::Image,
                    block: bi,
                    source: None,
                    visible_start: usize::MAX,
                    tokens: Vec::new(),
                });
                y += height + block_gap();
            }
        }
        previous_bottom = match block {
            Block::AiLocator(_) => 12.0,
            Block::ObjectReference(_) => 8.0,
            Block::Heading { level, .. } => {
                crate::ui::editor_preferences::current().heading_bottom
                    [level.saturating_sub(1).min(5) as usize]
            }
            Block::Paragraph(_) if paragraph_continues_after(blocks, source, bi) => 0.0,
            Block::Paragraph(_) | Block::Math(_) => {
                crate::ui::editor_preferences::current().paragraph_spacing
            }
            Block::Code { .. } => code_margin(),
            Block::Table { .. } => table_margin(),
            Block::Image { .. } => block_gap(),
            Block::Quote(_) if quote_continues_after(blocks, source, bi) => 0.0,
            Block::Quote(_) => block_gap(),
            Block::ListItem { .. } if list_continues_after(blocks, bi) => {
                crate::ui::editor_preferences::current().list_spacing
            }
            Block::ListItem { .. } => crate::ui::editor_preferences::current().paragraph_spacing,
            _ => 0.0,
        };
        for line in &mut lines[first_line..] {
            line.x += indent;
        }
        has_rendered_block = true;
        progress.next_block = bi + 1;
    }

    progress.y = y;
    progress.previous_bottom = previous_bottom;
    progress.has_rendered_block = has_rendered_block;

    Layout {
        lines,
        height: y + padding_bottom(),
        headings,
        table_widths: HashMap::new(),
    }
}

/// 活动的 Markdown 源码能在一行内放下时，保持渲染块的范围不变。
/// 多行源码刻意不加约束：藏字或压字，比让一次真正扩大的编辑
/// 撑高它的块更糟。
fn stabilize_single_line_source_block(layout: &mut Layout, stable: &Layout, active: usize) {
    let source_lines: Vec<usize> = layout
        .lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            (line.block == active && line.source.is_some()).then_some(index)
        })
        .collect();
    if source_lines.len() != 1 {
        return;
    }

    let next_y = |layout: &Layout| {
        layout
            .lines
            .iter()
            .filter(|line| line.block > active)
            .map(|line| line.y)
            .min_by(f32::total_cmp)
            .unwrap_or(layout.height)
    };
    let delta = next_y(stable) - next_y(layout);
    if delta.abs() < 0.01 {
        return;
    }
    let line = &mut layout.lines[source_lines[0]];
    // 光标所在行绝不能缩成不可命中的细条。delta 为负说明源码
    // 真的需要更多空间，因此保留其高度。
    if line.height + delta < TextStyle::Document.line_height() * 0.75 {
        return;
    }
    line.height += delta;
    let source_y = line.y;
    for later in layout.lines.iter_mut().filter(|line| line.block > active) {
        later.y += delta;
    }
    for heading in layout
        .headings
        .iter_mut()
        .filter(|heading| heading.y > source_y)
    {
        heading.y += delta;
    }
    layout.height += delta;
}

/// 活动块：源码逐行原样排版。每个视觉行带精确的字节范围。
fn push_raw(
    out: &mut Vec<LaidOutLine>,
    rb: &RangedBlock,
    bi: usize,
    source: &str,
    width: f32,
    mut y: f32,
) -> f32 {
    let (style, x, decoration) = match &rb.block {
        Block::Heading { level: 1, .. } => (TextStyle::Heading1, 0.0, Decoration::None),
        Block::Heading { level: 2, .. } => (TextStyle::Heading2, 0.0, Decoration::None),
        Block::Heading { level: 3, .. } => (TextStyle::Heading3, 0.0, Decoration::None),
        Block::Heading { level: 4, .. } => (TextStyle::Heading4, 0.0, Decoration::None),
        Block::Heading { level: 5, .. } => (TextStyle::Heading5, 0.0, Decoration::None),
        Block::Heading { .. } => (TextStyle::Heading6, 0.0, Decoration::None),
        Block::Code { .. } => (
            TextStyle::DocumentMono,
            code_pad_x(),
            Decoration::CodeBackground { line_number: None },
        ),
        Block::Quote(_) => (
            TextStyle::Document,
            quote_bar() + quote_indent(),
            Decoration::QuoteBar,
        ),
        _ => (TextStyle::Document, 0.0, Decoration::None),
    };
    let text = source.get(rb.start..rb.end).unwrap_or("");
    // 活动的代码块：围栏行也露出来，整体仍套在卡片里（头部留空，语言名就在围栏行上）
    let is_code = matches!(rb.block, Block::Code { .. });
    let (lang, colored) = match &rb.block {
        Block::Code { lang, .. } => {
            let raw_lines: Vec<String> = text
                .split('\n')
                .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
                .collect();
            (lang.clone(), highlight::highlight(lang, &raw_lines))
        }
        _ => (String::new(), Vec::new()),
    };
    if is_code {
        let n = text.split('\n').count();
        y += code_margin();
        out.push(LaidOutLine {
            runs: vec![Run::plain(lang)],
            style: TextStyle::Caption,
            x: 0.0,
            y,
            height: code_header_height(),
            decoration: Decoration::CodeHeader {
                body_lines: n,
                collapsed: false,
            },
            block: bi,
            source: None,
            visible_start: usize::MAX,
            tokens: Vec::new(),
        });
        y += code_header_height() + code_pad_y();
    }
    let code_line_count = text.split('\n').count();
    let line_number_width = code_line_number_width(code_line_count);
    let show_line_numbers = crate::ui::editor_preferences::current().code_show_line_numbers;
    let code_width = (width - code_pad_x() * 2.0 - line_number_width).max(1.0);
    let code_header_index = is_code.then(|| out.len() - 1);
    let mut code_visual_lines = 0usize;
    let mut offset = rb.start;
    for (i, para) in text.split('\n').enumerate() {
        if i > 0 {
            offset += 1;
        }
        let source_len = para.len();
        let para = para.strip_suffix('\r').unwrap_or(para);
        // 代码也遵循用户的换行偏好；源码范围和高亮范围都按视觉行拆分。
        let wrapped = if is_code {
            if crate::ui::editor_preferences::current().code_wrap {
                text::wrap_source(para, style, code_width)
            } else {
                vec![vec![Run::plain(para)]]
            }
        } else {
            text::wrap_source(para, style, (width - x).max(40.0))
        };
        let mut local = 0usize;
        for (visual, line_runs) in wrapped.into_iter().enumerate() {
            let len: usize = line_runs.iter().map(|r| r.text.len()).sum();
            out.push(LaidOutLine {
                runs: line_runs,
                style,
                x: if is_code { x + line_number_width } else { x },
                y,
                height: style.line_height(),
                decoration: if is_code {
                    Decoration::CodeBackground {
                        line_number: (visual == 0 && show_line_numbers).then_some(i + 1),
                    }
                } else {
                    decoration
                },
                block: bi,
                source: Some((offset + local, offset + local + len)),
                visible_start: 0,
                tokens: if is_code {
                    code_tokens_in_range(
                        colored.get(i).map(Vec::as_slice).unwrap_or_default(),
                        local,
                        local + len,
                    )
                } else {
                    Vec::new()
                },
            });
            local += len;
            y += style.line_height();
            code_visual_lines += usize::from(is_code);
        }
        offset += source_len;
    }
    if is_code {
        if let Some(header) = code_header_index.and_then(|index| out.get_mut(index)) {
            header.decoration = Decoration::CodeHeader {
                body_lines: code_visual_lines,
                collapsed: false,
            };
        }
        y += code_pad_y() + code_scrollbar_height() + 1.0 + code_margin();
    }
    y
}

#[allow(clippy::too_many_arguments)]
fn push_wrapped_cached(
    out: &mut Vec<LaidOutLine>,
    text: &str,
    style: TextStyle,
    x: f32,
    width: f32,
    mut y: f32,
    decoration: Decoration,
    block: usize,
    cache: &mut Option<&mut LayoutCache>,
) -> f32 {
    let wrapped = if let Some(cache) = cache.as_deref_mut() {
        cache.wrapped(text, style, width)
    } else {
        let runs = text::parse_inline(text);
        crate::ui::math_runs::wrap_mapped(&runs, style, width)
            .into_iter()
            .map(|(runs, visible_start)| WrappedLine {
                height: text::runs_height(&runs, style),
                runs,
                visible_start,
            })
            .collect()
    };
    for line in wrapped {
        out.push(LaidOutLine {
            runs: line.runs,
            style,
            x,
            y,
            height: line.height,
            decoration,
            block,
            source: None,
            visible_start: line.visible_start,
            tokens: Vec::new(),
        });
        y += line.height;
    }
    y
}

/// 将原始代码行的着色范围裁到一个视觉续行，并把字节下标改成续行内坐标。
fn code_tokens_in_range(
    tokens: &[highlight::Span],
    start: usize,
    end: usize,
) -> Vec<highlight::Span> {
    tokens
        .iter()
        .filter_map(|token| {
            let left = token.start.max(start);
            let right = token.end.min(end);
            (left < right).then(|| highlight::Span {
                start: left - start,
                end: right - start,
                kind: token.kind,
            })
        })
        .collect()
}

#[cfg(test)]
mod code_token_tests {
    use super::*;

    #[test]
    fn wrapped_code_tokens_skip_ranges_before_and_after_the_visual_line() {
        let tokens = [
            highlight::Span {
                start: 0,
                end: 3,
                kind: highlight::Token::Keyword,
            },
            highlight::Span {
                start: 6,
                end: 12,
                kind: highlight::Token::String,
            },
            highlight::Span {
                start: 20,
                end: 24,
                kind: highlight::Token::Comment,
            },
        ];
        assert_eq!(
            code_tokens_in_range(&tokens, 8, 16),
            vec![highlight::Span {
                start: 0,
                end: 4,
                kind: highlight::Token::String
            },]
        );
        assert!(code_tokens_in_range(&tokens, 12, 20).is_empty());
    }
}
