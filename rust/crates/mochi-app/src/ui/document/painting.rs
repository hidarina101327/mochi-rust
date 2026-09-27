//! 绘制文档内容、导出内容，并解析图片资源路径。
use super::*;

/// 画文档。`scroll` 是已滚过的像素。
/// 没有图片基准目录的版本（测试用；产品代码走 [`paint_in`]）。
#[cfg(test)]
pub fn paint(list: &mut DrawList, area: Rect, doc: &Layout, scroll: f32, p: &Palette) {
    paint_with_padding(list, area, doc, scroll, padding_x(), None, p, false, None);
}

/// 带图片基准目录的版本：相对路径的 `![](./a.png)` 按文档所在目录解析。
pub fn paint_in(
    list: &mut DrawList,
    area: Rect,
    doc: &Layout,
    scroll: f32,
    base_dir: Option<&std::path::Path>,
    p: &Palette,
) {
    paint_with_padding(
        list,
        area,
        doc,
        scroll,
        padding_x(),
        base_dir,
        p,
        false,
        None,
    );
}

/// 编辑器窗格会在布局旁保留这份小型背景索引，
/// 滚动时就不必遍历前面所有文本行来查找跨行卡片。
pub fn paint_in_indexed(
    list: &mut DrawList,
    area: Rect,
    doc: &Layout,
    scroll: f32,
    base_dir: Option<&std::path::Path>,
    p: &Palette,
    backgrounds: &[usize],
) {
    paint_with_padding(
        list,
        area,
        doc,
        scroll,
        padding_x(),
        base_dir,
        p,
        false,
        Some(backgrounds),
    );
}

/// 静态打印视图。不要把编辑器控件放进图像或
/// 可搜索文本中；后续页面仍要绘制从可视区域外延续过来的代码卡片背景。
pub fn paint_export_in(
    list: &mut DrawList,
    area: Rect,
    doc: &Layout,
    scroll: f32,
    base_dir: Option<&std::path::Path>,
    p: &Palette,
) {
    paint_with_padding(
        list,
        area,
        doc,
        scroll,
        padding_x(),
        base_dir,
        p,
        true,
        None,
    );
}

/// 把 `![](src)` 的 src 变成 gfx 能打开的东西：URL 原样；相对路径拼到文档目录；`%20` 等还原。
pub fn resolve_image_src(src: &str, base_dir: Option<&std::path::Path>) -> String {
    let lower = src.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("data:") {
        return src.to_owned();
    }
    let decoded = percent_decode(src.strip_prefix("file:///").unwrap_or(src));
    let path = std::path::Path::new(&decoded);
    if path.is_absolute() {
        return decoded;
    }
    match base_dir {
        Some(base) => base
            .join(decoded.trim_start_matches("./"))
            .to_string_lossy()
            .into_owned(),
        None => decoded,
    }
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_owned();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(v) = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn container_painted_height(doc: &Layout, line: &LaidOutLine, end: usize) -> f32 {
    let start = doc.block_line_range(line.block).start;
    let end = doc.lines.partition_point(|candidate| candidate.block < end);
    let extent = doc
        .lines
        .get(end.saturating_sub(1))
        .filter(|candidate| candidate.block >= line.block && end > start)
        .map(|candidate| (candidate.y + candidate.height - line.y).max(line.height))
        .unwrap_or(line.height);
    extent + 4.0
}

fn painted_height(doc: &Layout, line: &LaidOutLine) -> f32 {
    match line.decoration {
        Decoration::Container {
            end,
            collapsed: false,
            ..
        } => container_painted_height(doc, line, end),
        Decoration::CodeHeader {
            body_lines,
            collapsed: false,
        } => code_card_height(body_lines),
        _ => line.height,
    }
}

fn code_line_number_width_for_block(doc: &Layout, block: usize) -> f32 {
    let number_of_lines = doc
        .lines_for_block(block)
        .iter()
        .filter_map(|candidate| match candidate.decoration {
            Decoration::CodeBackground { line_number } => line_number,
            _ => None,
        })
        .max()
        .unwrap_or(1);
    code_line_number_width(number_of_lines)
}

fn cached_code_line_number_width(
    widths: &mut HashMap<usize, f32>,
    doc: &Layout,
    block: usize,
) -> f32 {
    if let Some(width) = widths.get(&block) {
        return *width;
    }
    let width = code_line_number_width_for_block(doc, block);
    widths.insert(block, width);
    width
}

fn paint_with_padding(
    list: &mut DrawList,
    area: Rect,
    doc: &Layout,
    scroll: f32,
    pad_x: f32,
    base_dir: Option<&std::path::Path>,
    p: &Palette,
    printing: bool,
    backgrounds: Option<&[usize]>,
) {
    if area.is_empty() {
        return;
    }
    list.push_clip(area);

    let origin_x = area.left + pad_x;
    let top = area.top - scroll;

    // 普通行的底部位置单调递增，因此可用它查找视口
    // 范围，同时保留起点位于视口上方的背景绘制逻辑。
    // 当容器或代码块标题较大的绘制区域越过顶部边缘时，
    // 再把这些标题补回绘制结果中。
    let first_visible = doc
        .lines
        .partition_point(|line| top + line.y + line.height < area.top);
    let last_visible = doc
        .lines
        .partition_point(|line| top + line.y <= area.bottom);
    let mut spanning_headers = Vec::new();
    let mut inspect = |index: usize| {
        let line = &doc.lines[index];
        let height = painted_height(doc, line);
        let y = top + line.y;
        if y + height >= area.top && y <= area.bottom {
            spanning_headers.push(index);
        }
    };
    if let Some(backgrounds) = backgrounds {
        for &index in backgrounds
            .iter()
            .take_while(|&&index| index < first_visible)
        {
            inspect(index);
        }
    } else {
        for index in 0..first_visible {
            inspect(index);
        }
    }
    let mut code_number_widths = HashMap::new();
    for index in spanning_headers
        .into_iter()
        .chain(first_visible..last_visible)
    {
        let line = &doc.lines[index];
        let y = top + line.y;
        let painted_height = painted_height(doc, line);
        if y + painted_height < area.top || y > area.bottom {
            continue;
        }
        let x = origin_x + line.x;
        let mut rect = Rect::new(x, y, area.right - 16.0, y + line.height);

        match line.decoration {
            Decoration::Container {
                details,
                collapsed,
                background,
                border,
                ..
            } => {
                let right = origin_x + content_width(area) - line.x;
                let card = Rect::new(x, y, right, y + painted_height);
                let bg = if details {
                    p.surface
                } else if theme::is_dark(p) {
                    theme::mix(background, p.surface, 0.13)
                } else {
                    background
                };
                let border = if details {
                    p.border
                } else if theme::is_dark(p) {
                    theme::mix(border, p.surface, 0.4)
                } else {
                    border
                };
                list.rounded_rect(card, 7.0, bg);
                list.rounded_border(card, 7.0, border);
                let icon = if details {
                    if collapsed {
                        crate::ui::icons::Icon::CHEVRON_RIGHT
                    } else {
                        crate::ui::icons::Icon::CHEVRON_DOWN
                    }
                } else {
                    crate::ui::icons::Icon::PENCIL_LINE
                };
                list.icon_centered(
                    Rect::new(x + 8.0, y, x + 30.0, y + line.height),
                    icon,
                    15.0,
                    p.muted,
                );
                let title = line.runs.first().map(|r| r.text.as_str()).unwrap_or("");
                list.text(
                    Rect::new(x + 36.0, y, right - 34.0, y + line.height),
                    text::ellipsize(title, TextStyle::Title, (right - x - 70.0).max(0.0)),
                    TextStyle::Title,
                    p.foreground,
                );
                if !printing {
                    list.icon_centered(
                        Rect::new(right - 30.0, y, right - 6.0, y + line.height),
                        crate::ui::icons::Icon::MORE_HORIZONTAL,
                        16.0,
                        p.muted,
                    );
                }
                continue;
            }
            Decoration::AiLocator => {
                let card = Rect::new(x, y, x + content_width(area), y + line.height);
                list.rounded_rect(card, 8.0, p.surface);
                list.rounded_border(card, 8.0, p.border);
                list.rect(
                    Rect::new(
                        card.left,
                        card.top + 7.0,
                        card.left + 3.0,
                        card.bottom - 7.0,
                    ),
                    p.accent,
                );
                let icon = Rect::from_size(
                    card.left + 17.0,
                    (card.top + card.bottom) / 2.0 - 8.0,
                    16.0,
                    16.0,
                );
                list.icon_centered(icon, crate::ui::icons::Icon::CROSSHAIR, 16.0, p.accent);
                let title = line
                    .runs
                    .first()
                    .map(|r| r.text.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("AI 会话");
                let left = card.left + 43.0;
                let right = card.right - 15.0;
                list.push_clip(card);
                let label_bottom =
                    card.top + 11.0 + 13.0 * crate::ui::editor_preferences::current().line_height;
                list.text(
                    Rect::new(left, card.top + 11.0, right, label_bottom),
                    text::ellipsize(
                        &format!("AI 定位 · {}", title.replace(['\r', '\n'], " ")),
                        TextStyle::Title,
                        (right - left).max(0.0),
                    ),
                    TextStyle::Title,
                    p.foreground,
                );
                if let Some(snippet) = line.runs.get(1).filter(|r| !r.text.is_empty()) {
                    list.text(
                        Rect::new(left, label_bottom + 2.0, right, card.bottom - 11.0),
                        text::ellipsize(
                            &snippet.text.replace(['\r', '\n'], " "),
                            TextStyle::Small,
                            (right - left).max(0.0),
                        ),
                        TextStyle::Small,
                        p.muted,
                    );
                }
                list.pop_clip();
                continue;
            }
            Decoration::ObjectReference { text_style } => {
                let width = if text_style {
                    (text::measure(&line.runs[0].text, TextStyle::Document) + 48.0)
                        .min(content_width(area))
                } else {
                    content_width(area)
                };
                let card = Rect::from_size(x, y, width, line.height);
                list.rounded_rect(
                    card,
                    if text_style { 5.0 } else { 9.0 },
                    theme::mix(p.accent, p.surface, 0.08),
                );
                list.rounded_border(
                    card,
                    if text_style { 5.0 } else { 9.0 },
                    theme::mix(p.accent, p.border, 0.35),
                );
                list.icon_centered(
                    Rect::from_size(x + 10.0, y + 9.0, 18.0, 18.0),
                    crate::ui::icons::Icon::LINK2,
                    16.0,
                    p.accent,
                );
                let right = card.right - 12.0;
                let label = &line.runs[0].text;
                list.push_clip(card);
                list.text(
                    Rect::new(
                        x + 36.0,
                        y + 4.0,
                        right,
                        y + 4.0 + TextStyle::Document.line_height(),
                    ),
                    text::ellipsize(label, TextStyle::Document, (right - x - 36.0).max(1.0)),
                    TextStyle::Document,
                    p.accent,
                );
                if !text_style {
                    let detail = format!("{} · {}", line.runs[1].text, line.runs[2].text);
                    let top = y + TextStyle::Document.line_height() + 10.0;
                    list.text(
                        Rect::new(x + 36.0, top, right, top + TextStyle::Small.line_height()),
                        text::ellipsize(&detail, TextStyle::Small, (right - x - 36.0).max(1.0)),
                        TextStyle::Small,
                        p.muted,
                    );
                }
                list.pop_clip();
                continue;
            }
            Decoration::Divider => {
                let w = (area.width() - pad_x * 2.0).min(max_line_width());
                list.hline(origin_x, origin_x + w, y, p.border);
                continue;
            }
            Decoration::QuoteBar => {
                list.rect(
                    Rect::new(origin_x, y, origin_x + quote_bar(), y + line.height),
                    crate::ui::editor_preferences::current()
                        .quote_border_color
                        .unwrap_or(p.border),
                );
            }
            Decoration::HeadingUnderline => {
                let right = origin_x + content_width(area);
                list.hline(x, right, y + line.height - 1.0, p.border);
                rect.bottom -= 9.0;
            }
            Decoration::Math => {
                let tex = line.runs.first().map(|r| r.text.as_str()).unwrap_or("");
                let font_size = line.style.font_size() * text::MATH_SCALE;
                if let Some((width, height)) = crate::ui::math_layout::size(tex, font_size) {
                    let scale = (content_width(area) / width.max(1.0)).min(1.0);
                    let top = y + (line.height - height * scale) / 2.0;
                    list.math(
                        Rect::new(x, top, x + width * scale, top + height * scale),
                        tex,
                        font_size * scale,
                        crate::ui::editor_preferences::current()
                            .document_text_color
                            .unwrap_or(p.foreground),
                    );
                } else {
                    list.text(
                        rect,
                        text::ellipsize(tex, line.style, content_width(area)),
                        line.style,
                        p.danger,
                    );
                }
                continue;
            }
            Decoration::CodeHeader {
                body_lines,
                collapsed,
            } => {
                // 整张卡片：底色、边框、头部底色与分隔线、语言名、复制按钮
                let w = card_width(area);
                let (bg, header_bg, border, _) = code_card_colors(p);
                let header_height = code_header_height();
                let card = Rect::new(
                    origin_x,
                    y,
                    origin_x + w,
                    y + if collapsed {
                        header_height
                    } else {
                        code_card_height(body_lines)
                    },
                );
                list.rounded_rect(card, code_radius(), bg);
                list.rounded_border(card, code_radius(), border);
                if !crate::ui::editor_preferences::current().code_wrap {
                    let block_lines = doc.lines_for_block(line.block);
                    let number_width =
                        cached_code_line_number_width(&mut code_number_widths, doc, line.block);
                    let normal_x = code_pad_x() + number_width;
                    let visible = (card.width() - code_pad_x() * 2.0 - number_width).max(1.0);
                    let full = block_lines
                        .iter()
                        .filter(|candidate| {
                            matches!(candidate.decoration, Decoration::CodeBackground { .. })
                        })
                        .map(|candidate| {
                            candidate
                                .runs
                                .iter()
                                .map(|run| text::measure(&run.text, candidate.style))
                                .sum::<f32>()
                        })
                        .fold(0.0, f32::max);
                    if full > visible {
                        let offset = block_lines
                            .iter()
                            .find(|candidate| {
                                matches!(candidate.decoration, Decoration::CodeBackground { .. })
                            })
                            .map(|candidate| (normal_x - candidate.x).max(0.0))
                            .unwrap_or(0.0)
                            .min(full - visible);
                        let track = Rect::new(
                            card.left + code_pad_x(),
                            card.bottom - 11.0,
                            card.right - code_pad_x(),
                            card.bottom - 4.0,
                        );
                        let thumb_width =
                            (track.width() * visible / full).clamp(24.0, track.width());
                        let thumb = Rect::from_size(
                            track.left + offset / (full - visible) * (track.width() - thumb_width),
                            track.top,
                            thumb_width,
                            track.height(),
                        );
                        list.rounded_rect(track, 3.0, border);
                        list.rounded_rect(thumb, 3.0, p.muted);
                    }
                }
                if !code_show_title() {
                    let lang = line.runs.first().map(|r| r.text.as_str()).unwrap_or("");
                    let label = if lang.is_empty() { "plaintext" } else { lang };
                    list.text_run(
                        Rect::new(
                            card.left + code_pad_x(),
                            card.top + 2.0,
                            card.right - 8.0,
                            card.top + code_pad_y(),
                        ),
                        label.to_owned(),
                        TextStyle::Caption,
                        p.muted,
                        crate::ui::draw::Align::Trailing,
                        Emphasis::Bold,
                    );
                    continue;
                }
                let header = Rect::new(
                    card.left + 1.0,
                    card.top + 1.0,
                    card.right - 1.0,
                    card.top + header_height,
                );
                list.push_clip(header);
                list.rounded_rect(
                    Rect::new(
                        header.left,
                        header.top,
                        header.right,
                        header.bottom + code_radius(),
                    ),
                    code_radius(),
                    header_bg,
                );
                list.pop_clip();
                list.hline(card.left, card.right, header.bottom, border);
                if printing {
                    let lang = line
                        .runs
                        .first()
                        .map(|r| r.text.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("code");
                    let title = line
                        .runs
                        .get(1)
                        .map(|r| r.text.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(lang);
                    list.text_run(
                        Rect::new(
                            header.left + 12.0,
                            header.top,
                            header.right - 12.0,
                            header.bottom,
                        ),
                        text::ellipsize(
                            title,
                            TextStyle::Caption,
                            (header.width() - 24.0).max(0.0),
                        ),
                        TextStyle::Caption,
                        p.muted,
                        crate::ui::draw::Align::Leading,
                        Emphasis::Bold,
                    );
                    continue;
                }
                list.icon_centered(
                    Rect::new(
                        header.left + 10.0,
                        header.top,
                        header.left + 32.0,
                        header.bottom,
                    ),
                    if collapsed {
                        crate::ui::icons::Icon::CHEVRON_RIGHT
                    } else {
                        crate::ui::icons::Icon::CHEVRON_DOWN
                    },
                    14.0,
                    p.muted,
                );
                let lang = line.runs.first().map(|r| r.text.as_str()).unwrap_or("");
                let label = if lang.is_empty() {
                    "plaintext".to_owned()
                } else {
                    lang.to_owned()
                };
                let language_left = (header.right - 114.0).max(header.left + 40.0);
                let copy_left = language_left - 34.0;
                let title = line
                    .runs
                    .get(1)
                    .map(|r| r.text.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("代码块名称");
                list.text(
                    Rect::new(
                        header.left + 40.0,
                        header.top,
                        copy_left - 4.0,
                        header.bottom,
                    ),
                    text::ellipsize(
                        title,
                        TextStyle::Caption,
                        (copy_left - header.left - 44.0).max(0.0),
                    ),
                    TextStyle::Caption,
                    p.muted,
                );
                list.text_run(
                    Rect::new(
                        language_left,
                        header.top,
                        header.right - 24.0,
                        header.bottom,
                    ),
                    text::ellipsize(
                        &label,
                        TextStyle::Caption,
                        (header.right - 24.0 - language_left).max(0.0),
                    ),
                    TextStyle::Caption,
                    p.muted,
                    crate::ui::draw::Align::Leading,
                    Emphasis::Bold,
                );
                list.icon_centered(
                    Rect::new(
                        header.right - 22.0,
                        header.top,
                        header.right - 8.0,
                        header.bottom,
                    ),
                    crate::ui::icons::Icon::CHEVRON_DOWN,
                    13.0,
                    p.muted,
                );
                list.icon_centered(
                    Rect::new(
                        copy_left,
                        header.top + 4.0,
                        copy_left + 28.0,
                        header.bottom - 4.0,
                    ),
                    crate::ui::icons::Icon::COPY,
                    14.0,
                    p.muted,
                );
                continue;
            }
            Decoration::CodeBackground { line_number } => {
                // 卡片已由头部画好；代码行按语法着色逐段画，不着色的段用代码前景色
                let rect_w = (area.width() - pad_x * 2.0).min(max_line_width());
                // 文字可移动，但裁剪区和行号栏始终贴在卡片内边距上，不能随滚动滑出卡片。
                let number_width =
                    cached_code_line_number_width(&mut code_number_widths, doc, line.block);
                let text_left = origin_x + code_pad_x() + number_width;
                let text_rect = Rect::new(
                    text_left,
                    y,
                    origin_x + rect_w - code_pad_x(),
                    y + line.height,
                );
                if let Some(number) = line_number {
                    list.text_run(
                        Rect::new(origin_x + code_pad_x(), y, text_left - 8.0, y + line.height),
                        number.to_string(),
                        TextStyle::DocumentMono,
                        p.muted,
                        crate::ui::draw::Align::Trailing,
                        Emphasis::Code,
                    );
                }
                let raw = line.runs.first().map(|r| r.text.as_str()).unwrap_or("");
                if !raw.is_empty() && !text_rect.is_empty() {
                    list.push_clip(text_rect);
                    let mut cx = x;
                    let mut pos = 0usize;
                    let mut segments: Vec<(usize, usize, Option<highlight::Token>)> = Vec::new();
                    for s in &line.tokens {
                        if s.start > pos {
                            segments.push((pos, s.start, None));
                        }
                        segments.push((s.start, s.end.min(raw.len()), Some(s.kind)));
                        pos = s.end.min(raw.len());
                    }
                    if pos < raw.len() {
                        segments.push((pos, raw.len(), None));
                    }
                    let (_, _, _, code_fg) = code_card_colors(p);
                    let dark = theme::luminance(code_card_colors(p).0) < 0.5;
                    for (a, b, kind) in segments {
                        let Some(piece) = raw.get(a..b) else { continue };
                        if piece.is_empty() {
                            continue;
                        }
                        let w = text::measure(piece, TextStyle::DocumentMono);
                        let color = kind.map(|k| highlight::color(k, dark)).unwrap_or(code_fg);
                        let emphasis = if kind.map(highlight::italic).unwrap_or(false) {
                            Emphasis::Italic
                        } else {
                            Emphasis::Code
                        };
                        list.text_run(
                            Rect::new(cx, y, text_rect.right, y + line.height),
                            piece.to_owned(),
                            TextStyle::DocumentMono,
                            color,
                            crate::ui::draw::Align::Leading,
                            emphasis,
                        );
                        cx += w;
                        if cx >= text_rect.right {
                            break;
                        }
                    }
                    list.pop_clip();
                }
                continue;
            }
            Decoration::TableRow { cols, header } => {
                list.push_clip(Rect::new(
                    origin_x,
                    y,
                    origin_x + content_width(area),
                    y + line.height,
                ));
                let origin_x = x;
                let available = content_width(area);
                let col_w = table_column_width(cols, available);
                let custom = doc
                    .table_widths
                    .get(&line.block)
                    .filter(|widths| widths.len() == cols);
                let prefs = crate::ui::editor_preferences::current();
                let border = prefs.table_border;
                let border_color = prefs.table_border_color.unwrap_or(p.border);
                let header_background = prefs
                    .table_header_background_color
                    .unwrap_or(p.surface_muted);
                let table_text_color = prefs.table_text_color.unwrap_or(p.foreground);
                let padding = prefs.table_padding;
                let mut cell_left = origin_x;
                for (ci, cell) in line.runs.iter().enumerate() {
                    let width = custom
                        .and_then(|widths| widths.get(ci))
                        .copied()
                        .unwrap_or(col_w);
                    let cell_rect = Rect::new(cell_left, y, cell_left + width, y + line.height);
                    cell_left += width;
                    if header {
                        list.rect(cell_rect, header_background);
                    }
                    // border-collapse：每格画上边与左边，最右/最下由最后一格/最后一行补
                    list.rect(
                        Rect::new(
                            cell_rect.left,
                            cell_rect.top,
                            cell_rect.right,
                            cell_rect.top + border,
                        ),
                        border_color,
                    );
                    list.rect(
                        Rect::new(
                            cell_rect.left,
                            cell_rect.top,
                            cell_rect.left + border,
                            cell_rect.bottom,
                        ),
                        border_color,
                    );
                    if ci + 1 == cols {
                        list.rect(
                            Rect::new(
                                cell_rect.right - border,
                                cell_rect.top,
                                cell_rect.right,
                                cell_rect.bottom,
                            ),
                            border_color,
                        );
                    }
                    list.rect(
                        Rect::new(
                            cell_rect.left,
                            cell_rect.bottom - border,
                            cell_rect.right,
                            cell_rect.bottom,
                        ),
                        border_color,
                    );
                    let text_rect = Rect::new(
                        cell_rect.left + padding + 4.0,
                        cell_rect.top + padding,
                        cell_rect.right - padding - 4.0,
                        cell_rect.top + padding + TextStyle::Table.line_height(),
                    );
                    if !cell.text.is_empty() && !text_rect.is_empty() {
                        list.push_clip(text_rect);
                        // 单元格里的行内标记照常解析；表头整体半粗
                        let mut cx = text_rect.left;
                        for run in text::parse_inline(&cell.text) {
                            if run.text.is_empty() {
                                continue;
                            }
                            let w =
                                text::measure_runs(std::slice::from_ref(&run), TextStyle::Table);
                            let emphasis = if header && run.emphasis == Emphasis::None {
                                Emphasis::Bold
                            } else {
                                run.emphasis
                            };
                            let color = if run.emphasis.base() == Emphasis::Link {
                                p.accent
                            } else {
                                table_text_color
                            };
                            list.text_run(
                                Rect::new(cx, text_rect.top, text_rect.right, text_rect.bottom),
                                run.text.clone(),
                                TextStyle::Table,
                                color,
                                crate::ui::draw::Align::Leading,
                                emphasis,
                            );
                            cx += w;
                            if cx >= text_rect.right {
                                break;
                            }
                        }
                        list.pop_clip();
                    }
                }
                list.pop_clip();
                continue;
            }
            Decoration::Image => {
                let w =
                    ((area.width() - pad_x * 2.0).min(max_line_width()) - line.x * 2.0).max(1.0);
                let frame = Rect::new(x, y, x + w, y + line.height);
                let src = line.runs.get(1).map(|r| r.text.as_str()).unwrap_or("");
                let alt = line
                    .runs
                    .first()
                    .map(|r| r.text.clone())
                    .unwrap_or_default();
                let width = line
                    .runs
                    .get(2)
                    .and_then(|r| r.text.parse::<f32>().ok())
                    .unwrap_or(frame.width());
                let frame = Rect::new(
                    frame.left,
                    frame.top,
                    frame.left + width.min(frame.width()),
                    frame.bottom,
                );
                list.image(frame, resolve_image_src(src, base_dir), alt);
                continue;
            }
            Decoration::None => {}
        }

        let preferences = crate::ui::editor_preferences::current();
        let is_heading = matches!(
            line.style,
            TextStyle::Heading1
                | TextStyle::Heading2
                | TextStyle::Heading3
                | TextStyle::Heading4
                | TextStyle::Heading5
                | TextStyle::Heading6
        );
        let color = if line.decoration == Decoration::QuoteBar {
            preferences.quote_text_color.unwrap_or(p.muted)
        } else if is_heading {
            preferences.heading_text_color.unwrap_or(p.foreground)
        } else {
            preferences.document_text_color.unwrap_or(p.foreground)
        };
        // 任务勾选框：画成真正的方框（TipTap 的 TaskItem 是 `<input type=checkbox>`），
        // 未勾选是灰边框，勾选是 accent 底 + 白勾
        if line.visible_start == usize::MAX
            && line.runs.len() == 1
            && matches!(line.runs[0].text.as_str(), "☐" | "☑")
        {
            let checked = line.runs[0].text == "☑";
            let size = CHECKBOX_SIZE;
            let box_rect = Rect::new(
                x,
                y + (line.height - size) / 2.0,
                x + size,
                y + (line.height + size) / 2.0,
            );
            if checked {
                list.rounded_rect(box_rect, 4.0, p.accent);
                list.icon_centered(
                    box_rect,
                    crate::ui::icons::Icon::CHECK,
                    12.0,
                    p.accent_foreground,
                );
            } else {
                list.rounded_border(box_rect, 4.0, p.muted);
            }
            continue;
        }
        // 逐 run 画，x 按已画部分的宽度推进——同一行里粗体和正文要接得上
        let mut run_x = rect.left;
        for run in &line.runs {
            if run.text.is_empty() {
                continue;
            }
            let w = text::measure_runs(std::slice::from_ref(run), line.style);
            // 链接：`text-accent underline`
            let run_color = if run.emphasis.base() == Emphasis::Link {
                p.accent
            } else if line.visible_start == usize::MAX {
                preferences.list_marker_color.unwrap_or(color)
            } else {
                color
            };
            list.text_run(
                Rect::new(run_x, rect.top, (run_x + w).min(rect.right), rect.bottom),
                run.text.clone(),
                line.style,
                run_color,
                crate::ui::draw::Align::Leading,
                run.emphasis,
            );
            if run.emphasis.base() == Emphasis::Link {
                let baseline = rect.top + (line.height + line.style.font_size()) / 2.0 + 2.0;
                list.hline(run_x, (run_x + w).min(rect.right), baseline, p.accent);
            }
            run_x += w;
            if run_x >= rect.right {
                break;
            }
        }
    }

    list.pop_clip();
}

/// 块级卡片（代码块、表格、图片）的宽度：内容宽，且不超过限宽。
pub fn card_width(area: Rect) -> f32 {
    content_width(area).min(max_line_width())
}

/// 给一个未换行的代码块计算可横向滚动的最大距离。
pub fn code_horizontal_overflow(layout: &Layout, block: usize, width: f32) -> f32 {
    if crate::ui::editor_preferences::current().code_wrap {
        return 0.0;
    }
    let lines = layout
        .lines_for_block(block)
        .iter()
        .filter(|line| {
            line.block == block && matches!(line.decoration, Decoration::CodeBackground { .. })
        })
        .collect::<Vec<_>>();
    let number_of_lines = lines
        .iter()
        .filter_map(|line| match line.decoration {
            Decoration::CodeBackground { line_number } => line_number,
            _ => None,
        })
        .max()
        .unwrap_or(1);
    let visible = (width - code_pad_x() * 2.0 - code_line_number_width(number_of_lines)).max(1.0);
    let full = lines
        .iter()
        .map(|line| {
            line.runs
                .iter()
                .map(|run| text::measure(&run.text, line.style))
                .sum::<f32>()
        })
        .fold(0.0, f32::max);
    (full - visible).max(0.0)
}

/// 内容区可用宽度（扣掉左右留白）。
pub fn content_width(area: Rect) -> f32 {
    let l = crate::ui::editor_preferences::current();
    (area.width() - l.padding_left - l.padding_right)
        .max(40.0)
        .min(max_line_width())
}

/// 滚动上界。滚过头会露出一片空白，夹住它。
#[cfg(test)]
pub fn max_scroll(doc: &Layout, area: Rect) -> f32 {
    (doc.height - area.height()).max(0.0)
}
