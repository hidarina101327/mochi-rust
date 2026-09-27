//! 源码范围使用字节偏移，与 pulldown-cmark 一致；编辑区外保留原始 UTF-8 和 CRLF。

use anyhow::{bail, Result};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::model::SourceSpan;

/// 唯一会被识别为持久化块标记的 HTML 注释格式。
pub const BLOCK_MARKER_PREFIX: &str = "<!-- mochi:block ";
pub const BLOCK_MARKER_SUFFIX: &str = " -->";

/// 在围栏代码、行内代码和缩进代码区域之外找到的标记。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// `mochi:block` 与结尾 `-->` 之间的内容。
    pub token: String,
    /// 整条 HTML 注释的范围。
    pub span: SourceSpan,
    /// 注释所在的完整源码行，包括换行符。编辑块时若要移动独占一行的标记，会用到它。
    pub line_span: SourceSpan,
    pub raw: String,
}

/// 在代码和注释之外找到的原始 `[[...]]` 引用。转换为 `BlockId` 的校验由 `model` 负责。
/// 这里保留字符串形式，可避免语法扫描器与模型模块循环依赖，也让模型能明确报告格式错误的引用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawReference {
    pub token: String,
    pub span: SourceSpan,
    pub raw: String,
}

/// Markdown 解析器在分配 ID 前输出的结构类型。列表每一层都会生成独立项，
/// 因此外层项和子项的范围可能重叠。编辑嵌套项时，调用方必须使用子项 ID，
/// 并检查内容是否已过期。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateKind {
    Heading { level: u8 },
    Paragraph,
    ListItem { checked: Option<bool> },
    FencedCode { language: Option<String> },
    IndentedCode,
    Table,
    Formula { display: bool },
    Html,
    ThematicBreak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub kind: CandidateKind,
    pub span: SourceSpan,
}

/// 解析文档中可单独定位的块。
///
/// 块边界由 `pulldown-cmark` 判定，覆盖表格、围栏代码、任务标记和展示公式。
/// 列表项内的段落不会单独生成块，因为可编辑单位是列表项本身。
/// 嵌套列表项仍保留为候选块，确保每一项都有稳定记录。
pub fn parse_candidates(source: &str) -> Result<Vec<Candidate>> {
    let mut options = Options::empty();
    options.insert(
        Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_MATH
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_GFM,
    );

    let mut open: Vec<OpenCandidate> = Vec::new();
    let mut out = Vec::new();
    let parser = Parser::new_ext(source, options);

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(tag) => match tag {
                Tag::Item => open.push(OpenCandidate {
                    kind: CandidateKind::ListItem { checked: None },
                    start: range.start,
                }),
                Tag::Paragraph if !inside_item(&open) => open.push(OpenCandidate {
                    kind: CandidateKind::Paragraph,
                    start: range.start,
                }),
                Tag::Heading { level, .. } if !inside_item(&open) => open.push(OpenCandidate {
                    kind: CandidateKind::Heading {
                        level: heading_level(level),
                    },
                    start: range.start,
                }),
                Tag::CodeBlock(kind) => {
                    let kind = match kind {
                        CodeBlockKind::Fenced(language) => CandidateKind::FencedCode {
                            language: (!language.is_empty()).then(|| language.to_string()),
                        },
                        CodeBlockKind::Indented => CandidateKind::IndentedCode,
                    };
                    open.push(OpenCandidate {
                        kind,
                        start: range.start,
                    });
                }
                Tag::Table(_) => open.push(OpenCandidate {
                    kind: CandidateKind::Table,
                    start: range.start,
                }),
                Tag::HtmlBlock => open.push(OpenCandidate {
                    kind: CandidateKind::Html,
                    start: range.start,
                }),
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Item => {
                    finish_candidate(&mut open, &mut out, CandidateEnd::Item, range.end)
                }
                TagEnd::Paragraph => {
                    finish_candidate(&mut open, &mut out, CandidateEnd::Paragraph, range.end)
                }
                TagEnd::Heading(_) => {
                    finish_candidate(&mut open, &mut out, CandidateEnd::Heading, range.end)
                }
                TagEnd::CodeBlock => {
                    finish_candidate(&mut open, &mut out, CandidateEnd::Code, range.end)
                }
                TagEnd::Table => {
                    finish_candidate(&mut open, &mut out, CandidateEnd::Table, range.end)
                }
                TagEnd::HtmlBlock => {
                    finish_candidate(&mut open, &mut out, CandidateEnd::Html, range.end)
                }
                _ => {}
            },
            Event::TaskListMarker(checked) => {
                if let Some(item) = open
                    .iter_mut()
                    .rev()
                    .find(|candidate| matches!(candidate.kind, CandidateKind::ListItem { .. }))
                {
                    item.kind = CandidateKind::ListItem {
                        checked: Some(checked),
                    };
                }
            }
            Event::DisplayMath(_) => out.push(Candidate {
                kind: CandidateKind::Formula { display: true },
                span: SourceSpan::new(range.start, range.end),
            }),
            Event::InlineMath(_) if !inside_item(&open) => out.push(Candidate {
                kind: CandidateKind::Formula { display: false },
                span: SourceSpan::new(range.start, range.end),
            }),
            Event::Rule => out.push(Candidate {
                kind: CandidateKind::ThematicBreak,
                span: SourceSpan::new(range.start, range.end),
            }),
            _ => {}
        }
    }

    if !open.is_empty() {
        // pulldown-cmark 保证事件成对出现。这里再作防护，避免后续解析器改动返回
        // 不完整的范围，进而造成不安全的替换。
        bail!("markdown parser left {} block(s) open", open.len());
    }

    out.sort_by(|left, right| {
        left.span
            .start
            .cmp(&right.span.start)
            // 父项和子项可能从同一个字节开始。父项代表外层记录，排在前面以保证索引顺序稳定。
            .then_with(|| right.span.end.cmp(&left.span.end))
            .then_with(|| format!("{:?}", left.kind).cmp(&format!("{:?}", right.kind)))
    });

    // 正文中的行内公式属于行内语法，不是另一个可单独编辑的块。
    // 如果整段只有公式，就用 Formula 表示，不再生成重复的 Paragraph。
    // pulldown-cmark 对某些公式给出的范围不含一侧或两侧分隔符，因此独立公式
    // 使用所在段落的完整源码范围，包括原有换行符。
    let mut filtered = Vec::with_capacity(out.len());
    for candidate in out.iter().cloned() {
        if let CandidateKind::Formula { .. } = candidate.kind {
            if let Some(paragraph) = standalone_formula_paragraph(source, &candidate, &out) {
                let mut formula = candidate;
                formula.span = paragraph.span;
                filtered.push(formula);
            }
            // 正文中的行内公式归所在段落所有。若保留范围重叠的公式候选块，
            // 替换时可能把段落截断，或从公式处拆开 MC 容器。
            // 独立公式已在上面按所在段落的范围加入；其他行内公式不单独成块。
        } else if !matches!(candidate.kind, CandidateKind::Paragraph)
            || !out.iter().any(|formula| {
                matches!(formula.kind, CandidateKind::Formula { .. })
                    && standalone_formula_paragraph(source, formula, &out)
                        .is_some_and(|paragraph| paragraph.span == candidate.span)
            })
        {
            filtered.push(candidate);
        }
    }
    filtered.sort_by(|left, right| {
        left.span
            .start
            .cmp(&right.span.start)
            .then_with(|| right.span.end.cmp(&left.span.end))
    });
    // 独占一行的持久化标记在词法上是 HTML 注释。CommonMark 可能将它输出为 HtmlBlock，
    // 尤其是在分隔线或段落之前。它属于元数据，不是用户块；标记归属交由模型处理，
    // 避免生成一个实际不存在的块。
    filtered.retain(|candidate| {
        !(matches!(candidate.kind, CandidateKind::Html)
            && is_marker_only_comment(source, candidate.span))
    });

    // YAML 头部是文档元数据，不是可编辑的正文块。原生编辑器把光标放在其后，
    // 块模型也必须在分配 ID 前明确划出同一边界。
    if let Some(frontmatter) = frontmatter_span(source) {
        filtered.retain(|candidate| !frontmatter.contains(candidate.span));
    }

    // 墨池的富内容容器以源码语法持久化，CommonMark 不会把它们解析为整体，
    // `:::mochi-highlight` 尤其如此。每个完整容器合并为一个 Html 候选块，
    // 丢弃解析器给出的内部段落和表格。嵌套容器仍保留为嵌套候选块，
    // 方便调用方定位外层面板或指定子容器，同时不暴露内部实现片段。
    let containers = container_ranges(source);
    if !containers.is_empty() {
        filtered.retain(|candidate| {
            !containers
                .iter()
                .any(|container| container.contains(candidate.span))
        });
        filtered.extend(containers.into_iter().map(|span| Candidate {
            kind: CandidateKind::Html,
            span,
        }));
    }

    // Electron 版代码卡片的元数据属于代码块源码的一部分，不是单独的 HTML 块。
    // 将元数据行纳入围栏代码候选块后，持久化标记可以放在元数据之前；
    // 元数据仍紧挨代码围栏，供原生代码卡片读取器识别。
    coalesce_code_metadata(source, &mut filtered);

    filtered.sort_by(|left, right| {
        left.span
            .start
            .cmp(&right.span.start)
            .then_with(|| right.span.end.cmp(&left.span.end))
    });
    Ok(filtered)
}

fn coalesce_code_metadata(source: &str, candidates: &mut Vec<Candidate>) {
    let mut metadata = Vec::new();
    for candidate in candidates.iter_mut() {
        if !matches!(candidate.kind, CandidateKind::FencedCode { .. }) {
            continue;
        }
        let Some(prefix) = code_metadata_span(source, candidate.span.start) else {
            continue;
        };
        candidate.span.start = prefix.start;
        metadata.push(prefix);
    }
    if metadata.is_empty() {
        return;
    }
    candidates.retain(|candidate| {
        !metadata.iter().any(|prefix| {
            prefix.contains(candidate.span)
                && !matches!(candidate.kind, CandidateKind::FencedCode { .. })
        })
    });
}

fn code_metadata_span(source: &str, code_start: usize) -> Option<SourceSpan> {
    let before = source.get(..code_start)?.trim_end_matches(['\r', '\n']);
    let line_start = before.rfind('\n').map_or(0, |at| at + 1);
    let line = before.get(line_start..)?.trim();
    if line.starts_with("<!-- mochi-code-block ") && line.ends_with("-->") {
        Some(SourceSpan::new(line_start, code_start))
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateEnd {
    Item,
    Paragraph,
    Heading,
    Code,
    Table,
    Html,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenCandidate {
    kind: CandidateKind,
    start: usize,
}

fn finish_candidate(
    open: &mut Vec<OpenCandidate>,
    out: &mut Vec<Candidate>,
    expected: CandidateEnd,
    end: usize,
) {
    let matches = open.last().is_some_and(|candidate| match expected {
        CandidateEnd::Item => matches!(candidate.kind, CandidateKind::ListItem { .. }),
        CandidateEnd::Paragraph => matches!(candidate.kind, CandidateKind::Paragraph),
        CandidateEnd::Heading => matches!(candidate.kind, CandidateKind::Heading { .. }),
        CandidateEnd::Code => matches!(
            candidate.kind,
            CandidateKind::FencedCode { .. } | CandidateKind::IndentedCode
        ),
        CandidateEnd::Table => matches!(candidate.kind, CandidateKind::Table),
        CandidateEnd::Html => matches!(candidate.kind, CandidateKind::Html),
    });
    if matches {
        let candidate = open.pop().expect("candidate was just checked");
        out.push(Candidate {
            kind: candidate.kind,
            span: SourceSpan::new(candidate.start, end),
        });
    }
}

fn inside_item(open: &[OpenCandidate]) -> bool {
    open.iter()
        .any(|candidate| matches!(candidate.kind, CandidateKind::ListItem { .. }))
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn standalone_formula_paragraph<'a>(
    source: &str,
    formula: &Candidate,
    all: &'a [Candidate],
) -> Option<&'a Candidate> {
    all.iter().find(|paragraph| {
        if !matches!(paragraph.kind, CandidateKind::Paragraph)
            || paragraph.span.start > formula.span.start
            || paragraph.span.end < formula.span.end
        {
            return false;
        }
        let text = source[paragraph.span.start..paragraph.span.end].trim();
        if formula.span.start != paragraph.span.start {
            return false;
        }
        match &formula.kind {
            CandidateKind::Formula { display: true } => {
                text.starts_with("$$") && text.ends_with("$$") && text.len() >= 4
            }
            CandidateKind::Formula { display: false } => {
                text.starts_with('$')
                    && !text.starts_with("$$")
                    && text.ends_with('$')
                    && text.len() >= 2
            }
            _ => false,
        }
    })
}

fn is_marker_only_comment(source: &str, span: SourceSpan) -> bool {
    let Some(text) = source.get(span.start..span.end).map(str::trim) else {
        return false;
    };
    let Some(body) = text.strip_prefix("<!--") else {
        return false;
    };
    let Some(body) = body.strip_suffix("-->") else {
        return false;
    };
    is_marker_body(body)
}

/// 如果源码以闭合的 YAML 围栏开头，返回完整的头部范围。未闭合的围栏按普通 Markdown
/// 处理，与编辑器的恢复行为一致。
fn frontmatter_span(source: &str) -> Option<SourceSpan> {
    let lines = source_lines(source);
    let (_, _, first) = lines.first().copied()?;
    if first.trim() != "---" {
        return None;
    }
    lines
        .iter()
        .skip(1)
        .find(|(_, _, line)| line.trim() == "---")
        .map(|(_, end, _)| SourceSpan::new(0, *end))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerKind {
    Highlight,
    Details,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenContainer {
    kind: ContainerKind,
    start: usize,
}

/// 扫描原生编辑器输出的完整容器语法，按行识别。跳过代码围栏，确保示例保持原样。
/// 不完整的容器不会纳入结果；格式有误的编辑交由常规 Markdown 解析流程处理，
/// 避免悄悄把后续无关内容归入容器。
fn container_ranges(source: &str) -> Vec<SourceSpan> {
    let lines = source_lines(source);
    let mut stack = Vec::new();
    let mut closed = Vec::new();
    let mut fence = None;

    for (start, end, line) in lines {
        if let Some((marker, count)) = fence {
            if fence_close(line, marker, count) {
                fence = None;
            }
            continue;
        }
        if let Some((marker, count)) = fence_open(line) {
            fence = Some((marker, count));
            continue;
        }

        if let Some(kind) = container_open(line) {
            stack.push(OpenContainer { kind, start });
            continue;
        }

        let Some(kind) = container_close(line) else {
            continue;
        };
        if stack.last().is_some_and(|open| open.kind == kind) {
            let open = stack.pop().expect("container stack was just checked");
            closed.push(SourceSpan::new(open.start, end));
        }
    }

    // 即使子容器语法上已闭合，只要外层面板尚未结束，它仍属于外层面板，不能单独暴露。
    closed.retain(|span| {
        !stack
            .iter()
            .any(|open| open.start <= span.start && span.start < source.len())
    });
    closed.sort_by_key(|span| (span.start, std::cmp::Reverse(span.end)));
    closed
}

fn container_open(line: &str) -> Option<ContainerKind> {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix(":::mochi-highlight") {
        if rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace) {
            return Some(ContainerKind::Highlight);
        }
    }
    if line == "<details>" || (line.starts_with("<details ") && line.ends_with('>')) {
        return Some(ContainerKind::Details);
    }
    None
}

fn container_close(line: &str) -> Option<ContainerKind> {
    match line.trim() {
        ":::" => Some(ContainerKind::Highlight),
        "</details>" => Some(ContainerKind::Details),
        _ => None,
    }
}

fn fence_open(line: &str) -> Option<(char, usize)> {
    let line = line.trim();
    let marker = line.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let count = line
        .chars()
        .take_while(|character| *character == marker)
        .count();
    if count < 3 || (marker == '`' && line[count..].contains('`')) {
        return None;
    }
    Some((marker, count))
}

fn fence_close(line: &str, marker: char, count: usize) -> bool {
    let line = line.trim();
    let n = line
        .chars()
        .take_while(|character| *character == marker)
        .count();
    n >= count && line[n..].trim().is_empty()
}

/// 返回每一行的 `(起始位置, 含换行符的结束位置, 不含换行符的内容)`。
/// 字节偏移对中文等多字节字符及 CRLF 换行同样有效。
fn source_lines(source: &str) -> Vec<(usize, usize, &str)> {
    let mut lines = Vec::new();
    let mut offset = 0;
    for raw in source.split_inclusive('\n') {
        let start = offset;
        offset += raw.len();
        let content = raw.strip_suffix('\n').unwrap_or(raw);
        let content = content.strip_suffix('\r').unwrap_or(content);
        lines.push((start, offset, content));
    }
    if source.is_empty() {
        lines.push((0, 0, ""));
    }
    lines
}

/// 返回围栏代码块、缩进代码块和行内代码的源码范围。扫描时会遮蔽其中的注释和引用，
/// 避免把代码示例误认成持久化 ID 或链接。
pub fn code_ranges(source: &str) -> Vec<SourceSpan> {
    let mut options = Options::empty();
    options.insert(
        Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_MATH
            | Options::ENABLE_GFM,
    );
    let mut ranges = Vec::new();
    let mut code_block_start = None;
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code_block_start = Some(range.start),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = code_block_start.take() {
                    ranges.push(SourceSpan::new(start, range.end));
                }
            }
            Event::Code(_) => ranges.push(SourceSpan::new(range.start, range.end)),
            _ => {}
        }
    }
    if let Some(start) = code_block_start {
        ranges.push(SourceSpan::new(start, source.len()));
    }
    ranges
}

/// 在代码区域之外查找形似块标记的注释。只要注释以 `mochi:block` 开头，
/// 即使 ID 无效也会返回，由 `model` 给出用户可见的校验错误，
/// 而不是悄悄将其当作普通注释。
pub fn scan_markers(source: &str) -> Result<Vec<Marker>> {
    let mut masked_ranges = code_ranges(source);
    if let Some(frontmatter) = frontmatter_span(source) {
        masked_ranges.push(frontmatter);
    }
    let masked = mask_ranges(source, masked_ranges);
    let mut markers = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = masked[cursor..].find("<!--") {
        let start = cursor + relative;
        let Some(close_relative) = masked[start + 4..].find("-->") else {
            let body = &masked[start + 4..];
            if body.trim_start().starts_with("mochi:block") {
                bail!("unterminated mochi:block marker at byte {start}");
            }
            break;
        };
        let end = start + 4 + close_relative + 3;
        let raw = &source[start..end];
        let body = &source[start + 4..end - 3];
        if is_marker_body(body) {
            let token = body
                .trim()
                .strip_prefix("mochi:block")
                .unwrap_or_default()
                .trim()
                .to_owned();
            markers.push(Marker {
                token,
                span: SourceSpan::new(start, end),
                line_span: line_span(source, start),
                raw: raw.to_owned(),
            });
        }
        cursor = end;
    }
    Ok(markers)
}

fn is_marker_body(body: &str) -> bool {
    let trimmed = body.trim_start();
    trimmed
        .strip_prefix("mochi:block")
        .is_some_and(|rest| rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace))
}

/// 扫描 `[[...]]` 引用，同时遮蔽代码和所有 HTML 注释。
/// 以 `block_` 开头的内容都会交给 `model` 校验，即使其中的 UUID 格式有误。
pub fn scan_references(source: &str) -> Result<Vec<RawReference>> {
    let mut ranges = code_ranges(source);
    ranges.extend(comment_ranges(source));
    if let Some(frontmatter) = frontmatter_span(source) {
        ranges.push(frontmatter);
    }
    let masked = mask_ranges(source, ranges);
    let mut references = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = masked[cursor..].find("[[") {
        let start = cursor + relative;
        let Some(close_relative) = masked[start + 2..].find("]]") else {
            let body = &masked[start + 2..];
            if body.trim_start().starts_with("block_") {
                bail!("unterminated block reference at byte {start}");
            }
            break;
        };
        let end = start + 2 + close_relative + 2;
        let token = source[start + 2..end - 2].to_owned();
        if token.starts_with("block_") {
            references.push(RawReference {
                token,
                span: SourceSpan::new(start, end),
                raw: source[start..end].to_owned(),
            });
        }
        cursor = end;
    }
    Ok(references)
}

fn comment_ranges(source: &str) -> Vec<SourceSpan> {
    let masked_code = mask_ranges(source, code_ranges(source));
    let mut ranges = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = masked_code[cursor..].find("<!--") {
        let start = cursor + relative;
        let Some(close_relative) = masked_code[start + 4..].find("-->") else {
            ranges.push(SourceSpan::new(start, source.len()));
            break;
        };
        let end = start + 4 + close_relative + 3;
        ranges.push(SourceSpan::new(start, end));
        cursor = end;
    }
    ranges
}

/// 遮蔽指定的字节范围，但保留 CR/LF。其余字节全部替换为 ASCII 空格，
/// 既保留原偏移，也保证结果仍是有效的 UTF-8；标记和代码分隔符本身都是 ASCII。
pub fn mask_ranges(source: &str, ranges: Vec<SourceSpan>) -> String {
    if ranges.is_empty() {
        return source.to_owned();
    }
    let mut bytes = source.as_bytes().to_vec();
    for range in ranges {
        let start = range.start.min(bytes.len());
        let end = range.end.min(bytes.len());
        for byte in &mut bytes[start..end] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(bytes)
        .expect("masking preserves UTF-8 by replacing complete ASCII/code bytes")
}

/// 返回包含 `offset` 的源码行，保留原有的 CRLF、LF 或 CR 换行符。
pub fn line_span(source: &str, offset: usize) -> SourceSpan {
    let mut offset = offset.min(source.len());
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let bytes = source.as_bytes();
    let mut start = offset;
    while start > 0 {
        let byte = bytes[start - 1];
        if byte == b'\n' || byte == b'\r' {
            break;
        }
        start -= 1;
    }
    let mut end = offset;
    while end < bytes.len() && bytes[end] != b'\n' && bytes[end] != b'\r' {
        end += 1;
    }
    if end < bytes.len() {
        if bytes[end] == b'\r' && bytes.get(end + 1) == Some(&b'\n') {
            end += 2;
        } else {
            end += 1;
        }
    }
    SourceSpan::new(start, end)
}

/// 返回一行的起始位置，并保留源码原有的换行方式。
pub fn line_start(source: &str, offset: usize) -> usize {
    line_span(source, offset).start
}

/// 根据文档中的第一个换行推断换行方式。新文档使用 LF；已有文档若使用 CRLF 或 CR，
/// 新插入的标记行也沿用原格式。
pub fn line_ending(source: &str) -> &'static str {
    let bytes = source.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\r' {
            return if bytes.get(index + 1) == Some(&b'\n') {
                "\r\n"
            } else {
                "\r"
            };
        }
        if *byte == b'\n' {
            return "\n";
        }
    }
    "\n"
}
