//! 在富文本块内容与 Markdown 源码范围之间建立映射。
use super::*;

#[derive(Debug, Clone)]
pub struct Mapping {
    pub units: UnitList,
    pub(super) wrappers: WrapperList,
    pub start: usize,
    pub end: usize,
}

/// 定位真正的行内正文，包括 HTML 对齐包裹和列表缩进。在已知正文里
/// 搜索，才不会匹配到目标或 HTML 属性里的 wiki 链接别名。
pub fn body_range(rb: &RangedBlock, source: &str) -> Range<usize> {
    let raw = &source[rb.start..rb.end];
    let body = match &rb.block {
        Block::Heading { text, .. }
        | Block::ListItem { text, .. }
        | Block::Aligned { text, .. }
        | Block::Paragraph(text)
        | Block::Quote(text) => text,
        _ => return rb.start..rb.end,
    };
    let prefix = match &rb.block {
        Block::Aligned { .. } => raw.find('>').map(|i| i + 1).unwrap_or(0),
        Block::Heading { .. } | Block::ListItem { .. } | Block::Quote(_) => {
            let indent = raw.len() - raw.trim_start().len();
            let s = &raw[indent..];
            let task = s.as_bytes();
            let n = if task.len() >= 6
                && matches!(task[0], b'-' | b'*' | b'+')
                && task[1..3] == *b" ["
                && matches!(task[3], b' ' | b'x' | b'X')
                && task[4..6] == *b"] "
            {
                6
            } else if s.starts_with('>') {
                1
            } else {
                s.find(' ').map(|i| i + 1).unwrap_or(0)
            };
            (indent + n).min(raw.len())
        }
        _ => 0,
    };
    let start = rb.start + prefix + raw[prefix..].find(body.as_str()).unwrap_or(0);
    start..(start + body.len()).min(rb.end)
}

impl Mapping {
    /// 普通文本块的廉价精确答案。有歧义的标记交给 `block_text`，
    /// 评论序号的过滤结果因此保持不变。
    pub(in crate::ui) fn obvious_block_text(rb: &RangedBlock, source: &str) -> Option<bool> {
        if matches!(rb.block, Block::Code { .. }) {
            return Some(!source[code_body(rb, source)].is_empty());
        }
        if matches!(
            rb.block,
            Block::Container(_)
                | Block::Table { .. }
                | Block::Image { .. }
                | Block::Divider
                | Block::AiLocator(_)
                | Block::ObjectReference(_)
                | Block::Math(_)
        ) {
            return Some(false);
        }
        let body = &source[body_range(rb, source)];
        if body.is_empty() {
            return Some(false);
        }
        // 链接/HTML/实体/公式会吞掉本该可见的字母。
        // 没有这些结构时，字母或空白穿过行内标记仍会保留。
        if !body.contains(['<', '[', '$', '&'])
            && body
                .chars()
                .any(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        {
            Some(true)
        } else {
            None
        }
    }

    /// 返回可见的锚点文字，不分配字符表，也不建编辑包裹。
    /// 叶子对齐与 `from_inline_storage` 保持一致：解码后但不出现在
    /// 源码正文里的字符同样不算映射单元。
    pub(in crate::ui) fn block_text(rb: &RangedBlock, source: &str) -> String {
        if matches!(rb.block, Block::Code { .. }) {
            return source[code_body(rb, source)].to_owned();
        }
        if matches!(
            rb.block,
            Block::Container(_)
                | Block::Table { .. }
                | Block::Image { .. }
                | Block::Divider
                | Block::AiLocator(_)
                | Block::ObjectReference(_)
                | Block::Math(_)
        ) {
            return String::new();
        }
        let content = &source[body_range(rb, source)];
        let mut text = String::with_capacity(content.len());
        for span in crate::ui::text::parse_inline_spans(content) {
            if span.run.emphasis.base() == crate::ui::text::Emphasis::Math {
                text.push('$');
                text.push_str(&span.run.text);
                text.push('$');
                continue;
            }
            let full = &content[span.start..span.end];
            let bounds = inline_body(full, span.run.emphasis);
            let mut at = bounds.start;
            for ch in span.run.text.chars() {
                let Some(found) = full[at..bounds.end].find(ch) else {
                    continue;
                };
                at += found + ch.len_utf8();
                text.push(ch);
            }
        }
        text
    }

    /// 缓存的块可能因文档前方的编辑而移动。单元和包裹区间相对于
    /// 共享源码保持不变；rebase 只挪动这个轻量视图的原点。
    pub(in crate::ui) fn rebase(&mut self, delta: isize) {
        self.units.rebase(delta);
        self.wrappers.rebase(delta);
        self.start = self.start.saturating_add_signed(delta);
        self.end = self.end.saturating_add_signed(delta);
    }

    pub fn inline(source: &str, range: Range<usize>) -> Self {
        let storage: Rc<str> = Rc::from(&source[range.clone()]);
        Self::from_inline_storage(storage, 0..range.len(), range.start)
    }

    /// 构造简单的单字符映射，用于代码体和评论锚点——
    /// 那里每个源码字符都原样可见。
    pub(in crate::ui) fn plain(source: &str, range: Range<usize>) -> Self {
        let storage: Rc<str> = Rc::from(&source[range.clone()]);
        let mut builder = UnitBuilder::default();
        for (visible, (start, ch)) in storage.char_indices().enumerate() {
            let end = start + ch.len_utf8();
            builder.push_source(start..end, visible..visible + 1);
        }
        let (chunks, unit_count) = builder.finish(&storage);
        Self::from_parts(
            storage,
            range.start,
            chunks,
            unit_count,
            Vec::new(),
            range.start,
            range.end,
        )
    }

    pub(super) fn from_inline_storage(
        storage: Rc<str>,
        content_range: Range<usize>,
        origin: usize,
    ) -> Self {
        let content = &storage[content_range.clone()];
        let mut wrappers = Vec::new();
        let code_closings = text::CodeClosings::new(content, content_range.start);
        collect_wrappers(
            content,
            content_range.start,
            &code_closings,
            &mut wrappers,
            0,
        );
        wrappers.sort_by_key(|w| (w.full.start, w.full.end, w.body.start, w.body.end));
        wrappers.dedup();
        let mut visible = 0;
        let mut builder = UnitBuilder::default();
        for span in text::parse_inline_spans(content) {
            let full = &content[span.start..span.end];
            let absolute = content_range.start + span.start;
            if span.run.emphasis.base() == text::Emphasis::Math {
                let count = text::visible_len(&span.run);
                builder.push_single(UnitData {
                    source: absolute..content_range.start + span.end,
                    visible: visible..visible + count,
                    text: UnitText::Owned(Rc::from(format!("${}$", span.run.text))),
                });
                visible += count;
                continue;
            }
            let bounds = inline_body(full, span.run.emphasis);
            let mut at = bounds.start;
            for ch in span.run.text.chars() {
                // 普通和带样式的叶子文本都是源码子串。
                // 对齐只落在标签/正文上，绝不落到分隔符或目标上。
                let Some(found) = full[at..bounds.end].find(ch) else {
                    continue;
                };
                at += found;
                let escaped = at > bounds.start
                    && full.as_bytes()[at - 1] == b'\\'
                    && ch.is_ascii_punctuation();
                let source_start = absolute + at - usize::from(escaped);
                let text_start = absolute + at;
                let source = source_start..absolute + at + ch.len_utf8();
                let text = text_start..text_start + ch.len_utf8();
                if escaped {
                    builder.push_single(UnitData {
                        source,
                        visible: visible..visible + 1,
                        text: UnitText::Source(text),
                    });
                } else {
                    debug_assert_eq!(source, text);
                    builder.push_source(source, visible..visible + 1);
                }
                visible += 1;
                at += ch.len_utf8();
            }
        }
        let (chunks, unit_count) = builder.finish(&storage);
        let mut start = origin + content_range.start;
        let mut end = origin + content_range.end;
        if let Some(first) = chunks.first() {
            start = origin + first.source_start();
        }
        if let Some(last) = chunks.last() {
            end = origin + last.source_end();
        }
        // 空的 HTML 标记是合法的输入位置，不是可见的标签。
        if unit_count == 0 {
            if let Some(w) = wrappers.iter().rev().find(|w| w.body.is_empty()) {
                start = origin + w.body.start;
                end = origin + w.body.end;
            }
        }
        Self::from_parts(storage, origin, chunks, unit_count, wrappers, start, end)
    }

    pub fn block(rb: &RangedBlock, source: &str) -> Self {
        if matches!(
            rb.block,
            Block::Code { .. }
                | Block::Container(_)
                | Block::Table { .. }
                | Block::Image { .. }
                | Block::Divider
                | Block::AiLocator(_)
                | Block::ObjectReference(_)
                | Block::Math(_)
        ) {
            return Self::empty(rb.start, rb.end);
        }
        let raw = source.get(rb.start..rb.end).unwrap_or("");
        let storage: Rc<str> = Rc::from(raw);
        Self::block_with_storage(rb, source, storage)
    }

    /// 与 [`Mapping::block`] 相同，但复用缓存条目的不可变原始块存储，
    /// 缓存键与映射便不必各自持有重复的文本。
    pub(in crate::ui) fn block_with_storage(
        rb: &RangedBlock,
        source: &str,
        storage: Rc<str>,
    ) -> Self {
        if matches!(
            rb.block,
            Block::Code { .. }
                | Block::Container(_)
                | Block::Table { .. }
                | Block::Image { .. }
                | Block::Divider
                | Block::AiLocator(_)
                | Block::ObjectReference(_)
                | Block::Math(_)
        ) {
            return Self::empty(rb.start, rb.end);
        }
        let body = body_range(rb, source);
        let content_range = body.start.saturating_sub(rb.start)..body.end.saturating_sub(rb.start);
        Self::from_inline_storage(storage, content_range, rb.start)
    }

    pub(super) fn from_parts(
        storage: Rc<str>,
        origin: usize,
        chunks: Vec<UnitChunk>,
        unit_count: usize,
        wrappers: Vec<WrapperData>,
        start: usize,
        end: usize,
    ) -> Self {
        let data = Rc::new(MappingData {
            storage,
            chunks,
            unit_count,
            wrappers,
        });
        Self {
            units: UnitList::new(data.clone(), origin),
            wrappers: WrapperList::new(data, origin),
            start,
            end,
        }
    }

    pub(super) fn empty(start: usize, end: usize) -> Self {
        Self {
            units: UnitList::default(),
            wrappers: WrapperList::default(),
            start,
            end,
        }
    }

    pub(in crate::ui) fn memory_bytes(&self) -> usize {
        // 两个视图共享同一份 `MappingData`；分配只计一次。
        self.units.memory_bytes()
    }

    pub fn source_at(&self, visible: usize) -> usize {
        let chunks = &self.units.data.chunks;
        let index = chunks.partition_point(|chunk| chunk.visible().end <= visible);
        let Some(chunk) = chunks.get(index) else {
            return self.end;
        };
        match chunk {
            UnitChunk::Source(chunk) => {
                if visible <= chunk.visible.start {
                    self.units.origin + chunk.source.start
                } else {
                    let unit = (visible - chunk.visible.start).min(chunk.unit_count() - 1);
                    self.units.origin + chunk.byte_at(&self.units.data.storage, unit)
                }
            }
            UnitChunk::Single(unit) => {
                if visible <= unit.visible.start {
                    self.units.origin + unit.source.start
                } else {
                    self.units.origin + unit.source.end
                }
            }
        }
    }

    pub fn visible_at(&self, source: usize) -> usize {
        let chunks = &self.units.data.chunks;
        let index = chunks.partition_point(|chunk| {
            self.units.origin.saturating_add(chunk.source_end()) <= source
        });
        let Some(chunk) = chunks.get(index) else {
            return chunks.last().map(|chunk| chunk.visible().end).unwrap_or(0);
        };
        match chunk {
            UnitChunk::Source(chunk) => {
                let relative = source.saturating_sub(self.units.origin);
                chunk.visible.start
                    + chunk.first_unit_with_end_after(&self.units.data.storage, relative)
            }
            UnitChunk::Single(unit) => unit.visible.start,
        }
    }
}

fn inline_body(raw: &str, emphasis: text::Emphasis) -> Range<usize> {
    if emphasis.base() == text::Emphasis::Code && raw.starts_with('`') {
        let delimiter = raw.bytes().take_while(|byte| *byte == b'`').count();
        // ` `` + Enter ` 的自动完成态：四个反引号就是两侧各两个。
        if delimiter == 4 && raw == "````" {
            return 2..2;
        }
        if delimiter > 0 && raw.len() >= delimiter * 2 && raw.ends_with(&"`".repeat(delimiter)) {
            return delimiter..raw.len() - delimiter;
        }
    }
    if emphasis.base() == text::Emphasis::Link {
        if raw.starts_with("[[") && raw.ends_with("]]") {
            let begin = raw[..raw.len() - 2].rfind('|').map(|i| i + 1).unwrap_or(2);
            let body = &raw[begin..raw.len() - 2];
            let start = begin + body.len() - body.trim_start().len();
            return start..begin + body.trim_end().len();
        }
        if raw.starts_with('[') {
            return 1..raw.find("](").unwrap_or(raw.len());
        }
    }
    for marker in ["***", "___", "**", "__", "~~", "`", "*", "_"] {
        if emphasis != text::Emphasis::None
            && raw.len() >= marker.len() * 2
            && raw.starts_with(marker)
            && raw.ends_with(marker)
        {
            return marker.len()..raw.len() - marker.len();
        }
    }
    0..raw.len()
}

fn collect_wrappers(
    raw: &str,
    base: usize,
    code_closings: &text::CodeClosings,
    out: &mut Vec<WrapperData>,
    depth: usize,
) {
    if depth > 16 {
        return;
    }
    let mut i = 0;
    while i < raw.len() {
        if raw.as_bytes()[i] == b'\\' {
            i += 1;
            if i < raw.len() {
                i += raw[i..].chars().next().unwrap().len_utf8();
            }
            continue;
        }
        if raw.as_bytes()[i] == b'`' {
            if let Some(end) = inline_code_end(code_closings, raw, base, i) {
                i = end;
                continue;
            }
        }
        if let Some((begin, end, finish, _)) = crate::ui::text::markdown_span(&raw[i..]) {
            out.push(WrapperData {
                full: base + i..base + i + finish,
                body: base + i + begin..base + i + end,
            });
            collect_wrappers(
                &raw[i + begin..i + end],
                base + i + begin,
                code_closings,
                out,
                depth + 1,
            );
            i += finish;
        } else if let Some(span) = (raw.as_bytes()[i] == b'<')
            .then(|| crate::ui::styles::html_span(&raw[i..]))
            .flatten()
        {
            out.push(WrapperData {
                full: base + i..base + i + span.end,
                body: base + i + span.body_start..base + i + span.body_end,
            });
            collect_wrappers(
                &raw[i + span.body_start..i + span.body_end],
                base + i + span.body_start,
                code_closings,
                out,
                depth + 1,
            );
            i += span.end;
        } else {
            i += raw[i..].chars().next().unwrap().len_utf8();
        }
    }
    for span in text::parse_inline_spans(raw) {
        let slice = &raw[span.start..span.end];
        let body = inline_body(slice, span.run.emphasis);
        if body != (0..slice.len()) {
            out.push(WrapperData {
                full: base + span.start..base + span.end,
                body: base + span.start + body.start..base + span.start + body.end,
            });
        }
    }
}

/// 返回行内代码末尾（右侧围栏之后）的字节偏移。和 `text` 的解析规则保持
/// 同一套“同长度反引号配对”语义，防止代码里的 `**` 被误当成加粗。
pub(super) fn inline_code_end(
    closings: &text::CodeClosings,
    raw: &str,
    base: usize,
    start: usize,
) -> Option<usize> {
    let global_start = base + start;
    let limit = base + raw.len();
    let delimiter = closings.delimiter_at_before(global_start, limit)?;
    if let Some(global_end) = closings.closing_start_before(global_start, limit, delimiter) {
        return Some(global_end + delimiter - base);
    }
    // 保留富文本映射的历史回退行为：旧的 `find('`')?` 循环只有当
    // 索引中的最后一个 run 恰好结束于本切片末尾（或起始 run 本身
    // 就到达末尾）时才会走到这个分支，后面跟着普通文字时不会。
    (delimiter == 4
        && raw.get(start..start + delimiter) == Some("````")
        && closings.last_run_ends_at_before(global_start, limit))
    .then_some(start + delimiter)
}
