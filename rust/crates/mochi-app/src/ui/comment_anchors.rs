//! 兼容 Electron 版的 UTF-16 评论锚点，作用于编辑器可见文本。
use super::{
    document::{self, Block},
    rich::Mapping,
};
use mochi_core::sidecars::CommentAnchor;
use std::ops::Range;
#[cfg(test)]
struct TextBlock {
    mapping: Mapping,
    text: String,
    kind: &'static str,
    start: usize,
    end: usize,
}
#[cfg(test)]
fn blocks(source: &str) -> Vec<TextBlock> {
    let mut out = Vec::new();
    for rb in document::parse_ranged(source).blocks {
        let kind = match rb.block {
            Block::Heading { .. } => "heading",
            Block::Code { .. } => "codeBlock",
            _ => "paragraph",
        };
        let mapping = if matches!(rb.block, Block::Code { .. }) {
            let range = super::rich::code_body(&rb, source);
            Mapping::plain(source, range)
        } else {
            Mapping::block(&rb, source)
        };
        if mapping.units.is_empty() {
            continue;
        }
        let start = mapping.units.iter().next().unwrap().source.start;
        let end = mapping.units.iter().next_back().unwrap().source.end;
        let text = mapping.units.iter().map(|u| u.text.as_str()).collect();
        out.push(TextBlock {
            start,
            end,
            mapping,
            text,
            kind,
        });
    }
    out
}
fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}
fn byte_at(text: &str, offset: usize) -> usize {
    let mut n = 0;
    for (i, ch) in text.char_indices() {
        if n >= offset {
            return i;
        }
        n += ch.len_utf16();
    }
    text.len()
}
#[cfg(test)]
fn legacy_create(source: &str, range: Range<usize>, kind: &str) -> Option<CommentAnchor> {
    let blocks = blocks(source);
    let (index, block) = blocks
        .iter()
        .enumerate()
        .find(|(_, b)| b.start <= range.end && range.start <= b.end)?;
    let before = block
        .mapping
        .units
        .iter()
        .take_while(|u| u.source.end <= range.start)
        .map(|u| u.text.as_str())
        .collect::<String>();
    let selected = block
        .mapping
        .units
        .iter()
        .filter(|u| u.source.start < range.end && u.source.end > range.start)
        .map(|u| u.text.as_str())
        .collect::<String>();
    let start = utf16_len(&before);
    let end = start + utf16_len(&selected);
    Some(CommentAnchor {
        kind: kind.into(),
        selected_text: selected,
        block_text: block.text.clone(),
        block_type: block.kind.into(),
        block_index: index as i64,
        start_offset: start as i64,
        end_offset: end as i64,
        prefix: block.text
            [byte_at(&block.text, start.saturating_sub(32))..byte_at(&block.text, start)]
            .into(),
        suffix: block.text[byte_at(&block.text, end)..byte_at(&block.text, end + 32)].into(),
    })
}
#[cfg(test)]
fn dice(a: &str, b: &str) -> f64 {
    let normalize = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .encode_utf16()
            .collect::<Vec<_>>()
    };
    let (a, b) = (normalize(a), normalize(b));
    if a == b {
        return 1.0;
    }
    if a.len() < 2 || b.len() < 2 {
        return 0.0;
    }
    let mut pairs = std::collections::HashMap::new();
    for pair in a.windows(2) {
        *pairs.entry((pair[0], pair[1])).or_insert(0) += 1;
    }
    let mut hits = 0;
    for pair in b.windows(2) {
        if let Some(count) = pairs.get_mut(&(pair[0], pair[1])) {
            if *count > 0 {
                *count -= 1;
                hits += 1;
            }
        }
    }
    2.0 * hits as f64 / (a.len() + b.len() - 2) as f64
}
#[cfg(test)]
fn legacy_resolve(source: &str, anchor: &CommentAnchor) -> Option<Range<usize>> {
    if anchor.kind == "document" {
        return None;
    }
    let blocks = blocks(source);
    let (block, score) = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let score = dice(&anchor.block_text, &b.text)
                + if b.kind == anchor.block_type {
                    0.08
                } else {
                    0.0
                }
                + (0.06 - (i as i64 - anchor.block_index).abs() as f64 * 0.01).max(0.0)
                + if !anchor.selected_text.is_empty() && b.text.contains(&anchor.selected_text) {
                    0.25
                } else {
                    0.0
                };
            (b, score)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    if score < 0.18 {
        return None;
    }
    let selected = (!anchor.selected_text.is_empty())
        .then(|| block.text.find(&anchor.selected_text))
        .flatten();
    let start = selected
        .or_else(|| {
            (!anchor.prefix.is_empty())
                .then(|| {
                    block
                        .text
                        .find(&anchor.prefix)
                        .map(|i| i + anchor.prefix.len())
                })
                .flatten()
        })
        .unwrap_or_else(|| byte_at(&block.text, anchor.start_offset.max(0) as usize));
    let end = selected
        .map(|i| i + anchor.selected_text.len())
        .or_else(|| {
            (!anchor.suffix.is_empty())
                .then(|| block.text[start..].find(&anchor.suffix).map(|i| i + start))
                .flatten()
        })
        .unwrap_or_else(|| byte_at(&block.text, anchor.end_offset.max(0) as usize))
        .max(start);
    let mut offset = 0;
    let mut chosen = block.mapping.units.iter().filter(|u| {
        let from = offset;
        offset += u.text.as_str().len();
        from < end && offset > start
    });
    let first = chosen.next()?;
    let last = chosen.last().unwrap_or_else(|| first.clone());
    Some(first.source.start..last.source.end)
}
struct IndexedBlock {
    parsed_index: usize,
    text: std::cell::OnceCell<String>,
    kind: &'static str,
}

#[derive(Default)]
pub struct IndexCache {
    source: String,
    entries: Vec<IndexEntry>,
}

struct IndexEntry {
    range: Range<usize>,
    variant: std::mem::Discriminant<Block>,
    nonempty: bool,
}

/// 为一批锚点建一个轻量的可见文本索引。文档已有的解析结果直接借用；
/// 只有最终命中的块才需要字符映射。
pub struct Resolver<'a> {
    source: &'a str,
    parsed: &'a document::Parsed,
    blocks: Vec<IndexedBlock>,
    next_parsed: usize,
    cache: Option<&'a mut IndexCache>,
    next_entries: Vec<IndexEntry>,
}

fn block_kind(block: &Block) -> &'static str {
    match block {
        Block::Heading { .. } => "heading",
        Block::Code { .. } => "codeBlock",
        _ => "paragraph",
    }
}

fn mapping(block: &document::RangedBlock, source: &str) -> Mapping {
    if matches!(block.block, Block::Code { .. }) {
        Mapping::plain(source, super::rich::code_body(block, source))
    } else {
        Mapping::block(block, source)
    }
}

fn normalize(text: &str) -> Vec<u16> {
    let mut units = Vec::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !units.is_empty() {
            units.push(b' ' as u16);
        }
        units.extend(word.encode_utf16());
    }
    units
}

struct Dice {
    units: Vec<u16>,
    pairs: std::collections::HashMap<(u16, u16), usize>,
}

impl Dice {
    fn new(text: &str) -> Self {
        let units = normalize(text);
        let mut pairs = std::collections::HashMap::new();
        for pair in units.windows(2) {
            *pairs.entry((pair[0], pair[1])).or_insert(0) += 1;
        }
        Self { units, pairs }
    }

    fn score(&self, text: &str) -> f64 {
        let units = normalize(text);
        if self.units == units {
            return 1.0;
        }
        if self.units.len() < 2 || units.len() < 2 {
            return 0.0;
        }
        let mut used = std::collections::HashMap::new();
        let mut hits = 0;
        for pair in units.windows(2) {
            let key = (pair[0], pair[1]);
            if let Some(&limit) = self.pairs.get(&key) {
                let count = used.entry(key).or_insert(0usize);
                if *count < limit {
                    *count += 1;
                    hits += 1;
                }
            }
        }
        2.0 * hits as f64 / (self.units.len() + units.len() - 2) as f64
    }
}

impl<'a> Resolver<'a> {
    pub fn new(source: &'a str, parsed: &'a document::Parsed) -> Self {
        Self {
            source,
            parsed,
            blocks: Vec::new(),
            next_parsed: 0,
            cache: None,
            next_entries: Vec::new(),
        }
    }

    pub fn with_cache(
        source: &'a str,
        parsed: &'a document::Parsed,
        cache: &'a mut IndexCache,
    ) -> Self {
        let mut resolver = Self::new(source, parsed);
        resolver.cache = Some(cache);
        resolver
    }

    fn fill_to(&mut self, count: usize) {
        while self.blocks.len() < count && self.next_parsed < self.parsed.blocks.len() {
            let parsed_index = self.next_parsed;
            self.next_parsed += 1;
            let block = &self.parsed.blocks[parsed_index];
            let text = std::cell::OnceCell::new();
            let variant = std::mem::discriminant(&block.block);
            let reused = self.cache.as_ref().and_then(|cache| {
                let entry = cache.entries.get(parsed_index)?;
                (entry.variant == variant
                    && cache.source[entry.range.clone()] == self.source[block.start..block.end])
                    .then_some(entry.nonempty)
            });
            let nonempty = reused.unwrap_or_else(|| {
                Mapping::obvious_block_text(block, self.source).unwrap_or_else(|| {
                    !text
                        .get_or_init(|| Mapping::block_text(block, self.source))
                        .is_empty()
                })
            });
            if self.cache.is_some() {
                self.next_entries.push(IndexEntry {
                    range: block.start..block.end,
                    variant,
                    nonempty,
                });
            }
            if nonempty {
                self.blocks.push(IndexedBlock {
                    parsed_index,
                    text,
                    kind: block_kind(&block.block),
                });
            }
        }
    }

    fn text<'b>(&self, block: &'b IndexedBlock) -> &'b str {
        block.text.get_or_init(|| {
            Mapping::block_text(&self.parsed.blocks[block.parsed_index], self.source)
        })
    }

    pub fn resolve(&mut self, anchor: &CommentAnchor) -> Option<Range<usize>> {
        if anchor.kind == "document" {
            return None;
        }
        let dice = Dice::new(&anchor.block_text);
        if let Ok(index) = usize::try_from(anchor.block_index) {
            self.fill_to(index.saturating_add(1));
            if let Some(block) = self.blocks.get(index) {
                let text = self.text(block);
                // 只有这个原始序号能拿到完整的邻近加分。其余各项都到上限
                // 时，后面未见过的块不可能追平或超过它。这种常见情形下
                // 连剩余的过滤索引都不用建。
                if block.kind == anchor.block_type
                    && (anchor.selected_text.is_empty() || text.contains(&anchor.selected_text))
                    && dice.score(text) == 1.0
                {
                    return self.range(index, anchor);
                }
            }
        }
        self.fill_to(usize::MAX);
        if self.blocks.is_empty() {
            return None;
        }
        let preferred = usize::try_from(anchor.block_index)
            .ok()
            .filter(|&i| i < self.blocks.len())
            .unwrap_or(0);
        let mut best: Option<(usize, f64)> = None;
        let possible_selected: Vec<bool> = self
            .blocks
            .iter()
            .map(|block| {
                let rb = &self.parsed.blocks[block.parsed_index];
                !anchor.selected_text.is_empty()
                    && is_subsequence(&self.source[rb.start..rb.end], &anchor.selected_text)
            })
            .collect();
        // 附近未变化的锚点往往能拿到满分。分数边界只用来剪枝，绝不影响
        // 候选内容或全局平局规则。先试带选中文本的候选，再试其余的——
        // 否则过期的存储序号会先和整个前缀做昂贵的模糊比较，很晚才轮到
        // 文件后面那个未变化的锚点。
        for index in std::iter::once(preferred)
            .chain((0..self.blocks.len()).filter(|&i| i != preferred && possible_selected[i]))
            .chain((0..self.blocks.len()).filter(|&i| i != preferred && !possible_selected[i]))
        {
            let block = &self.blocks[index];
            let kind = if block.kind == anchor.block_type {
                0.08
            } else {
                0.0
            };
            let distance = (index as i128 - i128::from(anchor.block_index)).unsigned_abs();
            let nearby = (0.06 - distance as f64 * 0.01).max(0.0);
            // Mapping 的可见单元按顺序保留源字符，公式的 TeX 也不例外。
            // 因此可见文本匹配必然也是源文本的子序列。这条上界还能容许
            // "狂**热**" 这类被格式标记拆开的写法，而纯子串判断做不到。
            let upper = 1.0 + kind + nearby + if possible_selected[index] { 0.25 } else { 0.0 };
            if best.is_some_and(|(old_index, score)| {
                upper < score || (upper == score && index < old_index)
            }) {
                continue;
            }
            let text = self.text(block);
            let selected =
                if !anchor.selected_text.is_empty() && text.contains(&anchor.selected_text) {
                    0.25
                } else {
                    0.0
                };
            let upper = 1.0 + kind + nearby + selected;
            if best.is_some_and(|(old_index, score)| {
                upper < score || (upper == score && index < old_index)
            }) {
                continue;
            }
            let score = dice.score(text) + kind + nearby + selected;
            if best.is_none_or(|(old_index, old_score)| {
                score > old_score || (score == old_score && index > old_index)
            }) {
                best = Some((index, score));
            }
        }
        let (index, score) = best?;
        if score < 0.18 {
            return None;
        }
        self.range(index, anchor)
    }

    fn range(&self, index: usize, anchor: &CommentAnchor) -> Option<Range<usize>> {
        let block = &self.blocks[index];
        let text = self.text(block);
        let selected = (!anchor.selected_text.is_empty())
            .then(|| text.find(&anchor.selected_text))
            .flatten();
        let start = selected
            .or_else(|| {
                (!anchor.prefix.is_empty())
                    .then(|| text.find(&anchor.prefix).map(|i| i + anchor.prefix.len()))
                    .flatten()
            })
            .unwrap_or_else(|| byte_at(text, anchor.start_offset.max(0) as usize));
        let end = selected
            .map(|i| i + anchor.selected_text.len())
            .or_else(|| {
                (!anchor.suffix.is_empty())
                    .then(|| text[start..].find(&anchor.suffix).map(|i| i + start))
                    .flatten()
            })
            .unwrap_or_else(|| byte_at(text, anchor.end_offset.max(0) as usize))
            .max(start);
        let mapping = mapping(&self.parsed.blocks[block.parsed_index], self.source);
        let mut offset = 0;
        let mut chosen = mapping.units.iter().filter(|unit| {
            let from = offset;
            offset += unit.text.as_str().len();
            from < end && offset > start
        });
        let first = chosen.next()?;
        let last = chosen.last().unwrap_or_else(|| first.clone());
        Some(first.source.start..last.source.end)
    }
}

impl Drop for Resolver<'_> {
    fn drop(&mut self) {
        if self.next_parsed == 0 {
            return;
        }
        if let Some(cache) = &mut self.cache {
            // 复用容量，只保留索引成员资格，不留下过期的可见文本或几何
            // 信息。编辑或结构性解析之后，每条复用项都要靠源码字节完全
            // 一致且块变体相同来校验。
            cache.source.clear();
            cache.source.push_str(self.source);
            cache.entries = std::mem::take(&mut self.next_entries);
        }
    }
}

fn is_subsequence(mut source: &str, visible: &str) -> bool {
    for ch in visible.chars() {
        let Some(offset) = source.find(ch) else {
            return false;
        };
        source = &source[offset + ch.len_utf8()..];
    }
    true
}

pub fn resolve(source: &str, anchor: &CommentAnchor) -> Option<Range<usize>> {
    Resolver::new(source, &document::parse_ranged(source)).resolve(anchor)
}

pub fn create(source: &str, range: Range<usize>, kind: &str) -> Option<CommentAnchor> {
    let parsed = document::parse_ranged(source);
    let mut resolver = Resolver::new(source, &parsed);
    resolver.fill_to(usize::MAX);
    for (index, block) in resolver.blocks.iter().enumerate() {
        let rb = &parsed.blocks[block.parsed_index];
        if rb.end < range.start || rb.start > range.end {
            continue;
        }
        let mapping = mapping(rb, source);
        let Some(first) = mapping.units.first() else {
            continue;
        };
        let Some(last) = mapping.units.last() else {
            continue;
        };
        if first.source.start > range.end || last.source.end < range.start {
            continue;
        }
        let before: String = mapping
            .units
            .iter()
            .take_while(|u| u.source.end <= range.start)
            .map(|u| u.text.as_str())
            .collect();
        let selected: String = mapping
            .units
            .iter()
            .filter(|u| u.source.start < range.end && u.source.end > range.start)
            .map(|u| u.text.as_str())
            .collect();
        let start = utf16_len(&before);
        let end = start + utf16_len(&selected);
        let text = resolver.text(block);
        return Some(CommentAnchor {
            kind: kind.into(),
            selected_text: selected,
            block_text: text.to_owned(),
            block_type: block.kind.into(),
            block_index: index as i64,
            start_offset: start as i64,
            end_offset: end as i64,
            prefix: text[byte_at(text, start.saturating_sub(32))..byte_at(text, start)].into(),
            suffix: text[byte_at(text, end)..byte_at(text, end + 32)].into(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "set MOCHI_COMMENT_BENCHMARK_FILE to a document with a comment sidecar"]
    fn benchmark_real_comment_index_stages() {
        let path = std::env::var("MOCHI_COMMENT_BENCHMARK_FILE").unwrap();
        let mut source = std::fs::read_to_string(&path).unwrap();
        let comments = mochi_core::sidecars::load_comments(&path).comments;
        let anchor = comments.iter().find_map(|c| c.anchor.as_ref()).unwrap();
        let mut cache = IndexCache::default();
        for iteration in 0..3 {
            let parsed = document::parse_ranged(&source);
            let start = std::time::Instant::now();
            let mut resolver = Resolver::with_cache(&source, &parsed, &mut cache);
            resolver.fill_to(usize::MAX);
            let indexed = start.elapsed();
            let result = resolver.resolve(anchor);
            let resolved = start.elapsed();
            let materialized = resolver
                .blocks
                .iter()
                .filter(|b| b.text.get().is_some())
                .count();
            drop(resolver);
            eprintln!("iteration={iteration} index={indexed:?} resolve={:?} store={:?} visible_blocks={materialized} range={result:?}", resolved - indexed, start.elapsed() - resolved);
            source.insert(0, 'a');
        }
    }

    #[test]
    fn unchanged_anchor_only_materializes_winning_block_text() {
        let mut source = String::from("狂**热**\n\n");
        for index in 0..5000 {
            source.push_str(&format!("ordinary paragraph {index}\n\n"));
        }
        let anchor = legacy_create(&source, 0.."狂**热**".len(), "text").unwrap();
        assert_eq!(anchor.selected_text, "狂热");
        let parsed = document::parse_ranged(&source);
        let mut resolver = Resolver::new(&source, &parsed);
        assert!(resolver
            .blocks
            .iter()
            .all(|block| block.text.get().is_none()));
        assert_eq!(resolver.resolve(&anchor), legacy_resolve(&source, &anchor));
        assert_eq!(
            resolver.blocks.len(),
            1,
            "do not index the untouched suffix"
        );
        assert_eq!(
            resolver
                .blocks
                .iter()
                .filter(|block| block.text.get().is_some())
                .count(),
            1
        );
        let moved = format!("前置\n\n{source}");
        assert_eq!(resolve(&moved, &anchor), legacy_resolve(&moved, &anchor));
    }

    #[test]
    fn stale_anchor_ordinal_prioritizes_selected_text_and_preserves_global_ties() {
        let original = "n. 狂**热**";
        let anchor = legacy_create(original, 3..original.len(), "text").unwrap();
        let mut source = String::new();
        for index in 0..5000 {
            source.push_str(&format!("ordinary paragraph {index}\n\n"));
        }
        source.push_str(original);
        source.push_str("\n\n");
        source.push_str(original);
        let parsed = document::parse_ranged(&source);
        let mut resolver = Resolver::new(&source, &parsed);
        let resolved = resolver.resolve(&anchor);
        assert_eq!(resolved, legacy_resolve(&source, &anchor));
        assert!(resolved.unwrap().start > source.rfind(original).unwrap());
        assert_eq!(
            resolver
                .blocks
                .iter()
                .filter(|block| block.text.get().is_some())
                .count(),
            3,
            "only the stored ordinal and two possible matches need inline parsing"
        );
    }

    #[test]
    fn nonempty_shortcuts_preserve_filtered_block_ordinals() {
        let cases = [
            "",
            " ",
            "中文",
            "a",
            "😀",
            "&amp;",
            "&#65;",
            "<span></span>",
            "<b> </b>",
            "[ ](url)",
            "[[target|]]",
            "$x$",
            "**",
            "---",
            "\\*",
            "a\nb",
        ];
        for body in cases {
            for (open, close) in [
                ("", ""),
                ("**", "**"),
                ("_", "_"),
                ("~~", "~~"),
                ("`", "`"),
                ("==", "=="),
                ("<span>", "</span>"),
                ("[", "](url)"),
                ("$", "$"),
            ] {
                for prefix in ["", "# ", "- ", "> "] {
                    let source = format!("{prefix}{open}{body}{close}\n\nfollowing\n");
                    let parsed = document::parse_ranged(&source);
                    let mut resolver = Resolver::new(&source, &parsed);
                    resolver.fill_to(usize::MAX);
                    let reference = blocks(&source);
                    assert_eq!(resolver.blocks.len(), reference.len(), "{source:?}");
                    for (new, old) in resolver.blocks.iter().zip(&reference) {
                        assert_eq!(resolver.text(new), old.text, "{source:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn lightweight_text_and_fuzzy_resolution_match_character_mapping_reference() {
        let source = "# 标题😀\n\n前文 **粗体** _斜体_ `代码` \\*转义\\*\n- 列表 [标签](https://example.com)\n> 引用 [[目标|别名]]\n\n<span style=\"color: red\">红色</span> $x^2$ &amp; 中文\n\n```rust\nlet 中文 = 1;\n```\n\n| A | B |\n| - | - |\n| 甲 | 乙 |\n\n---\n\n重复目标\n\n重复目标\n";
        let parsed = document::parse_ranged(source);
        let mut indexed = Resolver::new(source, &parsed);
        indexed.fill_to(usize::MAX);
        let reference = blocks(source);
        assert_eq!(indexed.blocks.len(), reference.len());
        for (new, old) in indexed.blocks.iter().zip(&reference) {
            assert_eq!(indexed.text(new), old.text);
            assert_eq!(new.kind, old.kind);
            let selected = old.start..old.end;
            assert_eq!(
                create(source, selected.clone(), "text"),
                legacy_create(source, selected, "text")
            );
        }
        for selected in [
            "粗体",
            "标签",
            "别名",
            "红色",
            "x^2",
            "let 中文",
            "重复目标",
        ] {
            let start = source.find(selected).unwrap();
            let anchor = legacy_create(source, start..start + selected.len(), "text").unwrap();
            let mut cache = IndexCache::default();
            for changed in [
                source.to_owned(),
                format!("前置新段落\n\n{source}"),
                source.replace("**", ""),
                source.replace(selected, "替换😀"),
                source.replace('\n', "\r\n"),
            ] {
                for index in [-5, anchor.block_index, 500] {
                    let mut anchor = anchor.clone();
                    anchor.block_index = index;
                    assert_eq!(
                        resolve(&changed, &anchor),
                        legacy_resolve(&changed, &anchor),
                        "{selected}, index={index}"
                    );
                    let parsed = document::parse_ranged(&changed);
                    assert_eq!(
                        Resolver::with_cache(&changed, &parsed, &mut cache).resolve(&anchor),
                        legacy_resolve(&changed, &anchor),
                        "cached {selected}, index={index}"
                    );
                }
            }
        }
    }

    #[test]
    fn prepared_dice_preserves_whitespace_unicode_and_multiset_counts() {
        let cases = [
            "",
            " ",
            "a",
            "aa",
            "aaaa",
            "abab",
            "a  b\n\tc",
            "😀😀中",
            "狂热",
            "狂热 狂热",
        ];
        for a in cases {
            for b in cases {
                assert_eq!(Dice::new(a).score(b), dice(a, b), "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn utf16_anchors_follow_unicode_text_when_blocks_move_or_format_changes() {
        let source = "# 标题\n\n前文😀**选中的文字**后文";
        let start = source.find("选中的文字").unwrap();
        let anchor = create(source, start..start + "选中的文字".len(), "text").unwrap();
        assert_eq!(anchor.start_offset, 4);
        assert_eq!(anchor.selected_text, "选中的文字");
        for changed in [
            source.to_owned(),
            format!("新段落\n{source}"),
            source.replace("**", ""),
        ] {
            let range = resolve(&changed, &anchor).unwrap();
            assert_eq!(&changed[range], "选中的文字");
        }
    }
}
