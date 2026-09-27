//! 把解析后的 AI Markdown 内容转换为可绘制的界面节点。
use super::*;

pub(super) struct Builder {
    pub(super) layout: Layout,
    pub(super) standalone: bool,
    pub(super) base: f32,
}

fn paragraph_card(node: &Node) -> bool {
    matches!(node.kind, Kind::Paragraph)
        && inline_runs(node).iter().any(|r| {
            matches!(r.emphasis, Emphasis::Bold | Emphasis::BoldItalic)
                || matches!(r.emphasis,Emphasis::Styled{flags,..}if flags&1!=0)
        })
        && inline_runs(node).iter().any(|r| {
            matches!(r.emphasis, Emphasis::Italic | Emphasis::BoldItalic)
                || matches!(r.emphasis,Emphasis::Styled{flags,..}if flags&2!=0)
        })
}

impl Builder {
    pub(super) fn scroll_region(
        &mut self,
        node: &Node,
        kind: u8,
        viewport: Rect,
        track: Rect,
        content_width: f32,
        first: usize,
        actions: usize,
        links: usize,
    ) {
        let index = self.layout.scroll_regions.len();
        for action in &mut self.layout.actions[actions..] {
            action.scroll = Some(index);
        }
        for link in &mut self.layout.links[links..] {
            link.scroll = Some(index);
        }
        self.layout.scroll_regions.push(ScrollRegion {
            id: ScrollId {
                anchor: node.anchor,
                kind,
            },
            viewport,
            track,
            content_width,
            items: first..self.layout.items.len(),
        });
    }
    pub(super) fn style(&self, kind: u8) -> TextStyle {
        TextStyle::Ai {
            kind,
            standalone: self.standalone,
        }
    }
    pub(super) fn inline(
        &mut self,
        runs: &[Run],
        x: f32,
        y: f32,
        width: f32,
        style: TextStyle,
        tone: Tone,
    ) -> f32 {
        let parts = runs
            .iter()
            .cloned()
            .map(|run| InlinePart { run, target: None })
            .collect::<Vec<_>>();
        self.inline_parts(&parts, x, y, width, style, tone)
    }
    pub(super) fn inline_parts(
        &mut self,
        parts: &[InlinePart],
        x: f32,
        y: f32,
        width: f32,
        style: TextStyle,
        tone: Tone,
    ) -> f32 {
        let mut paragraphs = vec![Vec::<Run>::new()];
        let mut paragraph_parts = vec![Vec::<InlinePart>::new()];
        for part in parts {
            if part.run.emphasis.base() == Emphasis::Math {
                paragraphs.last_mut().unwrap().push(part.run.clone());
                paragraph_parts.last_mut().unwrap().push(part.clone());
                continue;
            }
            for (i, text) in part.run.text.split('\n').enumerate() {
                if i > 0 {
                    paragraphs.push(Vec::new());
                    paragraph_parts.push(Vec::new());
                }
                if !text.is_empty() {
                    paragraphs.last_mut().unwrap().push(Run {
                        text: text.into(),
                        emphasis: part.run.emphasis,
                    });
                    paragraph_parts.last_mut().unwrap().push(InlinePart {
                        run: Run {
                            text: text.into(),
                            emphasis: part.run.emphasis,
                        },
                        target: part.target.clone(),
                    });
                }
            }
        }
        let mut cy = y;
        for (paragraph, paragraph_parts) in paragraphs.into_iter().zip(paragraph_parts) {
            let wrapped = text::wrap_runs(&paragraph, style, width.max(1.0));
            let original = paragraph_parts
                .iter()
                .flat_map(|part| {
                    part.run
                        .text
                        .chars()
                        .map(move |ch| (ch, part.target.as_deref()))
                })
                .collect::<Vec<_>>();
            let mut original_cursor = 0;
            for row in wrapped {
                let height = text::runs_height(&row, style);
                let mut cx = x;
                for run in row {
                    let w = text::run_width(&run, style);
                    let run_left = cx;
                    let mut decorated = Vec::<(char, Option<&str>)>::new();
                    for ch in run.text.chars() {
                        while original_cursor < original.len() && original[original_cursor].0 != ch
                        {
                            original_cursor += 1;
                        }
                        let target = original
                            .get(original_cursor)
                            .and_then(|(_, target)| *target);
                        if original_cursor < original.len() {
                            original_cursor += 1;
                        }
                        decorated.push((ch, target));
                    }
                    let mut emit = |text: String, target: Option<&str>| {
                        let run = Run {
                            text,
                            emphasis: run.emphasis,
                        };
                        let rect =
                            Rect::new(cx, cy, cx + text::run_width(&run, style), cy + height);
                        if let Some(target) = target.filter(|_| !run.text.is_empty()) {
                            self.layout.links.push(LinkTarget {
                                scroll: None,
                                hit: rect,
                                target: target.to_owned(),
                            });
                        }
                        self.layout.items.push(Item::Text {
                            rect,
                            run,
                            style,
                            tone,
                            syntax: None,
                        });
                        cx = rect.right;
                    };
                    let mut text = String::new();
                    let mut target = None;
                    for (ch, next_target) in decorated {
                        if !text.is_empty() && target != next_target {
                            emit(std::mem::take(&mut text), target);
                        }
                        text.push(ch);
                        target = next_target;
                    }
                    if !text.is_empty() {
                        emit(text, target);
                    }
                    if run.emphasis.base() == Emphasis::Math && !run.text.trim().is_empty() {
                        let rect = Rect::new(run_left, cy, run_left + w, cy + height);
                        self.layout.actions.push(CopyTarget {
                            scroll: None,
                            area: rect,
                            hit: rect,
                            payload: CopyPayload {
                                kind: CopyKind::Formula,
                                text: run.text.trim().into(),
                                html: None,
                            },
                        });
                    }
                }
                self.layout.tail = (cx, cy);
                cy += height;
            }
        }
        cy - y
    }
    pub(super) fn inline_flow(
        &mut self,
        node: &Node,
        x: f32,
        y: f32,
        width: f32,
        tone: Tone,
    ) -> f32 {
        if node.children.is_empty() {
            let parts = inline_parts(node);
            return if parts.is_empty() {
                0.0
            } else {
                self.inline_parts(&parts, x, y, width, self.style(0), tone)
            };
        }
        let mut cy = y;
        let mut bottom = 0.0;
        let mut has_item = false;
        let mut parts = node
            .runs
            .iter()
            .cloned()
            .map(|run| InlinePart { run, target: None })
            .collect::<Vec<_>>();
        for child in &node.children {
            if matches!(child.kind, Kind::Inline | Kind::Link(_)) {
                collect_inline_parts(child, None, &mut parts);
                continue;
            }
            if !parts.is_empty() {
                if has_item {
                    cy += bottom;
                }
                cy += self.inline_parts(&parts, x, cy, width, self.style(0), tone);
                parts.clear();
                bottom = 0.0;
                has_item = true;
            }
            let (top, next) = self.margins(child);
            if has_item {
                cy += bottom.max(top);
            }
            cy += self.node(child, x, cy, width, tone);
            bottom = next;
            has_item = true;
        }
        if !parts.is_empty() {
            if has_item {
                cy += bottom;
            }
            cy += self.inline_parts(&parts, x, cy, width, self.style(0), tone);
        }
        cy - y
    }
    pub(super) fn margins(&self, node: &Node) -> (f32, f32) {
        if paragraph_card(node) {
            return (8.0, 8.0);
        }
        match node.kind {
            Kind::Inline => (0.0, 0.0),
            Kind::Heading(5..=6) => (0.0, 0.0),
            Kind::Heading(level) => {
                let size = self.style(level.min(4)).font_size();
                (0.8 * size, 0.3 * size)
            }
            Kind::Math => (0.7 * self.base, 0.7 * self.base),
            _ => (0.5 * self.base, 0.5 * self.base),
        }
    }
    pub(super) fn children(
        &mut self,
        nodes: &[Node],
        x: f32,
        y: f32,
        width: f32,
        tone: Tone,
    ) -> f32 {
        self.children_in(nodes, x, y, width, tone, false)
    }
    pub(super) fn children_in(
        &mut self,
        nodes: &[Node],
        x: f32,
        y: f32,
        width: f32,
        tone: Tone,
        nested_list: bool,
    ) -> f32 {
        let mut cy = y;
        let mut bottom = 0.0_f32;
        for (i, node) in nodes.iter().enumerate() {
            let (top, next) = if nested_list && matches!(node.kind, Kind::List) {
                (self.base * 0.1, self.base * 0.1)
            } else {
                self.margins(node)
            };
            if i > 0 {
                cy += bottom.max(top);
            }
            cy += self.node(node, x, cy, width, tone);
            bottom = next;
        }
        cy - y
    }
    pub(super) fn node(&mut self, node: &Node, x: f32, y: f32, width: f32, tone: Tone) -> f32 {
        let body = self.style(0);
        match &node.kind {
            Kind::Root => self.children(&node.children, x, y, width, tone),
            Kind::Paragraph if paragraph_card(node) => {
                let first = self.layout.items.len();
                let parts = inline_parts(node);
                let h = self.inline_parts(
                    &parts,
                    x + 15.0,
                    y + 8.0,
                    (width - 27.0).max(1.0),
                    body,
                    tone,
                ) + 16.0;
                self.layout.items.insert(
                    first,
                    Item::ParagraphCard(Rect::new(x, y, x + width, y + h)),
                );
                h
            }
            Kind::Paragraph | Kind::Cell | Kind::Inline => {
                self.inline_flow(node, x, y, width, tone)
            }
            Kind::Heading(level) => self.inline_parts(
                &inline_parts(node),
                x,
                y,
                width,
                self.style(if *level <= 4 { *level } else { 0 }),
                tone,
            ),
            Kind::Code { language } => {
                let style = self.style(5);
                let source = node
                    .runs
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<String>();
                let source = source.strip_suffix('\n').unwrap_or(&source);
                let lines = source.split('\n').map(str::to_owned).collect::<Vec<_>>();
                let tokens = highlight::highlight(language, &lines);
                // pre 继承的行高按正文字号算，而代码是 .9em。
                // 保留现有 28px 内边距作为语言/操作条。这样既维持了
                // Electron 卡片的实测高度，又让操作条独占一行，
                // 不再盖住第一行代码。
                let line_height = body.line_height();
                let natural = lines
                    .iter()
                    .map(|line| text::measure(line, style))
                    .fold(0.0, f32::max);
                let overflow = natural > (width - 28.0).max(1.0);
                let header_height = 28.0;
                let content_height = header_height + lines.len() as f32 * line_height;
                let height = content_height + if overflow { 8.0 } else { 0.0 };
                let card = Rect::new(x, y, x + width, y + height);
                self.layout.items.push(Item::CodeBox(card));
                let display_language = if language.trim().is_empty() {
                    // 与编辑器代码块一致，用同样的显式兜底标签。
                    "plaintext".to_owned()
                } else {
                    language.trim().to_owned()
                };
                self.layout.items.push(Item::CodeHeader {
                    rect: Rect::new(x, y, x + width, y + header_height),
                    language: display_language,
                });

                let button_w = text::measure("复制代码", TextStyle::Small) + 34.0;
                let hit = Rect::from_size(
                    (x + width - 6.0 - button_w).max(x),
                    y + 3.0,
                    button_w.min(width),
                    (header_height - 6.0).max(1.0),
                );
                self.layout.actions.push(CopyTarget {
                    // 操作固定在卡片头部，不随正文滚动，
                    // 代码区可以横向滚动，按钮必须待在原位。
                    scroll: None,
                    area: card,
                    hit,
                    payload: CopyPayload {
                        kind: CopyKind::Code,
                        text: source.to_owned(),
                        html: None,
                    },
                });

                self.layout.items.push(Item::PushClip(Rect::new(
                    if overflow { x } else { x + 14.0 },
                    y + header_height,
                    if overflow {
                        x + width
                    } else {
                        x + (width - 14.0).max(14.0)
                    },
                    y + content_height,
                )));
                let first = self.layout.items.len();
                // 固定头部的操作不要挂到滚动正文上。
                let actions = self.layout.actions.len();
                let links = self.layout.links.len();
                for (i, line) in lines.iter().enumerate() {
                    let cy = y + header_height + i as f32 * line_height;
                    let spans = tokens.get(i).into_iter().flatten();
                    let mut cursor = 0;
                    let mut cx = x + 14.0;
                    for span in spans {
                        // `highlight` 输出的都是合法 UTF-8 边界；这里仍防御性地
                        // 夹紧区间，免得将来 tokenizer 出错时破坏流式渲染帧。
                        let start = span.start.max(cursor).min(line.len());
                        let end = span.end.min(line.len()).max(start);
                        if !line.is_char_boundary(start) || !line.is_char_boundary(end) {
                            continue;
                        }
                        if start > cursor {
                            let text = &line[cursor..start];
                            let w = text::measure(text, style);
                            self.layout.items.push(Item::Text {
                                rect: Rect::new(cx, cy, cx + w, cy + line_height),
                                run: Run::plain(text),
                                style,
                                tone: Tone::Code,
                                syntax: None,
                            });
                            cx += w;
                        }
                        if end > start {
                            let text = &line[start..end];
                            let w = text::measure(text, style);
                            let emphasis = if highlight::italic(span.kind) {
                                Emphasis::Styled {
                                    fg: None,
                                    bg: None,
                                    flags: 2,
                                }
                            } else {
                                Emphasis::None
                            };
                            self.layout.items.push(Item::Text {
                                rect: Rect::new(cx, cy, cx + w, cy + line_height),
                                run: Run {
                                    text: text.to_owned(),
                                    emphasis,
                                },
                                style,
                                tone: Tone::Code,
                                syntax: Some(span.kind),
                            });
                            cx += w;
                        }
                        cursor = end.max(cursor);
                    }
                    if cursor < line.len() {
                        let text = &line[cursor..];
                        let w = text::measure(text, style);
                        self.layout.items.push(Item::Text {
                            rect: Rect::new(cx, cy, cx + w, cy + line_height),
                            run: Run::plain(text),
                            style,
                            tone: Tone::Code,
                            syntax: None,
                        });
                        cx += w;
                    } else if line.is_empty() {
                        self.layout.items.push(Item::Text {
                            rect: Rect::new(cx, cy, cx, cy + line_height),
                            run: Run::plain(""),
                            style,
                            tone: Tone::Code,
                            syntax: None,
                        });
                    }
                    self.layout.tail = (cx, cy);
                }
                if overflow {
                    self.scroll_region(
                        node,
                        0,
                        Rect::new(x, y + header_height, x + width, y + content_height),
                        Rect::new(x, y + content_height, x + width, y + height),
                        natural + 28.0,
                        first,
                        actions,
                        links,
                    );
                }
                self.layout.items.push(Item::PopClip);
                height
            }
            Kind::Quote => {
                let h = self.children(
                    &node.children,
                    x + 15.0,
                    y,
                    (width - 15.0).max(1.0),
                    Tone::Muted,
                );
                self.layout
                    .items
                    .push(Item::QuoteBar(Rect::new(x, y, x + 3.0, y + h)));
                h
            }
            Kind::List => {
                let mut cy = y;
                for (i, item) in node.children.iter().enumerate() {
                    if i > 0 {
                        cy += 0.2 * self.base;
                    }
                    let indent = 1.4 * self.base;
                    // Tailwind preflight 移除了列表标记；AI 样式只恢复缩进。
                    cy += self.node(item, x + indent, cy, (width - indent).max(1.0), tone);
                }
                cy - y
            }
            Kind::Item => {
                let parts = inline_parts(node);
                let h = if parts.is_empty() {
                    0.0
                } else {
                    self.inline_parts(&parts, x, y, width, body, tone)
                };
                let blocks = node
                    .children
                    .iter()
                    .filter(|child| !matches!(child.kind, Kind::Inline | Kind::Link(_)))
                    .cloned()
                    .collect::<Vec<_>>();
                h + self.children_in(&blocks, x, y + h, width, tone, true)
            }
            Kind::Link(_) => self.inline_parts(&inline_parts(node), x, y, width, body, tone),
            Kind::Table => {
                let table_first = self.layout.items.len();
                let actions = self.layout.actions.len();
                let links = self.layout.links.len();
                let cols = node
                    .children
                    .iter()
                    .map(|r| r.children.len())
                    .max()
                    .unwrap_or(1)
                    .max(1);
                let style = self.style(6);
                let mut preferred = vec![17.0_f32; cols];
                let mut minimum = vec![17.0_f32; cols];
                for row in &node.children {
                    for (i, cell) in row.children.iter().enumerate() {
                        let natural = text::measure_runs(&inline_runs(cell), style) + 16.0;
                        preferred[i] = preferred[i].max(natural);
                        for run in &inline_runs(cell) {
                            if run.emphasis.base() == Emphasis::Math {
                                minimum[i] = minimum[i].max(text::run_width(run, style) + 16.0);
                                continue;
                            }
                            let mut word = String::new();
                            for ch in run.text.chars().chain(std::iter::once(' ')) {
                                if ch.is_whitespace() || crate::ui::draw::is_wide(ch) {
                                    minimum[i] = minimum[i].max(
                                        text::run_width(
                                            &Run {
                                                text: std::mem::take(&mut word),
                                                emphasis: run.emphasis,
                                            },
                                            style,
                                        ) + 16.0,
                                    );
                                    if crate::ui::draw::is_wide(ch) {
                                        minimum[i] = minimum[i].max(
                                            text::run_width(
                                                &Run {
                                                    text: ch.to_string(),
                                                    emphasis: run.emphasis,
                                                },
                                                style,
                                            ) + 16.0,
                                        );
                                    }
                                } else {
                                    word.push(ch);
                                }
                            }
                        }
                    }
                }
                let natural = preferred.iter().sum::<f32>();
                let min = minimum.iter().sum::<f32>();
                let widths = (0..cols)
                    .map(|i| {
                        if width >= natural {
                            preferred[i] * width / natural
                        } else if width > min {
                            minimum[i]
                                + (preferred[i] - minimum[i]).max(0.0) * (width - min)
                                    / (natural - min).max(1.0)
                        } else {
                            minimum[i]
                        }
                    })
                    .collect::<Vec<_>>();
                let mut edges = vec![x];
                for w in &widths {
                    edges.push(edges.last().unwrap() + w);
                }
                let mut cy = y;
                for row in &node.children {
                    let header = matches!(row.kind, Kind::Row(true));
                    let mut heights = Vec::new();
                    let first = self.layout.items.len();
                    for (i, cell) in row.children.iter().enumerate() {
                        let cx = edges[i];
                        let cw = widths[i];
                        let mut parts = inline_parts(cell);
                        if header {
                            for part in &mut parts {
                                if part.run.emphasis == Emphasis::None {
                                    part.run.emphasis = Emphasis::Bold;
                                }
                            }
                        }
                        heights.push(self.inline_parts(
                            &parts,
                            cx + 8.0,
                            cy + 6.0,
                            (cw - 16.0).max(1.0),
                            style,
                            tone,
                        ));
                    }
                    let height = heights.into_iter().fold(style.line_height(), f32::max) + 12.0;
                    let backgrounds = (0..cols)
                        .map(|i| {
                            Item::Cell(Rect::new(edges[i], cy, edges[i + 1], cy + height), header)
                        })
                        .collect::<Vec<_>>();
                    self.layout.items.splice(first..first, backgrounds);
                    cy += height;
                }
                let area = Rect::new(x, y, x + width, cy);
                let button_w = text::measure("复制表格", TextStyle::Small) + 18.0;
                let hit = Rect::from_size(
                    (area.right - 6.0 - button_w).max(x),
                    y + 6.0,
                    button_w.min(width),
                    26.0,
                );
                self.layout.actions.push(CopyTarget {
                    scroll: None,
                    area,
                    hit,
                    payload: table_payload(node),
                });
                let natural = edges.last().unwrap() - x;
                if natural > width + 0.01 {
                    self.scroll_region(
                        node,
                        1,
                        area,
                        Rect::new(x, cy, x + width, cy + 8.0),
                        natural,
                        table_first,
                        actions,
                        links,
                    );
                    cy += 8.0;
                }
                cy - y
            }
            Kind::Math => {
                let tex = node
                    .runs
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<String>();
                let size = self.base * 1.08;
                if let Some((w, h)) = crate::ui::math_layout::size_wrapped(&tex, size, width) {
                    let first = self.layout.items.len();
                    let actions = self.layout.actions.len();
                    let left = x + ((width - w) / 2.0).max(0.0);
                    let padding = self.base * 0.15;
                    let rect = Rect::new(left, y + padding, left + w, y + padding + h);
                    if !tex.trim().is_empty() {
                        self.layout.actions.push(CopyTarget {
                            scroll: None,
                            area: rect,
                            hit: rect,
                            payload: CopyPayload {
                                kind: CopyKind::Formula,
                                text: tex.trim().into(),
                                html: None,
                            },
                        });
                    }
                    self.layout.items.push(Item::Math {
                        rect: Rect::new(left, y + padding, left + w, y + padding + h),
                        tex,
                        size,
                        wrap: width,
                    });
                    self.layout.tail = (left + w, y + padding);
                    let height = h + padding * 2.0;
                    if w > width + 0.01 {
                        let links = self.layout.links.len();
                        self.scroll_region(
                            node,
                            2,
                            Rect::new(x, y, x + width, y + height),
                            Rect::new(x, y + height, x + width, y + height + 8.0),
                            w,
                            first,
                            actions,
                            links,
                        );
                        height + 8.0
                    } else {
                        height
                    }
                } else {
                    self.inline(&[Run::plain(tex)], x, y, width, body, tone)
                }
            }
            Kind::Rule => {
                self.layout
                    .items
                    .push(Item::Rule(Rect::new(x, y + 4.0, x + width, y + 5.0)));
                9.0
            }
            Kind::Html => {
                let raw = node
                    .runs
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<String>();
                let lower = raw.trim_start().to_ascii_lowercase();
                if ["<script", "<style", "<iframe", "<object", "<embed"]
                    .iter()
                    .any(|prefix| lower.starts_with(prefix))
                {
                    return 0.0;
                }
                // 不支持的原始 HTML 按可见源码展示，绝不执行。
                self.inline(&[Run::plain(raw)], x, y, width, self.style(5), Tone::Muted)
            }
            Kind::Row(_) => 0.0,
        }
    }
}
