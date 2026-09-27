//! 布局层完成断行，DirectWrite 禁止二次换行，否则块高会与绘制不一致。
//! 无测量宿主或成形失败时才使用有界字宽估算。

use super::draw::TextStyle;

/// 行内标记。Markdown 的 `**粗**` / `*斜*` / `` `码` ``。
///
/// 嵌套标记在保留叶子源码区间的同时叠加样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Emphasis {
    None,
    Bold,
    BoldItalic,
    Italic,
    Code,
    /// 超链接的显示文字（`[文字](url)`、`[[维基链接]]`、裸 URL）。目标不在 run 里——
    /// 点击时按源码偏移回查 [`link_at`]，这样断行把一条链接拆成两个 run 也不会对不上。
    Link,
    /// 公式原子：Run.text 保留 TeX 原文，宽度和墨迹由原生排版给出。
    Math,
    Styled {
        fg: Option<u32>,
        bg: Option<u32>,
        flags: u8,
    },
}
impl Emphasis {
    pub fn base(self) -> Self {
        match self {
            Self::Styled { flags, .. } => {
                if flags & 64 != 0 {
                    Self::Math
                } else if flags & 16 != 0 {
                    Self::Code
                } else if flags & 3 == 3 {
                    Self::BoldItalic
                } else if flags & 1 != 0 {
                    Self::Bold
                } else if flags & 2 != 0 {
                    Self::Italic
                } else if flags & 32 != 0 {
                    Self::Link
                } else {
                    Self::None
                }
            }
            _ => self,
        }
    }
}

/// 带源码范围的 run：`start..end` 是这段在行里的字节范围（含被吃掉的标记符）。
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedRun {
    pub run: Run,
    pub start: usize,
    pub end: usize,
}

/// `$…$` 行内公式的边界规则照 TipTap 的输入规则 `/\$(?![ \t$])([^\n$]*?[^ \t\n$])\$/`：
/// 开头 `$` 后不能是空白或 `$`，结尾 `$` 前不能是空白。返回结束字节偏移（含闭合 `$`）。
fn match_math_at(line: &str, i: usize) -> Option<usize> {
    let rest = line.get(i..)?;
    let inner = rest.strip_prefix('$')?;
    let first = inner.chars().next()?;
    if first == ' ' || first == '\t' || first == '$' {
        return None;
    }
    let close = inner.find('$')?;
    let body = &inner[..close];
    if body.is_empty() || body.ends_with(' ') || body.ends_with('\t') || body.contains('\n') {
        return None;
    }
    Some(i + 1 + close + 1)
}

/// 源码里的一条链接：`(显示文字的字节范围, 整段的字节范围, 目标)`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSpan {
    pub start: usize,
    pub end: usize,
    pub target: String,
}

/// 行内链接使用的收尾分隔符索引。
///
/// 解析一行时每个 `[` 都会调用 `match_link_at`。若对每个候选都用
/// `str::find` 找最近的 `](`，遇到 `[[[[[[...` 这种坏行就会退化成
/// 二次方：每个候选都把同一段后缀重新扫一遍。这里在每次解析时
/// 一次性记下所有分隔符位置，再二分查找。
/// 索引刻意按字节计，与本模块其余部分使用的源码区间一致；
/// 所有分隔符都是 ASCII。
#[derive(Debug, Default)]
struct LinkClosings {
    wiki: Vec<usize>,
    markdown: Vec<usize>,
    parens: Vec<usize>,
}

impl LinkClosings {
    fn new(line: &str) -> Self {
        let bytes = line.as_bytes();
        let mut index = Self::default();
        for i in 0..bytes.len() {
            match bytes[i] {
                b']' if i + 1 < bytes.len() && bytes[i + 1] == b']' => {
                    index.wiki.push(i);
                }
                b']' if i + 1 < bytes.len() && bytes[i + 1] == b'(' => {
                    index.markdown.push(i);
                }
                b')' => index.parens.push(i),
                _ => {}
            }
        }
        index
    }

    fn first_at_or_after(values: &[usize], at: usize) -> Option<usize> {
        let index = values.partition_point(|value| *value < at);
        values.get(index).copied()
    }
}

#[derive(Debug, Clone, Copy)]
struct BacktickRun {
    start: usize,
    end: usize,
}

/// 一个源码切片内所有行内代码分隔符候选的统一索引。
///
/// 旧的匹配器对每个反引号都搜索完整后缀。除了未闭合 run 的二次方
/// 开销，它还在起始 run 内部的每个位置悄悄重复这一过程：五个反引号
/// 的 run 从第二个字符开始匹配时，仍可能与后面四个反引号的 run 闭合。
/// 这里保留完整的 run 和一棵区间最大值树，每个候选只需查询
/// 「可用长度不小于自身剩余定界符数量的下一个 run」。
/// 树里只存 run 长度，不复制任何源码文本。
#[derive(Debug, Default)]
pub(super) struct CodeClosings {
    runs: Vec<BacktickRun>,
    tree_base: usize,
    max_tree: Vec<usize>,
}

impl CodeClosings {
    pub(super) fn new(source: &str, origin: usize) -> Self {
        let bytes = source.as_bytes();
        let mut runs = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] != b'`' {
                at += 1;
                continue;
            }
            let start = at;
            while at < bytes.len() && bytes[at] == b'`' {
                at += 1;
            }
            runs.push(BacktickRun {
                start: origin + start,
                end: origin + at,
            });
        }
        if runs.is_empty() {
            return Self::default();
        }

        let tree_base = runs.len().next_power_of_two();
        let mut max_tree = vec![0; tree_base * 2];
        for (index, run) in runs.iter().enumerate() {
            max_tree[tree_base + index] = run.end - run.start;
        }
        for index in (1..tree_base).rev() {
            max_tree[index] = max_tree[index * 2].max(max_tree[index * 2 + 1]);
        }
        Self {
            runs,
            tree_base,
            max_tree,
        }
    }

    fn run_index_at(&self, start: usize) -> Option<usize> {
        let index = self.runs.partition_point(|run| run.end <= start);
        self.runs
            .get(index)
            .filter(|run| run.start <= start)
            .map(|_| index)
    }

    /// `start` 所在 run 里剩余的反引号数量，截到 `limit` 为止。
    /// 解析器递归检查内部切片时需要这个截断。
    pub(super) fn delimiter_at_before(&self, start: usize, limit: usize) -> Option<usize> {
        let index = self.run_index_at(start)?;
        let run = self.runs.get(index)?;
        if start >= limit {
            return None;
        }
        let end = run.end.min(limit);
        (end > start).then_some(end - start)
    }

    fn first_run_at_least(
        &self,
        node: usize,
        left: usize,
        right: usize,
        from: usize,
        until: usize,
        needed: usize,
    ) -> Option<usize> {
        if right <= from || left >= until || self.max_tree[node] < needed {
            return None;
        }
        if right - left == 1 {
            return (left < self.runs.len()).then_some(left);
        }
        let middle = left + (right - left) / 2;
        self.first_run_at_least(node * 2, left, middle, from, until, needed)
            .or_else(|| self.first_run_at_least(node * 2 + 1, middle, right, from, until, needed))
    }

    /// 找到第一个能闭合 `start` 处分隔符的后续 run。
    pub(super) fn closing_start_before(
        &self,
        start: usize,
        limit: usize,
        needed: usize,
    ) -> Option<usize> {
        let current = self.run_index_at(start)?;
        let until = self.runs.partition_point(|run| run.start < limit);
        let mut from = current + 1;
        while from < until {
            let index = self.first_run_at_least(1, 0, self.tree_base, from, until, needed)?;
            let run = &self.runs[index];
            // 跨过内部切片边界的 run 可能比索引里的完整长度短。
            // 最多跳过一个这样的跨界 run，有界搜索就穷尽了。
            if run.end.min(limit) - run.start >= needed {
                return Some(run.start);
            }
            from = index + 1;
        }
        None
    }

    /// 判断旧的富文本匹配器是否会在此切片末尾自然结束扫描循环。
    /// 它的 `find('`')?` 在最后一个 run 后面还有普通文字时会提前返回，
    /// 所以这个区别对四反引号的自动补全回退是可观察的。
    pub(super) fn last_run_ends_at_before(&self, start: usize, limit: usize) -> bool {
        let Some(current) = self.run_index_at(start) else {
            return false;
        };
        let until = self.runs.partition_point(|run| run.start < limit);
        let Some(last) = until.checked_sub(1) else {
            return false;
        };
        last >= current && self.runs[last].end.min(limit) == limit
    }
}

/// 扫出一行源码里的全部链接。与 [`parse_inline`] 认同一套写法。
pub fn scan_links(line: &str) -> Vec<LinkSpan> {
    let mut out = Vec::new();
    let closings = LinkClosings::new(line);
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some((end, target)) = match_link_at(line, i, &closings, 0) {
            out.push(LinkSpan {
                start: i,
                end,
                target,
            });
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

/// `line[i..]` 是否以一条链接开头。返回 (结束偏移, 目标)。
fn match_link_at(
    line: &str,
    i: usize,
    closings: &LinkClosings,
    base: usize,
) -> Option<(usize, String)> {
    if !line.is_char_boundary(i) {
        return None;
    }
    let rest = &line[i..];
    let limit = base + line.len();
    let absolute_start = base + i;
    // [[维基链接]] / [[维基链接|显示]]
    if let Some(inner) = rest.strip_prefix("[[") {
        let absolute_close = LinkClosings::first_at_or_after(&closings.wiki, absolute_start + 2)?;
        if absolute_close + 2 > limit {
            return None;
        }
        let close = absolute_close - base - i - 2;
        let body = &inner[..close];
        if body.is_empty() || body.contains('\n') {
            return None;
        }
        let target = body.split('|').next().unwrap_or(body).trim().to_owned();
        return Some((absolute_close + 2 - base, target));
    }
    // [文字](url)
    if rest.starts_with('[')
        && !rest.starts_with("[ ]")
        && !rest.starts_with("[x]")
        && !rest.starts_with("[X]")
    {
        let absolute_close = LinkClosings::first_at_or_after(&closings.markdown, absolute_start)?;
        if absolute_close + 2 > limit {
            return None;
        }
        let absolute_paren_end =
            LinkClosings::first_at_or_after(&closings.parens, absolute_close + 1)?;
        if absolute_paren_end >= limit {
            return None;
        }
        let url_start = absolute_close + 2 - base;
        let paren_end = absolute_paren_end - base;
        let url = line.get(url_start..paren_end)?.trim();
        if url.is_empty() || url.contains(' ') && !url.starts_with('<') {
            return None;
        }
        return Some((
            absolute_paren_end + 1 - base,
            url.trim_matches(['<', '>']).to_owned(),
        ));
    }
    // 裸 URL：到空白或收尾标点为止
    for scheme in ["https://", "http://", "mochi://"] {
        if rest.starts_with(scheme) {
            let mut end = rest.len();
            for (j, c) in rest.char_indices() {
                if c.is_whitespace() || "<>\"'`)]}，。；：！？、》」』".contains(c) {
                    end = j;
                    break;
                }
            }
            let url = rest[..end].trim_end_matches(['.', ',']);
            if url.len() > scheme.len() {
                return Some((i + url.len(), url.to_owned()));
            }
        }
    }
    None
}

/// `[文字](url)` 的显示文字；维基链接取 `|` 后的别名或目标本身；裸 URL 显示本身。
fn link_label(source: &str) -> String {
    if let Some(inner) = source.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        return inner.rsplit('|').next().unwrap_or(inner).trim().to_owned();
    }
    if source.starts_with('[') {
        if let Some(close) = source.find("](") {
            return source[1..close].replace("\\[", "[").replace("\\]", "]");
        }
    }
    source.to_owned()
}

/// 源码偏移 `offset` 落在哪条链接上（含链接整段：文字、括号、URL 都算）。
pub fn link_at(line: &str, offset: usize) -> Option<LinkSpan> {
    scan_links(line)
        .into_iter()
        .find(|l| l.start <= offset && offset < l.end)
}

/// 一段同样式的连续文字。
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub emphasis: Emphasis,
}

impl Run {
    pub fn plain(text: impl Into<String>) -> Self {
        Run {
            text: text.into(),
            emphasis: Emphasis::None,
        }
    }
}

/// 把一行文字切成带样式的 run。标记符本身不进结果——
/// 把 `**` 原样画出来正是"看起来像源码而不是文档"的根源。
///
/// 没有配对的标记符按普通文字处理：`a * b` 里的星号是乘号，不是斜体开头。
pub fn parse_inline(line: &str) -> Vec<Run> {
    parse_inline_spans(line)
        .into_iter()
        .map(|s| s.run)
        .collect()
}

/// 与 [`parse_inline`] 同一套规则，但每个 run 带着它在源码里的字节范围。
/// 编辑器把点击位置映射回源码要用：公式、链接这类「显示文字不是源码子序列」的 run，
/// 只能按范围整段定位。
pub fn parse_inline_spans(line: &str) -> Vec<SpannedRun> {
    let closings = LinkClosings::new(line);
    let code_closings = CodeClosings::new(line, 0);
    parse_spans_inner(line, 0, &closings, &code_closings, 0)
}
fn parse_spans_inner(
    line: &str,
    depth: usize,
    closings: &LinkClosings,
    code_closings: &CodeClosings,
    base: usize,
) -> Vec<SpannedRun> {
    let chars: Vec<char> = line.chars().collect();
    // 字符下标 → 字节偏移
    let byte_of: Vec<usize> = line
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(line.len()))
        .collect();
    let mut runs: Vec<SpannedRun> = Vec::new();
    let mut buf = String::new();
    let mut buf_start = 0usize;
    let mut i = 0;

    let flush = |runs: &mut Vec<SpannedRun>, buf: &mut String, start: usize, end: usize| {
        if !buf.is_empty() {
            runs.push(SpannedRun {
                run: Run::plain(std::mem::take(buf)),
                start,
                end,
            });
        }
    };

    while i < chars.len() {
        let here = byte_of[i];
        if let Some(length) = super::block_markers::prefix_len(&line[here..]) {
            flush(&mut runs, &mut buf, buf_start, here);
            i = byte_of
                .binary_search(&(here + length))
                .unwrap_or(chars.len());
            buf_start = here + length;
            continue;
        }
        if chars[i] == '\\' && chars.get(i + 1).is_some_and(|c| c.is_ascii_punctuation()) {
            flush(&mut runs, &mut buf, buf_start, here);
            runs.push(SpannedRun {
                run: Run::plain(chars[i + 1].to_string()),
                start: here,
                end: byte_of[i + 2],
            });
            i += 2;
            buf_start = byte_of[i];
            continue;
        }
        if chars[i] == '<' && depth < 16 {
            if let Some(span) = super::styles::html_span(&line[here..]) {
                flush(&mut runs, &mut buf, buf_start, here);
                for mut child in parse_spans_inner(
                    &line[here + span.body_start..here + span.body_end],
                    depth + 1,
                    closings,
                    code_closings,
                    base + here + span.body_start,
                ) {
                    child.start += here + span.body_start;
                    child.end += here + span.body_start;
                    child.run.emphasis = super::styles::combine(child.run.emphasis, &span);
                    runs.push(child);
                }
                let end = here + span.end;
                i = byte_of.binary_search(&end).unwrap_or(chars.len());
                buf_start = end;
                continue;
            }
        }
        // 链接先于其它标记：`[**x**](u)` 整体是链接，URL 里的 `_` 也不是斜体
        if chars[i] == '[' || chars[i] == 'h' || chars[i] == 'm' {
            if let Some((end_byte, _)) = match_link_at(line, here, closings, base) {
                flush(&mut runs, &mut buf, buf_start, here);
                let source = &line[here..end_byte];
                runs.push(SpannedRun {
                    run: Run {
                        text: link_label(source),
                        emphasis: Emphasis::Link,
                    },
                    start: here,
                    end: end_byte,
                });
                i = byte_of.binary_search(&end_byte).unwrap_or(chars.len());
                buf_start = end_byte;
                continue;
            }
        }
        // 行内公式 `$…$`
        if chars[i] == '$' {
            if let Some(end_byte) = match_math_at(line, here) {
                flush(&mut runs, &mut buf, buf_start, here);
                let tex = &line[here + 1..end_byte - 1];
                runs.push(SpannedRun {
                    run: Run {
                        text: tex.to_owned(),
                        emphasis: Emphasis::Math,
                    },
                    start: here,
                    end: end_byte,
                });
                i = byte_of.binary_search(&end_byte).unwrap_or(chars.len());
                buf_start = end_byte;
                continue;
            }
        }
        // 行内代码优先：`` `**x**` `` 里的星号是代码内容，不是加粗
        if chars[i] == '`' {
            if let Some((delimiter, end)) = inline_code_close(code_closings, line, base, here) {
                let finish = end + delimiter;
                if end >= here + delimiter && finish <= line.len() {
                    flush(&mut runs, &mut buf, buf_start, here);
                    runs.push(SpannedRun {
                        run: Run {
                            text: line[here + delimiter..end].chars().collect(),
                            emphasis: Emphasis::Code,
                        },
                        start: here,
                        end: finish,
                    });
                    i = byte_of.binary_search(&finish).unwrap_or(chars.len());
                    buf_start = byte_of[i];
                    continue;
                }
            }
        }
        if depth < 16 {
            if let Some((begin, end, finish, emphasis)) = markdown_span(&line[here..]) {
                flush(&mut runs, &mut buf, buf_start, here);
                let children = parse_spans_inner(
                    &line[here + begin..here + end],
                    depth + 1,
                    closings,
                    code_closings,
                    base + here + begin,
                );
                if children.len() == 1 && children[0].run.emphasis == Emphasis::None {
                    runs.push(SpannedRun {
                        run: Run {
                            text: children[0].run.text.clone(),
                            emphasis,
                        },
                        start: here,
                        end: here + finish,
                    });
                } else {
                    let flags = match emphasis {
                        Emphasis::Bold => 1,
                        Emphasis::Italic => 2,
                        Emphasis::BoldItalic => 3,
                        Emphasis::Styled { flags, .. } => flags,
                        _ => 0,
                    };
                    let outer = super::styles::Span {
                        body_start: begin,
                        body_end: end,
                        end: finish,
                        fg: None,
                        bg: None,
                        flags,
                    };
                    for mut child in children {
                        child.start += here + begin;
                        child.end += here + begin;
                        child.run.emphasis = super::styles::combine(child.run.emphasis, &outer);
                        runs.push(child);
                    }
                }
                i = byte_of
                    .binary_search(&(here + finish))
                    .unwrap_or(chars.len());
                buf_start = byte_of[i];
                continue;
            }
        }
        if buf.is_empty() {
            buf_start = here;
        }
        buf.push(chars[i]);
        i += 1;
    }
    flush(&mut runs, &mut buf, buf_start, line.len());
    if runs.is_empty() {
        runs.push(SpannedRun {
            run: Run::plain(""),
            start: 0,
            end: 0,
        });
    }
    runs
}

/// 绘制与保源码编辑共用的行内分隔符边界。
pub fn markdown_span(raw: &str) -> Option<(usize, usize, usize, Emphasis)> {
    for (marker, emphasis) in [
        ("***", Emphasis::BoldItalic),
        ("___", Emphasis::BoldItalic),
        ("**", Emphasis::Bold),
        ("__", Emphasis::Bold),
        (
            "~~",
            Emphasis::Styled {
                fg: None,
                bg: None,
                flags: 8,
            },
        ),
        ("*", Emphasis::Italic),
        ("_", Emphasis::Italic),
    ] {
        let Some(body) = raw.strip_prefix(marker) else {
            continue;
        };
        let mut offset = 0;
        while let Some(found) = body[offset..].find(marker) {
            let mut end = marker.len() + offset + found;
            if raw[..end].ends_with('\\') {
                offset += found + marker.len();
                continue;
            }
            if marker.len() == 1 && raw[end..].starts_with(&marker.repeat(2)) {
                offset += found + 2;
                continue;
            }
            if marker.len() == 2
                && marker != "~~"
                && raw[end..].starts_with(&marker[..1].repeat(3))
                && raw[marker.len()..end].matches(&marker[..1]).count() % 2 == 1
            {
                end += 1;
            }
            if end > marker.len() {
                return Some((marker.len(), end, end + marker.len(), emphasis));
            }
            break;
        }
    }
    None
}

/// 行内代码允许成对数量的反引号。空的 ` `` ` 还不是输入完成态，必须等
/// Enter 的自动补全变成 ` ```` `；后者视为一对双反引号的空代码，光标可以
/// 安全地落在中间继续输入。
fn inline_code_close(
    closings: &CodeClosings,
    line: &str,
    base: usize,
    start: usize,
) -> Option<(usize, usize)> {
    let global_start = base + start;
    let limit = base + line.len();
    let delimiter = closings.delimiter_at_before(global_start, limit)?;
    if let Some(global_end) = closings.closing_start_before(global_start, limit, delimiter) {
        return Some((delimiter, global_end - base));
    }
    // 这是 ` `` + Enter ` 的自动补全形态。四个连续反引号可唯一解释为
    // 两侧各两个，且没有让普通的两个反引号过早进入渲染态。
    (delimiter == 4).then_some((2, start + 2))
}

/// 把带样式的 run 按宽度断成若干视觉行，每行仍是一串 run。
///
/// 按文本断行规则跨 run 累计宽度；
/// 一行里的粗体和正文共享同一个"还剩多少宽度"。
pub fn wrap_runs(runs: &[Run], style: TextStyle, max_width: f32) -> Vec<Vec<Run>> {
    if super::measurement::available()
        || matches!(style, TextStyle::Ai { .. })
        || runs.iter().any(|r| r.emphasis.base() == Emphasis::Math)
    {
        return super::math_runs::wrap(runs, style, max_width);
    }
    wrap_runs_impl(runs, style, max_width, false)
}

/// 源码编辑器绝不在软换行处裁剪空白：区间、光标、选区和保存
/// 都指向原始的 UTF-8 字节。
pub fn wrap_source(source: &str, style: TextStyle, max_width: f32) -> Vec<Vec<Run>> {
    if super::measurement::available() {
        return super::math_runs::wrap_source(source, style, max_width);
    }
    wrap_runs_impl(&[Run::plain(source)], style, max_width, true)
}

fn wrap_runs_impl(
    runs: &[Run],
    style: TextStyle,
    max_width: f32,
    preserve_spaces: bool,
) -> Vec<Vec<Run>> {
    // 先摊平成 (字符, 样式)，断完再按样式重新聚合。
    // 直接在 run 上断行要同时处理"run 内断"和"run 间断"两种情况，
    // 摊平后只剩一种，短得多也不容易错。
    let flat: Vec<(char, Emphasis)> = runs
        .iter()
        .flat_map(|r| r.text.chars().map(move |c| (c, r.emphasis)))
        .collect();
    if flat.is_empty() {
        return vec![vec![Run::plain("")]];
    }

    let em = style.font_size();
    let mut lines: Vec<Vec<(char, Emphasis)>> = Vec::new();
    let mut line: Vec<(char, Emphasis)> = Vec::new();
    let mut width = 0.0f32;
    // 最近一个可断点：(在 line 里的位置, 是否由空格产生)。
    // 空格断点要吃掉那个空格，CJK 断点不能吃掉任何字。
    let mut break_at: Option<(usize, bool)> = None;

    for (ch, emph) in flat {
        let advance = advance_for(ch, style, emph) * em;
        if width + advance > max_width && !line.is_empty() && max_width > em {
            match break_at {
                Some((at, is_space)) if at > 0 => {
                    let mut rest: Vec<(char, Emphasis)> = line[at..].to_vec();
                    line.truncate(at);
                    if is_space && !preserve_spaces {
                        // 断在空格处：行尾那个空格和续行开头的空格都不要
                        while line.last().map(|(c, _)| *c == ' ').unwrap_or(false) {
                            line.pop();
                        }
                        while rest.first().map(|(c, _)| *c == ' ').unwrap_or(false) {
                            rest.remove(0);
                        }
                    }
                    lines.push(std::mem::take(&mut line));
                    width = rest
                        .iter()
                        .map(|(c, e)| advance_for(*c, style, *e) * em)
                        .sum();
                    line = rest;
                }
                _ => {
                    lines.push(std::mem::take(&mut line));
                    width = 0.0;
                }
            }
            break_at = None;
        }
        line.push((ch, emph));
        width += advance;

        // 断点在**字符之后**登记。空格处可断（拉丁文），全角字之后也可断（CJK
        // 逐字可断）——只登记空格的话，一段中文溢出时会回退到很远的那个空格，
        // 把行拉得很短。截图里「以及深度的 AI」那行只填了一半就是这么来的。
        if ch == ' ' {
            break_at = Some((line.len(), true));
        } else if super::draw::is_wide(ch) {
            break_at = Some((line.len(), false));
        }
    }
    lines.push(line);

    // CJK 禁则：行首不能是收尾类标点。把它挪回上一行，宁可让上一行略微超出，
    // 也不要出现「。」孤零零挂在行首——那是中文排版里最扎眼的错误。
    fix_line_start_punctuation(&mut lines);

    lines.into_iter().map(regroup).collect()
}

/// 收尾类标点不该出现在行首。
pub(super) const CLOSING_PUNCTUATION: &str = "。，、；：？！）】》」』”’%·…—";

/// 一次最多往上一行挤几个字。真实的中文禁则处理也只悬挂一两个标点；
/// 不设上限的话，一段纯标点会被一路挤回第一行，collapse 成超长的一行。
const MAX_HANGING: usize = 2;

fn fix_line_start_punctuation(lines: &mut Vec<Vec<(char, Emphasis)>>) {
    let mut i = 1;
    while i < lines.len() {
        let mut moved = 0;
        while moved < MAX_HANGING
            && lines[i]
                .first()
                .map(|(c, _)| CLOSING_PUNCTUATION.contains(*c))
                .unwrap_or(false)
        {
            if lines[i - 1].is_empty() {
                break; // 上一行已空，无处可挪
            }
            let ch = lines[i].remove(0);
            lines[i - 1].push(ch);
            moved += 1;
        }
        if lines[i].is_empty() {
            // 整行被挪空，删掉——留着会渲染成一行空白
            lines.remove(i);
        } else {
            i += 1;
        }
    }
}

/// KaTeX 的公式字号是正文的 1.21em；测量与字体格式都按它放大。
pub const MATH_SCALE: f32 = 1.21;
pub fn math_scale(style: TextStyle) -> f32 {
    if matches!(style, TextStyle::Ai { .. }) {
        1.08
    } else {
        MATH_SCALE
    }
}

/// 行内代码用等宽字，字宽与正文不同——测量要按各自的档来。
pub(super) fn advance_for(ch: char, style: TextStyle, emphasis: Emphasis) -> f32 {
    match emphasis.base() {
        Emphasis::Code => {
            TextStyle::Mono.advance_em(ch)
                * if matches!(style, TextStyle::Ai { .. }) {
                    0.9
                } else {
                    1.0
                }
        }
        // 组合字符（根号上划线、帽子）不占宽
        Emphasis::Math if ('\u{0300}'..='\u{036F}').contains(&ch) || ch == '\u{20D7}' => 0.0,
        Emphasis::Math => style.advance_em(ch) * math_scale(style),
        _ => style.advance_em(ch),
    }
}

fn regroup(chars: Vec<(char, Emphasis)>) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (ch, emph) in chars {
        match runs.last_mut() {
            Some(last) if last.emphasis == emph => last.text.push(ch),
            _ => runs.push(Run {
                text: ch.to_string(),
                emphasis: emph,
            }),
        }
    }
    if runs.is_empty() {
        runs.push(Run::plain(""));
    }
    runs
}

/// 一串 run 的宽度，字体成形结果优先。
pub fn measure_runs(runs: &[Run], style: TextStyle) -> f32 {
    runs.iter().map(|r| run_width(r, style)).sum()
}

pub fn run_width(run: &Run, style: TextStyle) -> f32 {
    if run.emphasis.base() == Emphasis::Math {
        return super::math_layout::size(&run.text, style.font_size() * math_scale(style))
            .map(|(w, _)| w)
            .unwrap_or_else(|| {
                measure(&super::mathtext::render(&run.text), style) * math_scale(style)
            });
    }
    let padding = inline_code_padding(style, run.emphasis) * 2.0;
    if let Some(shape) = super::measurement::shape(&run.text, style, run.emphasis) {
        return shape.width + padding;
    }
    run.text
        .chars()
        .map(|c| advance_for(c, style, run.emphasis) * style.font_size())
        .sum::<f32>()
        + padding
}
/// AI 行内代码在其 .9em 字号下有 .4em 的水平内边距。
pub fn inline_code_padding(style: TextStyle, emphasis: Emphasis) -> f32 {
    if matches!(style, TextStyle::Ai { .. }) && emphasis.base() == Emphasis::Code {
        style.font_size() * 0.9 * 0.4
    } else {
        0.0
    }
}
pub fn runs_height(runs: &[Run], style: TextStyle) -> f32 {
    runs.iter()
        .filter(|r| r.emphasis.base() == Emphasis::Math)
        .filter_map(|r| super::math_layout::size(&r.text, style.font_size() * math_scale(style)))
        .map(|(_, h)| h + 4.0)
        .fold(style.line_height(), f32::max)
}

/// 源码映射用的稳定「虚拟字符」计数。公式作为一个整体盒子做命中测试；
/// 这个可读标签维持原有的偏移契约。
pub fn visible_len(run: &Run) -> usize {
    if run.emphasis.base() == Emphasis::Math {
        super::mathtext::render(&run.text).chars().count().max(1)
    } else {
        run.text.chars().count()
    }
}

/// 一行文字的实际宽度（DIP）；没有可用宿主时使用保守估算。
pub fn measure(text: &str, style: TextStyle) -> f32 {
    if let Some(shape) = super::measurement::shape(text, style, Emphasis::None) {
        return shape.width;
    }
    let em = style.font_size();
    text.chars().map(|c| style.advance_em(c) * em).sum::<f32>()
}

/// 源码光标必须在完整的成形行里测量，而不能只成形一个孤立前缀
///（那样会改变字距或拆散组合序列）。
pub fn caret_x(value: &str, byte: usize, style: TextStyle) -> f32 {
    if let Some(shape) = super::measurement::shape(value, style, Emphasis::None) {
        return shape.caret(byte);
    }
    let mut end = byte.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    measure(&value[..end], style)
}

/// 已渲染 run 内的光标位置。AI markdown 的布局保留了 run 的强调信息，
/// 所以选区必须用与绘制相同的成形/等宽规则，
/// 而不是把每个字符都当普通文字来量。
pub fn caret_x_with_emphasis(
    value: &str,
    byte: usize,
    style: TextStyle,
    emphasis: Emphasis,
) -> f32 {
    let mut end = byte.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    let padding = inline_code_padding(style, emphasis);
    if emphasis.base() == Emphasis::Math {
        // 渲染出来的公式是一个可选取的原子。光标保持在两端之一，
        // 拖过它时复制的才是原始 TeX 内容。
        return padding
            + if end == 0 {
                0.0
            } else {
                run_width(
                    &Run {
                        text: value.to_owned(),
                        emphasis,
                    },
                    style,
                ) - padding * 2.0
            };
    }
    let inner = if let Some(shape) = super::measurement::shape(value, style, emphasis) {
        shape.caret(end)
    } else {
        value[..end]
            .chars()
            .map(|ch| advance_for(ch, style, emphasis) * style.font_size())
            .sum()
    };
    padding + inner
}

/// 把已渲染 run 内的指针位置映射到最近的 UTF-8 边界。
/// `x` 相对 run 的布局矩形，含行内代码内边距。
/// 返回的字节偏移可安全用于切割原始字符串。
pub fn offset_at_x_with_emphasis(
    value: &str,
    style: TextStyle,
    emphasis: Emphasis,
    x: f32,
) -> usize {
    if value.is_empty() {
        return 0;
    }
    let total = caret_x_with_emphasis(value, value.len(), style, emphasis);
    let x = x.clamp(0.0, total.max(0.0));
    if emphasis.base() == Emphasis::Math {
        return if x < total / 2.0 { 0 } else { value.len() };
    }
    if let Some(shape) = super::measurement::shape(value, style, emphasis) {
        let inner_x = (x - inline_code_padding(style, emphasis)).max(0.0);
        return shape.hit(inner_x).min(value.len());
    }
    let padding = inline_code_padding(style, emphasis);
    let mut acc = padding;
    for (index, ch) in value.char_indices() {
        let width = advance_for(ch, style, emphasis) * style.font_size();
        if x < acc + width / 2.0 {
            return index;
        }
        acc += width;
    }
    value.len()
}

/// 按宽度截断并加省略号。标签页标题、文件名过长时用。
pub fn ellipsize(text: &str, style: TextStyle, max_width: f32) -> String {
    if measure(text, style) <= max_width {
        return text.to_owned();
    }
    let ellipsis = "…";
    let budget = max_width - measure(ellipsis, style);
    if budget <= 0.0 {
        return ellipsis.to_owned();
    }
    if let Some(shape) = super::measurement::shape(text, style, Emphasis::None) {
        // 只能在成形边界处截断，截完还要验证最终字符串——
        // 加省略号可能改变截断处的字距。
        let mut count = 0;
        let mut width = 0.0;
        for cluster in &shape.clusters {
            width += cluster.advance;
            if width > budget {
                break;
            }
            count += 1;
        }
        while count > 0 {
            let candidate = format!("{}…", &text[..shape.clusters[count - 1].end]);
            if measure(&candidate, style) <= max_width {
                return candidate;
            }
            count -= 1;
        }
        return ellipsis.to_owned();
    }
    let mut out = String::new();
    let mut w = 0.0;
    for ch in text.chars() {
        let a = style.advance_em(ch) * style.font_size();
        if w + a > budget {
            break;
        }
        out.push(ch);
        w += a;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_links_render_their_label_and_keep_the_target_for_lookup() {
        let runs = parse_inline("看 [文档](https://a.b/c) 和 **粗**");
        assert_eq!(
            runs[1],
            Run {
                text: "文档".into(),
                emphasis: Emphasis::Link
            }
        );
        assert_eq!(
            runs[3],
            Run {
                text: "粗".into(),
                emphasis: Emphasis::Bold
            }
        );
        let links = scan_links("看 [文档](https://a.b/c) 和 **粗**");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "https://a.b/c");
        assert_eq!(
            &"看 [文档](https://a.b/c) 和 **粗**"[links[0].start..links[0].end],
            "[文档](https://a.b/c)"
        );
        // 偏移落在 URL 上也算这条链接
        assert_eq!(
            link_at("看 [文档](https://a.b/c)", 14).map(|l| l.target),
            Some("https://a.b/c".to_owned())
        );
        assert!(link_at("看 [文档](https://a.b/c)", 0).is_none());
    }

    #[test]
    fn inline_math_follows_the_tiptap_input_rule_and_carries_its_source_span() {
        let line = "能量 $E = mc^2$ 与 $\\alpha$。";
        let spans = parse_inline_spans(line);
        assert_eq!(
            spans[1].run,
            Run {
                text: "E = mc^2".into(),
                emphasis: Emphasis::Math
            }
        );
        assert_eq!(&line[spans[1].start..spans[1].end], "$E = mc^2$");
        assert_eq!(spans[3].run.text, "\\alpha");
        // `$ 5` 开头带空格、`5 $` 结尾带空格：不是公式（与 TipTap 的正则一致）
        assert_eq!(
            parse_inline("花了 $ 5 和 6 $ 元"),
            vec![Run::plain("花了 $ 5 和 6 $ 元")]
        );
        assert_eq!(parse_inline("$$"), vec![Run::plain("$$")]);
        // 普通 run 的范围含标记符
        let spans = parse_inline_spans("a **b** c");
        assert_eq!((spans[1].start, spans[1].end), (2, 7));
        assert_eq!((spans[2].start, spans[2].end), (7, 9));
    }

    #[test]
    fn inline_formulas_are_atomic_when_wrapping_and_adjacent_formulas_stay_separate() {
        let runs = parse_inline(r"中文 $\frac{a+b}{c+d}$ 后面 $x^2$$y^2$。");
        let wrapped = wrap_runs(&runs, TextStyle::Document, 100.0);
        let math = wrapped
            .iter()
            .flatten()
            .filter(|r| r.emphasis == Emphasis::Math)
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(math, vec![r"\frac{a+b}{c+d}", "x^2", "y^2"]);
        let tall = parse_inline(r"文字 $\begin{pmatrix}a\\b\\c\end{pmatrix}$ 结尾");
        assert!(runs_height(&tall, TextStyle::Document) > TextStyle::Document.line_height());
    }

    #[test]
    fn wiki_links_bare_urls_and_task_brackets_are_told_apart() {
        let runs = parse_inline("[[笔记|别名]] 与 https://x.y/z。");
        assert_eq!(
            runs[0],
            Run {
                text: "别名".into(),
                emphasis: Emphasis::Link
            }
        );
        assert_eq!(
            runs[2],
            Run {
                text: "https://x.y/z".into(),
                emphasis: Emphasis::Link
            },
            "句号不算进 URL"
        );
        assert_eq!(scan_links("[[笔记|别名]]")[0].target, "笔记");
        // `[ ]` 是任务框不是链接；没有 `(url)` 的方括号是普通文字
        assert!(scan_links("- [ ] 事项 [备注] 文字").is_empty());
        assert_eq!(parse_inline("[备注] 文字"), vec![Run::plain("[备注] 文字")]);
        // 链接里的下划线不是斜体
        let runs = parse_inline("[a_b](https://h/x_y_z)");
        assert_eq!(
            runs,
            vec![Run {
                text: "a_b".into(),
                emphasis: Emphasis::Link
            }]
        );
    }

    #[test]
    fn link_index_keeps_malformed_nested_escaped_and_bare_url_semantics() {
        let nested = "[outer [inner]](https://x/y)";
        assert_eq!(
            parse_inline(nested),
            vec![Run {
                text: "outer [inner]".into(),
                emphasis: Emphasis::Link,
            }]
        );
        assert_eq!(scan_links(nested)[0].target, "https://x/y");

        // 行内解析器把转义的起始方括号当作普通文字；
        // 它后面的链接不能被意外拼到那段文字上。
        let escaped = parse_inline(r"\[literal](url)");
        assert_eq!(
            escaped
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>(),
            "[literal](url)"
        );
        assert!(escaped.iter().all(|run| run.emphasis == Emphasis::None));

        let wiki = scan_links("[[target|alias]]").pop().unwrap();
        assert_eq!(
            &"[[target|alias]]"[wiki.start..wiki.end],
            "[[target|alias]]"
        );
        assert_eq!(wiki.target, "target");

        let bare = scan_links("see https://x/y.").pop().unwrap();
        assert_eq!(bare.target, "https://x/y");
        assert!(scan_links("[missing](bad url)").is_empty());
    }

    #[test]
    fn many_unclosed_brackets_remain_literal_without_rescanning_each_suffix() {
        // 这里过去会对每个起始方括号在剩余后缀上跑一遍 find("](")。
        // 输入保持足够大，才能捕获二次方复杂度的退化；
        // 同时又足够小，不影响日常单元测试的运行。
        let mut source = String::with_capacity(20_000 + 8);
        source.extend(std::iter::repeat_n('[', 20_000));
        source.push_str(" tail");

        let spans = parse_inline_spans(&source);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].run.emphasis, Emphasis::None);
        assert_eq!(spans[0].run.text, source);
        assert!(scan_links(&source).is_empty());
    }

    fn reference_inline_code_close(source: &str, start: usize) -> Option<(usize, usize)> {
        let bytes = source.as_bytes();
        let delimiter = bytes[start..]
            .iter()
            .take_while(|byte| **byte == b'`')
            .count();
        if delimiter == 0 {
            return None;
        }
        let mut at = start + delimiter;
        while at < bytes.len() {
            if bytes[at] == b'`' {
                let closing = bytes[at..].iter().take_while(|byte| **byte == b'`').count();
                if closing >= delimiter {
                    return Some((delimiter, at));
                }
                at += closing;
            } else {
                at += source[at..].chars().next().map(char::len_utf8).unwrap_or(1);
            }
        }
        (delimiter == 4).then_some((2, start + 2))
    }

    fn assert_index_matches_reference(source: &str) {
        let closings = CodeClosings::new(source, 0);
        for (start, _) in source.char_indices().filter(|(_, ch)| *ch == '`') {
            assert_eq!(
                inline_code_close(&closings, source, 0, start),
                reference_inline_code_close(source, start),
                "source={source:?}, start={start}"
            );
        }
    }

    #[test]
    fn code_closing_index_matches_the_old_scan_on_exhaustive_unicode_inputs() {
        fn visit(source: &mut String, remaining: usize) {
            assert_index_matches_reference(source);
            if remaining == 0 {
                return;
            }
            for ch in ['`', 'a', '中', '🙂'] {
                source.push(ch);
                visit(source, remaining - 1);
                source.pop();
            }
        }

        visit(&mut String::new(), 5);

        // 第二组确定性样本覆盖更长的 Unicode 正文和分隔符 run，
        // 而不引入测试期 RNG 依赖。
        let alphabet = ['`', 'x', '界', '😀', '\u{301}'];
        let mut state = 0x9e37_79b9_u64;
        for _ in 0..256 {
            let length = (next_test_state(&mut state) % 48) as usize;
            let mut source = String::new();
            for _ in 0..length {
                source
                    .push(alphabet[(next_test_state(&mut state) % alphabet.len() as u64) as usize]);
            }
            assert_index_matches_reference(&source);
        }
    }

    #[test]
    fn shared_code_index_matches_local_slices_at_utf8_and_run_boundaries() {
        let source = "前``a```中`🙂``尾````后";
        let closings = CodeClosings::new(source, 0);
        let mut boundaries = source.char_indices().map(|(at, _)| at).collect::<Vec<_>>();
        boundaries.push(source.len());
        for (start_index, start) in boundaries.iter().copied().enumerate() {
            for end in boundaries.iter().copied().skip(start_index + 1) {
                let slice = &source[start..end];
                let local = CodeClosings::new(slice, 0);
                for (at, _) in slice.char_indices().filter(|(_, ch)| *ch == '`') {
                    assert_eq!(
                        inline_code_close(&closings, slice, start, at),
                        inline_code_close(&local, slice, 0, at),
                        "slice={slice:?}, start={start}, at={at}, end={end}"
                    );
                }
            }
        }
    }

    fn next_test_state(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        *state
    }

    #[test]
    fn a_million_unclosed_backticks_do_not_rescan_the_suffix() {
        let count = 1_000_000;
        let mut source = "`".repeat(count);
        source.push_str("🙂尾");
        let spans = parse_inline_spans(&source);

        // 保留历史上的四反引号输入补全回退：
        // 只有最后四个反引号构成那个空的代码 run。
        assert_eq!(spans[0].run.text.chars().count(), count - 4);
        assert_eq!(spans[0].run.emphasis, Emphasis::None);
        assert_eq!(
            spans[1].run,
            Run {
                text: String::new(),
                emphasis: Emphasis::Code
            }
        );
        assert_eq!(spans.last().unwrap().run.text, "🙂尾");
    }

    /// 无样式文字的断行。走的是产品路径 `wrap_runs`——
    /// 早先这里有个独立的 `wrap` 实现，是同一套算法的第二份拷贝，已删。
    /// 断行规则：CJK 逐字可断、拉丁按空格断、塞不下的长单词硬切。
    fn wrap(text: &str, style: TextStyle, max_width: f32) -> Vec<String> {
        wrap_runs(&[Run::plain(text)], style, max_width)
            .iter()
            .map(|line| line.iter().map(|r| r.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn a_short_line_is_not_wrapped() {
        assert_eq!(wrap("短", TextStyle::Body, 500.0), vec!["短"]);
    }

    #[test]
    fn an_empty_string_still_yields_one_line() {
        // 空段落也要占一行高，否则连续空行会被折叠掉
        assert_eq!(wrap("", TextStyle::Body, 500.0), vec![""]);
    }

    #[test]
    fn chinese_wraps_per_character_because_it_has_no_spaces() {
        // 13px 正文、全角 1em：宽 65px 放得下 5 个字
        let lines = wrap("一二三四五六七八九十", TextStyle::Body, 65.0);
        assert_eq!(lines, vec!["一二三四五", "六七八九十"]);
    }

    #[test]
    fn latin_wraps_at_spaces_not_mid_word() {
        let lines = wrap("the quick brown fox", TextStyle::Body, 80.0);
        assert!(lines.len() > 1);
        // 没有哪一行是从半个单词开始的
        for line in &lines {
            assert!(!line.starts_with(' '), "行首不该留空格: {line:?}");
        }
        assert_eq!(lines.join(" "), "the quick brown fox");
    }

    #[test]
    fn a_single_word_too_long_to_fit_is_hard_broken() {
        // 没有可断点时必须硬切，否则这一行会无限撑出去
        let lines = wrap("supercalifragilistic", TextStyle::Body, 40.0);
        assert!(lines.len() > 1);
        assert_eq!(lines.concat(), "supercalifragilistic");
    }

    #[test]
    fn a_width_narrower_than_one_character_does_not_hang() {
        // 面板被拖到极窄时会走到这里；死循环比画错严重得多
        let lines = wrap("一二三", TextStyle::Body, 2.0);
        assert_eq!(lines, vec!["一二三"]);
    }

    #[test]
    fn wide_characters_count_as_a_full_em_and_latin_as_about_half() {
        let body = TextStyle::Body;
        assert_eq!(body.advance_em('中'), 1.0);
        assert!(body.advance_em('a') < 0.6);
        // 同样字数，中文比英文宽
        assert!(measure("中中中", body) > measure("aaa", body));
    }

    #[test]
    fn headings_are_larger_and_taller_than_body_text() {
        assert!(TextStyle::Heading1.font_size() > TextStyle::Heading2.font_size());
        assert!(TextStyle::Heading2.font_size() > TextStyle::Heading3.font_size());
        assert!(TextStyle::Heading3.font_size() > TextStyle::Body.font_size());
        assert!(TextStyle::Heading1.line_height() > TextStyle::Body.line_height());
    }

    #[test]
    fn ellipsize_leaves_short_text_alone() {
        assert_eq!(ellipsize("笔记.md", TextStyle::Body, 500.0), "笔记.md");
    }

    #[test]
    fn ellipsize_truncates_and_appends_the_ellipsis() {
        let out = ellipsize("一二三四五六七八九十", TextStyle::Body, 65.0);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() < 10);
        assert!(measure(&out, TextStyle::Body) <= 65.0);
    }

    #[test]
    fn ellipsize_degrades_to_just_the_ellipsis_when_there_is_no_room() {
        assert_eq!(ellipsize("很长的标题", TextStyle::Body, 3.0), "…");
    }

    #[test]
    fn wrapping_never_loses_or_duplicates_characters() {
        // 断行是最容易吞字/重字的地方，用一段中英混排兜住
        let src = "墨池 Mochi 是一个 AI 原生的桌面笔记应用，本地优先、Git 版本控制。";
        for width in [40.0, 77.0, 120.0, 333.0] {
            let joined: String = wrap(src, TextStyle::Body, width).concat();
            let a: String = joined.chars().filter(|c| !c.is_whitespace()).collect();
            let b: String = src.chars().filter(|c| !c.is_whitespace()).collect();
            assert_eq!(a, b, "宽度 {width} 处断行丢字了");
        }
    }

    // ---------- 行内标记 ----------

    fn plain_text(runs: &[Run]) -> String {
        runs.iter().map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn bold_markers_are_consumed_not_rendered() {
        // 把 ** 原样画出来正是「看着像源码而不是文档」的根源
        let runs = parse_inline("这是**原生渲染**的示例");
        assert_eq!(plain_text(&runs), "这是原生渲染的示例");
        assert_eq!(
            runs.iter()
                .find(|r| r.text == "原生渲染")
                .map(|r| r.emphasis),
            Some(Emphasis::Bold)
        );
    }

    #[test]
    fn italic_and_code_are_recognised() {
        let runs = parse_inline("普通 *斜体* 和 `代码`");
        assert_eq!(plain_text(&runs), "普通 斜体 和 代码");
        assert_eq!(
            runs.iter().find(|r| r.text == "斜体").map(|r| r.emphasis),
            Some(Emphasis::Italic)
        );
        assert_eq!(
            runs.iter().find(|r| r.text == "代码").map(|r| r.emphasis),
            Some(Emphasis::Code)
        );
    }

    #[test]
    fn double_backticks_need_a_completed_span_before_becoming_inline_code() {
        assert_eq!(parse_inline("``"), vec![Run::plain("``")]);
        assert_eq!(
            parse_inline("``value``"),
            vec![Run {
                text: "value".into(),
                emphasis: Emphasis::Code,
            }]
        );
    }

    #[test]
    fn bold_is_matched_before_italic() {
        // 先试斜体的话，`**` 会被当成一对空斜体，加粗就永远匹配不上
        let runs = parse_inline("**粗**");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].emphasis, Emphasis::Bold);
        assert_eq!(runs[0].text, "粗");
    }

    #[test]
    fn an_unpaired_marker_stays_literal() {
        // `a * b` 里的星号是乘号
        let runs = parse_inline("2 * 3 = 6");
        assert_eq!(plain_text(&runs), "2 * 3 = 6");
        assert!(runs.iter().all(|r| r.emphasis == Emphasis::None));
    }

    #[test]
    fn markup_inside_inline_code_is_left_alone() {
        // `` `**x**` `` 里的星号是代码内容
        let runs = parse_inline("写作 `**粗**` 时");
        let code = runs.iter().find(|r| r.emphasis == Emphasis::Code).unwrap();
        assert_eq!(code.text, "**粗**");
    }

    #[test]
    fn a_line_without_any_markup_is_a_single_plain_run() {
        let runs = parse_inline("完全没有标记");
        assert_eq!(runs, vec![Run::plain("完全没有标记")]);
    }

    #[test]
    fn an_empty_line_yields_one_empty_run() {
        assert_eq!(parse_inline(""), vec![Run::plain("")]);
    }

    #[test]
    fn wrapping_runs_keeps_the_emphasis_attached_to_the_right_characters() {
        let runs = parse_inline("一二三**四五六**七八九");
        let lines = wrap_runs(&runs, TextStyle::Body, 65.0); // 5 个全角字一行
        assert!(lines.len() > 1);

        let joined: String = lines
            .iter()
            .flat_map(|l| l.iter().map(|r| r.text.as_str()))
            .collect();
        assert_eq!(joined, "一二三四五六七八九");

        // "四五六" 三个字无论落在哪一行都必须还是粗体
        let bold: String = lines
            .iter()
            .flat_map(|l| l.iter())
            .filter(|r| r.emphasis == Emphasis::Bold)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(bold, "四五六");
    }

    #[test]
    fn wrapping_runs_never_loses_characters() {
        let src = "墨池 **Mochi** 是一个 `AI` 原生的*桌面*笔记应用，本地优先。";
        let runs = parse_inline(src);
        let expected: String = plain_text(&runs)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        for width in [40.0, 90.0, 200.0] {
            let got: String = wrap_runs(&runs, TextStyle::Body, width)
                .iter()
                .flat_map(|l| l.iter().map(|r| r.text.as_str()))
                .collect::<String>()
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            assert_eq!(got, expected, "宽度 {width} 处丢字");
        }
    }

    #[test]
    fn adjacent_runs_with_the_same_emphasis_are_merged_back_together() {
        // 摊平再聚合时如果不合并，一行会碎成几十条绘制指令
        let runs = parse_inline("一段完全没有标记的长文字");
        let lines = wrap_runs(&runs, TextStyle::Body, 1000.0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 1);
    }

    #[test]
    fn inline_code_is_measured_with_the_monospace_advance() {
        // 等宽字比正文宽，测量用错档会让断行位置系统性偏移
        let code = vec![Run {
            text: "aaaa".into(),
            emphasis: Emphasis::Code,
        }];
        let plain = vec![Run::plain("aaaa")];
        assert!(measure_runs(&code, TextStyle::Body) > measure_runs(&plain, TextStyle::Body));
    }

    #[test]
    fn a_cjk_run_after_a_space_breaks_in_place_not_back_at_the_space() {
        // 只把空格记为断点的话，「以及深度的 AI 集成；这一段…」溢出时会一路回退到
        // AI 后面那个空格，把行拉得只有一半宽——实测截图里就是这个样子。
        let src = "以及深度的 AI 集成；这一段特意写长一些好看看效果";
        let lines = wrap(src, TextStyle::Body, 156.0); // 约 12 个全角字
        assert!(lines.len() >= 2);
        // 首行应当基本填满，而不是在 "AI" 后面就断掉
        let first = &lines[0];
        assert!(
            measure(first, TextStyle::Body) > 156.0 * 0.75,
            "首行只填了 {:.0}px（上限 156），断早了：{first:?}",
            measure(first, TextStyle::Body)
        );
    }

    #[test]
    fn latin_still_breaks_at_spaces_and_drops_them() {
        // CJK 断点加进来之后，纯拉丁文的行为不能跟着变
        let lines = wrap("the quick brown fox jumps", TextStyle::Body, 80.0);
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(!line.starts_with(' '), "行首留了空格: {line:?}");
            assert!(!line.ends_with(' '), "行尾留了空格: {line:?}");
        }
        assert_eq!(lines.join(" "), "the quick brown fox jumps");
    }

    #[test]
    fn a_line_never_starts_with_closing_punctuation() {
        // 「。」孤零零挂在行首是中文排版里最扎眼的错误
        for width in [60.0, 78.0, 91.0, 104.0, 130.0] {
            let lines = wrap(
                "这是一句话。这是第二句话，还有第三句。",
                TextStyle::Body,
                width,
            );
            for line in lines.iter().skip(1) {
                let first = line.chars().next().unwrap_or('x');
                assert!(
                    !CLOSING_PUNCTUATION.contains(first),
                    "宽度 {width} 时行首出现了 {first:?}：{lines:?}"
                );
            }
        }
    }

    #[test]
    fn pushing_punctuation_back_never_empties_a_line_or_hangs() {
        // 一段纯标点是病态输入：禁则规则在这里必然让步（每行只肯往上挤两个字），
        // 但**必须**保证不死循环、不丢字。内容正确优先于排版好看。
        let lines = wrap("。。。。。。。。", TextStyle::Body, 40.0);
        assert!(!lines.is_empty());
        assert_eq!(lines.concat(), "。。。。。。。。");
        assert!(
            lines.iter().all(|l| !l.is_empty()),
            "不该留下空行: {lines:?}"
        );
    }
}
