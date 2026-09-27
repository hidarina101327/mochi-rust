//! 原生所见即所得的几何层。渲染出的字形保留源码区间，
//! 因此可以做选区和输入，而无需暴露 Markdown 语法或改写文件。
//! 可选的「当前块源码」偏好也走同一条布局路径。

use super::document::{self, Block, Layout, Parsed};
use super::draw::DrawList;
use super::editor::TextBuffer;
use super::layout::Rect;
use super::text;
#[cfg(test)]
use super::theme;
use super::theme::Palette;
use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub const CARET_WIDTH: f32 = 2.0;

/// 一次完整排版：解析结果 + 视觉行。按 (内容, 光标所在块, 宽度) 缓存。
#[derive(Debug, Clone, Default)]
pub struct LiveLayout {
    pub parsed: Parsed,
    pub layout: Layout,
    // 几何信息每个块都有，但源码/字形映射只有被选区、光标或输入
    // 碰到的块才需要。为每个字符都急切建一个单元，大文档的内存会翻倍。
    maps: Vec<OnceCell<super::rich::Mapping>>,
    source: Rc<str>,
    mapping_cache: MappingCache,
}

/// 未变行内块的源码映射的有界缓存。建映射要在布局之后把行内语法
/// 再扫一遍；按块的精确源码保留缓存，编辑只动了别的块时就省下
/// 这份工作。块在文档中移动时，区间随之平移。
#[derive(Debug, Clone, Default)]
pub struct MappingCache(Rc<RefCell<MappingStore>>);

#[derive(Debug, Default)]
struct MappingStore {
    entries: HashMap<u64, Vec<MappingEntry>>,
    count: usize,
    bytes: usize,
    #[cfg(test)]
    hits: usize,
    #[cfg(test)]
    misses: usize,
}

#[derive(Debug, Clone)]
struct MappingEntry {
    source: Rc<str>,
    start: usize,
    kind: u8,
    mapping: super::rich::Mapping,
}

const MAX_MAPPING_ENTRIES: usize = 8192;
const MAX_MAPPING_BYTES: usize = 16 * 1024 * 1024;

impl MappingCache {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    fn get(&self, rb: &document::RangedBlock, source: &str) -> super::rich::Mapping {
        self.0.borrow_mut().get(rb, source)
    }

    #[cfg(test)]
    fn stats(&self) -> (usize, usize, usize) {
        let store = self.0.borrow();
        (store.count, store.hits, store.misses)
    }
}

impl MappingStore {
    fn get(&mut self, rb: &document::RangedBlock, source: &str) -> super::rich::Mapping {
        let Some(kind) = mapping_kind(&rb.block) else {
            return super::rich::Mapping::block(rb, source);
        };
        let raw = source.get(rb.start..rb.end).unwrap_or("");
        let hash = mapping_hash(raw, kind);
        if let Some(entry) = self.entries.get(&hash).and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry.kind == kind && entry.source.as_ref() == raw)
        }) {
            let mut mapping = entry.mapping.clone();
            let delta = rb.start as isize - entry.start as isize;
            if delta != 0 {
                mapping.rebase(delta);
            }
            #[cfg(test)]
            {
                self.hits += 1;
            }
            return mapping;
        }
        #[cfg(test)]
        {
            self.misses += 1;
        }
        // 映射和缓存键共用这一份不可变的原始块。映射的普通单元指向它；
        // 只有公式单元另持一份共享字符串。
        let raw_storage: Rc<str> = Rc::from(raw);
        let mapping = super::rich::Mapping::block_with_storage(rb, source, raw_storage.clone());
        let bytes = mapping
            .memory_bytes()
            .saturating_add(std::mem::size_of::<MappingEntry>());
        // 绝不为了放进一个新条目而驱逐还活着的条目。长文档的映射数据
        // 合理地超过有界缓存很正常：顺序遍历一遍时如果驱逐最老的条目，
        // 下一次编辑按同样顺序走一遍文档，一边重建前缀一边驱逐后缀，
        // 然后又 miss 后缀（整库抖动）。准入在硬上限处停止，
        // 已付过成本的全部保留；未缓存的块照样正确布局，
        // 之后编辑文档还能复用留下的这批。
        if bytes <= MAX_MAPPING_BYTES
            && self.count < MAX_MAPPING_ENTRIES
            && self.bytes.saturating_add(bytes) <= MAX_MAPPING_BYTES
        {
            self.entries.entry(hash).or_default().push(MappingEntry {
                source: raw_storage,
                start: rb.start,
                kind,
                mapping: mapping.clone(),
            });
            self.count += 1;
            self.bytes += bytes;
        }
        mapping
    }
}

fn mapping_kind(block: &Block) -> Option<u8> {
    Some(match block {
        Block::Paragraph(_) => 0,
        Block::Heading { .. } => 1,
        Block::ListItem { .. } => 2,
        Block::Quote(_) => 3,
        Block::Aligned { .. } => 4,
        _ => return None,
    })
}

fn mapping_hash(source: &str, kind: u8) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut hasher);
    source.hash(&mut hasher);
    hasher.finish()
}

pub fn layout(
    source: &str,
    cursor: Option<usize>,
    width: f32,
    images: document::ImageSizer,
) -> LiveLayout {
    let parsed = document::parse_ranged(source);
    layout_parsed(source, parsed, cursor, width, images, None, None)
}

/// 布局一篇已解析的文档，并复用调用方给出的有界本地布局缓存。
/// 把解析放在这个函数之外对编辑路径很重要：块查找、图片发现、
/// 代码卡对账和布局都需要同一份 `Parsed`。
pub fn layout_cached(
    source: &str,
    parsed: document::Parsed,
    cursor: Option<usize>,
    width: f32,
    images: document::ImageSizer,
    cache: &mut document::LayoutCache,
    mappings: &mut MappingCache,
) -> LiveLayout {
    layout_parsed(
        source,
        parsed,
        cursor,
        width,
        images,
        Some(cache),
        Some(mappings),
    )
}

fn layout_parsed(
    source: &str,
    parsed: document::Parsed,
    cursor: Option<usize>,
    width: f32,
    images: document::ImageSizer,
    cache: Option<&mut document::LayoutCache>,
    mappings: Option<&mut MappingCache>,
) -> LiveLayout {
    let live_line_source = super::editor_preferences::current().live_line_source;
    // 每个可编辑空块都已有精确的源码行。只有用户显式开启「源码行」
    // 偏好时，光标移动才会改变几何。
    let active = cursor
        .filter(|_| live_line_source)
        .and_then(|c| parsed.block_at(c));
    let layout = match cache {
        Some(cache) => {
            document::layout_editor_cached(&parsed.blocks, source, active, width, images, cache)
        }
        None => document::layout_editor(&parsed.blocks, source, active, width, images),
    };
    let maps = parsed.blocks.iter().map(|_| OnceCell::new()).collect();
    LiveLayout {
        parsed,
        layout,
        maps,
        source: Rc::from(source),
        mapping_cache: mappings.as_deref().cloned().unwrap_or_default(),
    }
}

/// 表格单元格按表格自身字号使用同一套富文本源码映射。
pub fn inline_layout(source: &str, width: f32, style: super::draw::TextStyle) -> LiveLayout {
    let block = document::RangedBlock {
        block: Block::Paragraph(source.into()),
        start: 0,
        end: source.len(),
    };
    let map = super::rich::Mapping::inline(source, 0..source.len());
    let mut y = 0.0;
    let lines = super::math_runs::wrap_mapped(&text::parse_inline(source), style, width)
        .into_iter()
        .map(|(runs, visible_start)| {
            let height = text::runs_height(&runs, style);
            let line = document::LaidOutLine {
                runs,
                style,
                x: 0.0,
                y,
                height,
                decoration: document::Decoration::None,
                block: 0,
                source: None,
                visible_start,
                tokens: Vec::new(),
            };
            y += height;
            line
        })
        .collect();
    LiveLayout {
        parsed: Parsed {
            blocks: vec![block],
            body_start: 0,
        },
        layout: Layout {
            lines,
            height: y,
            headings: Vec::new(),
            table_widths: Default::default(),
        },
        maps: vec![OnceCell::from(map)],
        source: Rc::from(source),
        mapping_cache: MappingCache::default(),
    }
}

impl LiveLayout {
    pub fn clear_view(&mut self, blocks: std::ops::Range<usize>) {
        for mapping in &mut self.maps[blocks] {
            mapping.take();
        }
        self.layout = Layout::default();
    }
    pub fn pending(source: &str, parsed: Parsed, mappings: &MappingCache) -> Self {
        let maps = parsed.blocks.iter().map(|_| OnceCell::new()).collect();
        Self {
            parsed,
            layout: Layout::default(),
            maps,
            source: Rc::from(source),
            mapping_cache: mappings.clone(),
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// 只对单个物理行内块重排，不重新解析、不重新成形邻居。
    /// 结构性编辑刻意走完整的解析路径；这条快速通道绝不允许
    /// 跨 Markdown 块边界乱猜。
    pub fn update_inline(&mut self, source: &str, width: f32) -> bool {
        let before = self.source.as_ref();
        if before == source || super::editor_preferences::current().live_line_source {
            return false;
        }
        let (prefix, before_end, _) = super::editor::changed_bounds(before, source);
        let Some(bi) = self.parsed.block_at(prefix) else {
            return false;
        };
        let rb = &self.parsed.blocks[bi];
        if before_end > rb.end
            || self
                .parsed
                .blocks
                .iter()
                .any(|rb| matches!(rb.block, Block::Container(_)))
            || !matches!(rb.block, Block::Paragraph(_) | Block::Heading { .. })
        {
            return false;
        }
        let delta = source.len() as isize - before.len() as isize;
        let end = rb.end.saturating_add_signed(delta);
        let Some(raw) = source.get(rb.start..end) else {
            return false;
        };
        let old_raw = &before[rb.start..rb.end];
        // 表格、HTML 和围栏可能改变邻居的解读方式——哪怕单独解析这一行
        // 结果仍是个段落。
        if [raw, old_raw].iter().any(|raw| {
            raw.contains(['\n', '\r', '|', '<'])
                || raw.trim_start().starts_with(":::")
                || raw.trim_start().starts_with("$$")
                || document::fence_open(raw.trim_start()).is_some()
        }) {
            return false;
        }
        let mut next = document::parse_ranged(raw);
        if next.blocks.len() != 1 || next.body_start != 0 {
            return false;
        }
        let next_block = next.blocks.remove(0).block;
        if !match (&rb.block, &next_block) {
            (Block::Paragraph(_), Block::Paragraph(_)) => true,
            (Block::Heading { level: a, .. }, Block::Heading { level: b, .. }) => a == b,
            _ => false,
        } {
            return false;
        }
        let replacement = document::RangedBlock {
            block: next_block,
            start: rb.start,
            end,
        };
        let range = self.layout.block_line_range(bi);
        if range.is_empty() {
            return false;
        }
        let mut local = document::layout_editor(
            std::slice::from_ref(&replacement),
            source,
            None,
            width,
            &|_| None,
        );
        let Some(first) = local.lines.first() else {
            return false;
        };
        let y_offset = self.layout.lines[range.start].y - first.y;
        let old_bottom =
            self.layout.lines[range.end - 1].y + self.layout.lines[range.end - 1].height;
        for line in &mut local.lines {
            line.block = bi;
            line.y += y_offset;
        }
        let last = local.lines.last().unwrap();
        let height_delta = last.y + last.height - old_bottom;
        for line in &mut self.layout.lines[range.end..] {
            line.y += height_delta;
            if let Some((start, end)) = &mut line.source {
                *start = start.saturating_add_signed(delta);
                *end = end.saturating_add_signed(delta);
            }
        }
        let first_rendered = self.layout.lines.first().map_or(0, |line| line.block);
        let heading_index = self.parsed.blocks[first_rendered.min(bi)..bi]
            .iter()
            .filter(|rb| {
                matches!(
                    rb.block,
                    Block::Heading { .. } | Block::Aligned { level: 1..=6, .. }
                )
            })
            .count();
        let is_heading = matches!(replacement.block, Block::Heading { .. });
        if is_heading {
            let mut heading = local.headings.remove(0);
            heading.y += y_offset;
            self.layout.headings[heading_index] = heading;
        }
        for heading in &mut self.layout.headings[heading_index + usize::from(is_heading)..] {
            heading.y += height_delta;
        }
        self.layout.height += height_delta;
        self.layout.lines.splice(range, local.lines);
        self.parsed.blocks[bi] = replacement;
        for block in &mut self.parsed.blocks[bi + 1..] {
            block.start = block.start.saturating_add_signed(delta);
            block.end = block.end.saturating_add_signed(delta);
        }
        self.maps[bi].take();
        for mapping in &mut self.maps[bi + 1..] {
            if let Some(mapping) = mapping.get_mut() {
                mapping.rebase(delta);
            }
        }
        self.source = Rc::from(source);
        true
    }

    fn mapping(&self, block: usize) -> &super::rich::Mapping {
        self.maps[block].get_or_init(|| {
            self.mapping_cache
                .get(&self.parsed.blocks[block], &self.source)
        })
    }

    /// 光标可停留的最小偏移：文档头部之后。
    pub fn min_offset(&self) -> usize {
        self.parsed.body_start
    }

    fn line_of_offset(&self, offset: usize) -> Option<usize> {
        // 精确行优先。范围含右端；软换行处两行共享一个边界点，归**前一行**——
        // 在软换行处按 End 光标不该跳到下一行开头
        let block = self.parsed.block_at(offset)?;
        let range = self.layout.block_line_range(block);
        self.layout.lines[range.clone()]
            .iter()
            .position(|l| matches!(l.source, Some((s, e)) if s <= offset && offset <= e))
            .map(|index| range.start + index)
    }

    /// 光标偏移 → (视觉行下标, 行内 x)。光标不在任何精确行上（理论上不会：活动块总是
    /// 原样显示）时退回到所属块的第一行行首。
    pub fn locate(&self, offset: usize) -> Option<(usize, f32)> {
        if let Some(i) = self.line_of_offset(offset) {
            let line = &self.layout.lines[i];
            let (s, _) = line.source.unwrap();
            let full: String = line.runs.iter().map(|r| r.text.as_str()).collect();
            let local = (offset - s).min(full.len());
            if let Some(shape) = super::measurement::shape(&full, line.style, text::Emphasis::None)
            {
                return Some((i, shape.caret(local)));
            }
            let prefix = &full[..floor_char_boundary(&full, local)];
            return Some((i, text::measure(prefix, line.style)));
        }
        let bi = self.parsed.block_at(offset)?;
        let visible = self.mapping(bi).visible_at(offset);
        let range = self.layout.block_line_range(bi);
        let i = self.layout.lines[range.clone()]
            .iter()
            .position(|l| {
                l.block == bi
                    && l.visible_start != usize::MAX
                    && l.visible_start <= visible
                    && visible
                        <= l.visible_start + l.runs.iter().map(text::visible_len).sum::<usize>()
            })
            .or_else(|| (!range.is_empty()).then_some(0))?
            + range.start;
        Some((
            i,
            run_caret(
                &self.layout.lines[i],
                visible.saturating_sub(self.layout.lines[i].visible_start),
            ),
        ))
    }

    /// 光标矩形（相对文档坐标，未加 area 与滚动）。
    pub fn caret(&self, offset: usize) -> Option<Rect> {
        let (i, x) = self.locate(offset)?;
        let line = &self.layout.lines[i];
        Some(Rect::new(
            line.x + x,
            line.y,
            line.x + x + CARET_WIDTH,
            line.y + line.height,
        ))
    }

    /// y 落在第几行。行之间的空隙归上一行；顶部之上归第一行、底部之下归最后一行。
    pub fn line_at(&self, y: f32) -> Option<usize> {
        let lines = &self.layout.lines;
        if lines.is_empty() {
            return None;
        }
        Some(lines.partition_point(|line| line.y <= y).saturating_sub(1))
    }

    /// 点击 (x, y) → 源码偏移。
    pub fn hit(&self, source: &str, x: f32, y: f32) -> Option<usize> {
        let i = self.line_at(y)?;
        Some(self.offset_in_line(source, i, x))
    }

    /// 第 `i` 行上横坐标 `x`（相对内容区左缘）对应的源码偏移。
    pub fn offset_in_line(&self, source: &str, i: usize, x: f32) -> usize {
        let line = &self.layout.lines[i];
        let local_x = (x - line.x).max(0.0);
        // 走到 x 处是第几个字符（过了字符中线算右边）
        let mut walked = 0.0;
        let mut chars = 0usize;
        let mut bytes = 0usize;
        'runs: for run in &line.runs {
            if line.source.is_none() && run.emphasis.base() == text::Emphasis::Math {
                let width = text::run_width(run, line.style);
                if local_x < walked + width / 2.0 {
                    break;
                }
                walked += width;
                chars += text::visible_len(run);
                bytes += run.text.len();
                continue;
            }
            if let Some(shape) = super::measurement::shape(&run.text, line.style, run.emphasis) {
                if local_x <= walked + shape.width {
                    let byte = shape.hit(local_x - walked);
                    bytes += byte;
                    chars += run.text[..byte].chars().count();
                    break;
                }
                walked += shape.width;
                bytes += run.text.len();
                chars += run.text.chars().count();
                continue;
            }
            for ch in run.text.chars() {
                let w = text::advance_for(ch, line.style, run.emphasis) * line.style.font_size();
                if local_x < walked + w / 2.0 {
                    break 'runs;
                }
                walked += w;
                chars += 1;
                bytes += ch.len_utf8();
            }
        }
        if let Some((s, _)) = line.source {
            return s + bytes;
        }
        // 渲染行：可见字符序号 → 源码偏移
        let rb = &self.parsed.blocks[line.block];
        let block_src = source.get(rb.start..rb.end).unwrap_or("");
        if line.visible_start == usize::MAX {
            // 项目符号：落到文字开头
            return rb.start + content_start(&rb.block, block_src);
        }
        self.mapping(line.block)
            .source_at(line.visible_start + chars)
    }

    /// 上下移动。`desired_x` 是跨多次移动保持的目标横坐标（相对内容区左缘）。
    pub fn move_vertical(&self, source: &str, offset: usize, delta: i32, desired_x: f32) -> usize {
        let Some((i, _)) = self.locate(offset) else {
            return offset;
        };
        let n = self.layout.lines.len() as i32;
        let mut target = i as i32 + delta;
        // 跳过没有文字的装饰行（分隔线）
        while target >= 0
            && target < n
            && self.layout.lines[target as usize].source.is_none()
            && (self.layout.lines[target as usize].runs.is_empty()
                || self.layout.lines[target as usize].visible_start == usize::MAX)
        {
            target += delta.signum();
        }
        if target < 0 {
            return self.line_bounds(offset).0.min(offset);
        }
        if target >= n {
            return self.line_bounds(offset).1.max(offset);
        }
        self.offset_in_line(source, target as usize, desired_x)
    }

    /// 当前行的首/尾偏移（Home / End）。渲染行退回块的首/尾。
    pub fn line_bounds(&self, offset: usize) -> (usize, usize) {
        if let Some(i) = self.line_of_offset(offset) {
            return self.layout.lines[i].source.unwrap();
        }
        let Some((i, _)) = self.locate(offset) else {
            return (offset, offset);
        };
        let line = &self.layout.lines[i];
        let map = self.mapping(line.block);
        (
            map.source_at(line.visible_start),
            map.source_at(
                line.visible_start + line.runs.iter().map(text::visible_len).sum::<usize>(),
            ),
        )
    }

    pub fn move_horizontal(&self, buffer: &mut TextBuffer, right: bool, extend: bool) {
        if buffer.has_selection() && !extend {
            let (a, b) = buffer.selection();
            buffer.set_cursor(if right { b } else { a }, false);
            return;
        }
        let at = buffer.cursor();
        let Some(bi) = self.parsed.block_at(at) else {
            return;
        };
        let rb = &self.parsed.blocks[bi];
        let code_body = super::rich::code_body(rb, buffer.text());
        if super::editor_preferences::current().live_line_source
            || (matches!(rb.block, Block::Code { .. })
                && if right {
                    at < code_body.end
                } else {
                    at > code_body.start
                })
        {
            if right {
                buffer.move_right(extend);
            } else if at > self.min_offset() {
                buffer.move_left(extend);
            }
            return;
        }
        let map = self.mapping(bi);
        let next = if right {
            map.units
                .iter()
                .find(|u| u.source.end > at)
                .map(|u| u.source.end)
        } else {
            map.units
                .iter()
                .rev()
                .find(|u| u.source.start < at)
                .map(|u| u.source.start)
        };
        let next = next.or_else(|| {
            let target = if right {
                self.layout
                    .lines
                    .iter()
                    .find(|l| l.block > bi && l.visible_start != usize::MAX)
            } else {
                self.layout
                    .lines
                    .iter()
                    .rev()
                    .find(|l| l.block < bi && l.visible_start != usize::MAX)
            }?;
            Some(if let Some((s, e)) = target.source {
                if right {
                    s
                } else {
                    e
                }
            } else if right {
                self.mapping(target.block).start
            } else {
                self.mapping(target.block).end
            })
        });
        if let Some(next) = next {
            buffer.set_cursor(next.max(self.min_offset()), extend);
        }
    }

    pub fn selected_text(&self, buffer: &TextBuffer) -> String {
        let (a, b) = buffer.selection();
        let mut hidden = super::block_markers::hidden_ranges(buffer.text());
        for rb in &self.parsed.blocks {
            match &rb.block {
                Block::Container(panel) => hidden.push(panel.footer.clone()),
                Block::Code { .. } => {
                    hidden.push(super::code_blocks::metadata(buffer.text(), rb.start).0)
                }
                _ => {}
            }
        }
        let mut text = String::new();
        for (i, rb) in self.parsed.blocks.iter().enumerate() {
            if rb.end < a || rb.start >= b {
                continue;
            }
            let value = if let Block::Container(panel) = &rb.block {
                panel.title.clone()
            } else if matches!(rb.block, Block::Code { .. }) {
                let range = super::rich::code_body(rb, buffer.text());
                buffer.text()[a.max(range.start).min(range.end)..b.min(range.end).max(range.start)]
                    .to_owned()
            } else {
                self.mapping(i)
                    .units
                    .iter()
                    .filter(|u| u.source.start < b && u.source.end > a)
                    .map(|u| u.text.as_str())
                    .collect()
            };
            text.push_str(&value);
            // 空块也要贡献它被选中的换行符。用原始的边界，
            // 复制/粘贴时空行和 CRLF 才不会丢。
            if let Some(next) = self.parsed.blocks.get(i + 1) {
                let start = rb.end.max(a);
                let end = next.start.min(b);
                if start < end {
                    for (offset, ch) in buffer.text()[start..end].char_indices() {
                        if !hidden.iter().any(|r| r.contains(&(start + offset))) {
                            text.push(ch);
                        }
                    }
                }
            }
        }
        text
    }
}

/// 在每个完整的成形 run 内测量，保留粗体/斜体宽度和公式原子。
/// 对展平后的纯字符串测量会把光标放到错误的字形上。
fn run_caret(line: &document::LaidOutLine, mut visible: usize) -> f32 {
    let mut x = 0.0;
    for run in &line.runs {
        let n = text::visible_len(run);
        if visible >= n {
            x += text::run_width(run, line.style);
            visible -= n;
            continue;
        }
        if run.emphasis.base() == text::Emphasis::Math {
            return x;
        }
        let byte = run
            .text
            .char_indices()
            .nth(visible)
            .map(|(i, _)| i)
            .unwrap_or(run.text.len());
        return x + super::measurement::shape(&run.text, line.style, run.emphasis)
            .map(|s| s.caret(byte))
            .unwrap_or_else(|| {
                run.text[..byte]
                    .chars()
                    .map(|c| {
                        text::advance_for(c, line.style, run.emphasis) * line.style.font_size()
                    })
                    .sum()
            });
    }
    x
}

fn floor_char_boundary(s: &str, index: usize) -> usize {
    let mut i = index.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// 块可见文本在块源码里从第几个字节开始（跳过 `# `、`> `、`- [ ] ` 这类前缀）。
fn content_start(block: &Block, block_src: &str) -> usize {
    let indent = block_src.len() - block_src.trim_start().len();
    let trimmed = &block_src[indent..];
    let prefix = match block {
        Block::Aligned { .. } => trimmed.find('>').map(|i| i + 1).unwrap_or(0),
        Block::Heading { .. } => trimmed.find(' ').map(|i| i + 1).unwrap_or(trimmed.len()),
        Block::Quote(_) => {
            if trimmed.starts_with("> ") {
                2
            } else {
                usize::from(trimmed.starts_with('>'))
            }
        }
        Block::ListItem { .. } => {
            let mut p = 0;
            for bullet in [
                "- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "+ [ ] ", "- ", "* ", "+ ",
            ] {
                if trimmed.starts_with(bullet) {
                    p = bullet.len();
                    break;
                }
            }
            if p == 0 {
                let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
                if digits > 0
                    && (trimmed[digits..].starts_with(". ") || trimmed[digits..].starts_with(") "))
                {
                    p = digits + 2;
                }
            }
            p
        }
        _ => 0,
    };
    indent + prefix
}

/// 可见字符序号 → 块内字节偏移。
///
/// 先按 run 定位（每个 run 带源码范围），再在 run 内部做贪心对齐：渲染文本是源码去掉标记符后的
/// **子序列**，源码字符与可见字符相同就同时前进，不同就只前进源码（那是个标记符）。
/// 公式 run 的显示文字是转写出来的，不是源码子序列——点在它上面就落到公式前/后。
#[cfg(test)]
pub fn visible_to_source(block: &Block, block_src: &str, visible_index: usize) -> usize {
    if matches!(block, Block::Math(_)) {
        // 转写后的数学符号不是源码子序列；点入时落在 TeX 正文起点，随后按源码排版。
        let start = block_src.len() - block_src.trim_start().len() + 2;
        let end = block_src.trim_end().len().saturating_sub(2);
        return if visible_index == 0 {
            start.min(block_src.len())
        } else {
            end.max(start).min(block_src.len())
        };
    }
    let start = content_start(block, block_src);
    let content = block_src[start..].trim_end();
    let spans = text::parse_inline_spans(content);
    let mut seen = 0usize;
    let last = spans.len().saturating_sub(1);
    for (si, s) in spans.iter().enumerate() {
        let n = text::visible_len(&s.run);
        let inside = visible_index < seen + n || (si == last && visible_index == seen + n);
        if !inside {
            seen += n;
            continue;
        }
        let local = visible_index - seen;
        if s.run.emphasis.base() == text::Emphasis::Math {
            return start + if local == 0 { s.start } else { s.end };
        }
        return start + s.start + align_within(&content[s.start..s.end], &s.run.text, local);
    }
    start + content.len()
}

/// 在一个 run 的源码片段里，找第 `visible_index` 个可见字符对应的字节偏移。
#[cfg(test)]
fn align_within(source: &str, visible: &str, visible_index: usize) -> usize {
    let mut vis = visible.chars();
    let mut want = vis.next();
    let mut seen = 0usize;
    for (byte, ch) in source.char_indices() {
        let is_visible = want == Some(ch);
        // 到了目标序号：落在**下一个可见字符之前**、夹在中间的标记符之后。
        // 「是|**粗」点在两字之间，光标进到 `**` 里面——这样接着打的字仍在粗体里，
        // 与 Typora 的手感一致；停在星号外面的话打出来的字会跑到粗体外
        if seen >= visible_index && (is_visible || want.is_none()) {
            return byte;
        }
        if is_visible {
            seen += 1;
            want = vis.next();
        }
    }
    source.len()
}

/// 画光标与选区，叠在 `document::paint` 之上。坐标系与 `document::paint` 一致
/// （内容区左缘 = area.left + 左留白）。`active` 为假什么都不画；`caret` 决定画不画光标
/// （查找条拿着焦点时要显示当前匹配的选区，但光标在查找框里）。
#[allow(clippy::too_many_arguments)]
pub fn paint_overlay(
    list: &mut DrawList,
    area: Rect,
    live: &LiveLayout,
    buffer: &TextBuffer,
    scroll: f32,
    active: bool,
    caret: bool,
    p: &Palette,
) {
    if area.is_empty() || !active {
        return;
    }
    list.push_clip(area);
    let origin_x = area.left + super::editor_preferences::current().padding_left;
    let top = area.top - scroll;
    let first = live
        .layout
        .lines
        .partition_point(|line| line.y + line.height < scroll);
    let end = live
        .layout
        .lines
        .partition_point(|line| line.y <= scroll + area.height());
    let visible_lines = &live.layout.lines[first.min(end)..end];

    // 选区几何也被评论锚点复用，不必每帧再构造一个全文档 TextBuffer。
    let (sel_a, sel_b) = buffer.selection();
    if sel_b > sel_a && buffer.composition().is_none() {
        paint_selection(list, area, live, sel_a..sel_b, scroll, p);
    }

    // 组合串下划线（IME）
    if let Some(comp) = buffer.composition() {
        let start = comp.start;
        let end = start + comp.text.len();
        for line in visible_lines {
            let (x1, x2) = if let Some((a, b)) = line.source {
                if end <= a || start >= b {
                    continue;
                }
                let full = line
                    .runs
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<String>();
                (
                    text::caret_x(&full, start.max(a) - a, line.style),
                    text::caret_x(&full, end.min(b) - a, line.style),
                )
            } else {
                if line.visible_start == usize::MAX {
                    continue;
                }
                let rb = &live.parsed.blocks[line.block];
                if end <= rb.start || start >= rb.end {
                    continue;
                }
                let map = live.mapping(line.block);
                (
                    run_caret(
                        line,
                        map.visible_at(start).saturating_sub(line.visible_start),
                    ),
                    run_caret(line, map.visible_at(end).saturating_sub(line.visible_start)),
                )
            };
            if x2 <= x1 {
                continue;
            }
            let y = top + line.y;
            list.rect_alpha(
                Rect::new(
                    origin_x + line.x + x1,
                    y,
                    origin_x + line.x + x2,
                    y + line.height,
                ),
                p.accent,
                0.14,
            );
            list.rect(
                Rect::new(
                    origin_x + line.x + x1,
                    y + line.height - 2.0,
                    origin_x + line.x + x2,
                    y + line.height - 1.0,
                ),
                p.accent,
            );
        }
    }

    if caret {
        if let Some(c) = live.caret(buffer.display_cursor()) {
            let r = Rect::new(
                origin_x + c.left,
                top + c.top,
                origin_x + c.right,
                top + c.bottom,
            );
            if r.bottom >= area.top && r.top <= area.bottom {
                list.caret(r, p.foreground);
            }
        }
    }
    list.pop_clip();
}

/// 只用可见行绘制任意源码区间。评论与编辑器共用精确映射，
/// 但不必持有或复制它的文本。
pub fn paint_selection(
    list: &mut DrawList,
    area: Rect,
    live: &LiveLayout,
    selection: std::ops::Range<usize>,
    scroll: f32,
    p: &Palette,
) {
    if area.is_empty() || selection.is_empty() {
        return;
    }
    let (sel_a, sel_b) = (selection.start, selection.end);
    let origin_x = area.left + super::editor_preferences::current().padding_left;
    let top = area.top - scroll;
    let first = live
        .layout
        .lines
        .partition_point(|line| line.y + line.height < scroll);
    let end = live
        .layout
        .lines
        .partition_point(|line| line.y <= scroll + area.height());
    for line in &live.layout.lines[first.min(end)..end] {
        let y = top + line.y;
        if y + line.height < area.top || y > area.bottom {
            continue;
        }
        let (ls, le) = match line.source {
            Some(r) => r,
            None => {
                let rb = &live.parsed.blocks[line.block];
                (rb.start, rb.end)
            }
        };
        let a = sel_a.max(ls);
        let b = sel_b.min(le);
        let spans_past_end = sel_a <= le && sel_b > le;
        if a >= b && !spans_past_end {
            continue;
        }
        let full: String = line.runs.iter().map(|r| r.text.as_str()).collect();
        let (x1, x2) = match line.source {
            Some((s, _)) => {
                let x1 = text::caret_x(&full, a.saturating_sub(s), line.style);
                let x2 = if sel_b > le {
                    text::measure(&full, line.style) + line.style.font_size() * 0.4
                } else {
                    text::caret_x(&full, b.saturating_sub(s), line.style)
                };
                (x1, x2)
            }
            None => {
                if line.visible_start == usize::MAX {
                    if matches!(line.decoration, document::Decoration::Image) {
                        list.rounded_border(
                            Rect::new(
                                origin_x + line.x,
                                y,
                                origin_x + line.x + document::card_width(area),
                                y + line.height,
                            ),
                            3.0,
                            p.accent,
                        );
                    }
                    continue;
                }
                let map = live.mapping(line.block);
                let a = map.visible_at(a).saturating_sub(line.visible_start);
                let b = map.visible_at(b).saturating_sub(line.visible_start);
                (run_caret(line, a), run_caret(line, b))
            }
        };
        if x2 > x1 {
            list.rect_alpha(
                Rect::new(
                    origin_x + line.x + x1,
                    y,
                    origin_x + line.x + x2,
                    y + line.height,
                ),
                p.accent,
                0.22,
            );
        }
    }
}

/// 光标在客户区里的矩形（IME 候选窗定位）。
pub fn caret_in_area(
    area: Rect,
    live: &LiveLayout,
    buffer: &TextBuffer,
    scroll: f32,
) -> Option<Rect> {
    let c = live.caret(buffer.display_cursor())?;
    let origin_x = area.left + super::editor_preferences::current().padding_left;
    let top = area.top - scroll;
    Some(Rect::new(
        origin_x + c.left,
        top + c.top,
        origin_x + c.right,
        top + c.bottom,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::document::LaidOutLine;
    use crate::ui::draw::TextStyle;

    const W: f32 = 600.0;
    const SRC: &str =
        "# 标题\n\n这是**粗体**和普通。\n\n- 第一项\n- 第二项\n\n```rs\nlet a = 1;\n```\n";

    #[test]
    fn comment_range_geometry_matches_selection_without_copying_document() {
        let source = format!("{}\n{}", SRC, SRC.repeat(30));
        let live = layout(&source, None, W, &|_| None);
        let area = Rect::from_size(15.0, 20.0, 700.0, 280.0);
        let palette = theme::tokens().palette(false);
        let mut buffer = TextBuffer::new(&source);
        let start = source.find("粗体").unwrap();
        let end = source.len() - 1;
        buffer.set_cursor(start, false);
        buffer.set_cursor(end, true);
        for scroll in [0.0, 150.0, live.layout.height - 250.0] {
            let mut selection = DrawList::new();
            paint_overlay(
                &mut selection,
                area,
                &live,
                &buffer,
                scroll,
                true,
                false,
                palette,
            );
            let mut comment = DrawList::new();
            paint_selection(&mut comment, area, &live, start..end, scroll, palette);
            let rectangles = |list: &DrawList| {
                list.cmds()
                    .iter()
                    .filter_map(|cmd| {
                        if let crate::ui::draw::DrawCmd::RectAlpha { rect, .. } = cmd {
                            Some(*rect)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(rectangles(&comment), rectangles(&selection));
            assert!(!rectangles(&comment).is_empty());
        }
    }

    #[test]
    fn blocks_carry_their_source_ranges() {
        let p = document::parse_ranged(SRC);
        let heading = &p.blocks[0];
        assert_eq!(&SRC[heading.start..heading.end], "# 标题");
        let para = p
            .blocks
            .iter()
            .find(|b| matches!(b.block, Block::Paragraph(_)))
            .unwrap();
        assert_eq!(&SRC[para.start..para.end], "这是**粗体**和普通。");
        let code = p
            .blocks
            .iter()
            .find(|b| matches!(b.block, Block::Code { .. }))
            .unwrap();
        assert_eq!(&SRC[code.start..code.end], "```rs\nlet a = 1;\n```");
        // 以换行结尾：末尾还有一个空块，光标能停在文末
        assert_eq!(p.blocks.last().unwrap().start, SRC.len());
        assert_eq!(p.block_at(0), Some(0));
        assert_eq!(p.block_at(heading.end), Some(0), "块末尾的位置归该块");
        assert_eq!(p.block_at(heading.end + 1), Some(1), "换行之后是下一个块");
    }

    #[test]
    fn frontmatter_moves_the_body_start() {
        let p = document::parse_ranged("---\ntitle: x\n---\n正文");
        assert_eq!(p.body_start, "---\ntitle: x\n---\n".len());
        assert_eq!(p.blocks.len(), 1);
        assert_eq!(p.blocks[0].block, Block::Paragraph("正文".into()));
    }

    #[test]
    fn the_active_block_keeps_its_rich_text_and_hidden_syntax() {
        let para_start = SRC.find("这是").unwrap();
        let live = layout(SRC, Some(para_start + 3), W, &|_| None);
        let bi = live.parsed.block_at(para_start + 3).unwrap();
        assert!(matches!(live.parsed.blocks[bi].block, Block::Paragraph(_)));
        let raw: Vec<&LaidOutLine> = live.layout.lines.iter().filter(|l| l.block == bi).collect();
        assert_eq!(raw.len(), 1);
        let shown: String = raw[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(shown, "这是粗体和普通。", "编辑时仍然保持富文本");
        assert!(raw[0].source.is_none());
        // 标题仍是渲染态：不带 `# `
        let heading_line = live.layout.lines.iter().find(|l| l.block == 0).unwrap();
        assert_eq!(heading_line.runs[0].text, "标题");
        assert!(heading_line.source.is_none());
    }

    #[test]
    fn locating_the_cursor_inside_the_raw_block_measures_the_prefix() {
        let para_start = SRC.find("这是").unwrap();
        let live = layout(SRC, Some(para_start), W, &|_| None);
        let (i0, x0) = live.locate(para_start).unwrap();
        let (i1, x1) = live.locate(para_start + "这是".len()).unwrap();
        assert_eq!(i0, i1);
        assert_eq!(x0, 0.0);
        assert_eq!(x1, text::measure("这是", TextStyle::Body));
        let caret = live.caret(para_start + "这是".len()).unwrap();
        assert_eq!(caret.width(), CARET_WIDTH);
    }

    #[test]
    fn rendered_paragraph_keeps_trailing_spaces_in_the_caret_geometry() {
        let before = layout("文字", Some("文字".len()), W, &|_| None);
        let after = layout("文字 ", Some("文字 ".len()), W, &|_| None);

        // 仍是富文本布局，不因输入空格而切换成源码行；但光标必须立即前移。
        assert!(after.layout.lines[0].source.is_none());
        assert!(
            after.caret("文字 ".len()).unwrap().left > before.caret("文字".len()).unwrap().left
        );
    }

    #[test]
    fn clicking_a_rendered_block_maps_visible_characters_back_past_the_markers() {
        // 光标在标题里，段落是渲染态：显示「这是粗体和普通。」
        let live = layout(SRC, Some(2), W, &|_| None);
        let para = live
            .parsed
            .blocks
            .iter()
            .position(|b| matches!(b.block, Block::Paragraph(_)))
            .unwrap();
        let line_i = live
            .layout
            .lines
            .iter()
            .position(|l| l.block == para)
            .unwrap();
        let line = &live.layout.lines[line_i];
        assert!(line.source.is_none());
        let para_start = live.parsed.blocks[para].start;
        // 点在「粗」字前（可见第 2 个字之后）：源码里要越过 `**`
        let x = text::measure("这是", TextStyle::Body) + 1.0;
        let off = live.offset_in_line(SRC, line_i, x);
        assert_eq!(
            &SRC[off..off + "粗体".len()],
            "粗体",
            "落点应在星号之后、粗字之前"
        );
        // 点在行首
        assert_eq!(live.offset_in_line(SRC, line_i, 0.0), para_start);
        // 点在行尾之外：块末
        assert_eq!(
            live.offset_in_line(SRC, line_i, 9999.0),
            live.parsed.blocks[para].end
        );
    }

    #[test]
    fn rendered_cjk_clicks_and_carets_stay_on_character_boundaries() {
        // 回归测试：中文字符的光标位置曾会穿过“也”字。
        // 渲染出的文档必须把可见位置映射回真实的 UTF-8 边界，
        // 后续输入才能插在同一个点上。
        let src = "当然，也可以在下方的设置中调节自定义背景的透明度";
        let live = layout(src, Some(src.len()), W, &|_| None);
        let line = &live.layout.lines[0];
        let before_ye = src.find('也').unwrap();
        let x = text::measure("当然，", line.style) + 0.1;
        let offset = live.offset_in_line(src, 0, x);
        assert_eq!(offset, before_ye);
        assert!(src.is_char_boundary(offset));
        let caret = live.caret(offset).unwrap();
        let expected_x = text::measure("当然，", line.style);
        assert!(
            (caret.left - expected_x).abs() < 0.01,
            "caret must use the boundary before 「也」, not its glyph center"
        );

        let mut buffer = TextBuffer::new(src);
        buffer.set_cursor(offset, false);
        buffer.insert("X");
        assert_eq!(
            buffer.text(),
            "当然，X也可以在下方的设置中调节自定义背景的透明度"
        );
    }

    #[test]
    fn list_items_and_headings_skip_their_prefixes_when_mapped() {
        let live = layout(SRC, Some(SRC.len()), W, &|_| None);
        let item = live
            .parsed
            .blocks
            .iter()
            .position(|b| matches!(b.block, Block::ListItem { .. }))
            .unwrap();
        let rb = &live.parsed.blocks[item];
        let src = &SRC[rb.start..rb.end];
        assert_eq!(src, "- 第一项");
        assert_eq!(visible_to_source(&rb.block, src, 0), 2, "跳过 `- `");
        assert_eq!(visible_to_source(&rb.block, src, 1), 2 + "第".len());
        let h = &live.parsed.blocks[0];
        assert_eq!(
            visible_to_source(&h.block, &SRC[h.start..h.end], 0),
            2,
            "跳过 `# `"
        );
    }

    #[test]
    fn clicking_on_a_formula_lands_before_or_after_it_and_text_after_it_still_maps() {
        let src = "前 $x^2$ 后文";
        let block = Block::Paragraph(src.to_owned());
        // 可见文本「前 x² 后文」：序号 0 前，1 空格，2 x，3 ²，4 空格，5 后，6 文
        assert_eq!(
            visible_to_source(&block, src, 2),
            "前 ".len(),
            "点在公式开头：落在 `$` 前"
        );
        assert_eq!(
            visible_to_source(&block, src, 3),
            "前 $x^2$".len(),
            "点在公式中间：落到公式之后"
        );
        assert_eq!(
            visible_to_source(&block, src, 5),
            "前 $x^2$ ".len(),
            "公式后面的字照常对齐"
        );
        assert_eq!(visible_to_source(&block, src, 7), src.len());
    }

    #[test]
    fn formula_hit_testing_uses_native_box_width_and_preserves_following_chinese_offsets() {
        let src = r"前 $\frac{a+b}{c+d}$ 后文";
        let live = layout(src, None, 600.0, &|_| None);
        let line = &live.layout.lines[0];
        let prefix = text::measure("前 ", line.style);
        let formula = line
            .runs
            .iter()
            .find(|r| r.emphasis == text::Emphasis::Math)
            .unwrap();
        let width = text::run_width(formula, line.style);
        let y = line.y + line.height / 2.0;
        assert_eq!(live.hit(src, prefix + width * 0.2, y), Some("前 ".len()));
        assert_eq!(
            live.hit(src, prefix + width * 0.8, y),
            Some(src.rfind('$').unwrap() + 1)
        );
        assert_eq!(
            live.hit(
                src,
                prefix + width + text::measure(" ", line.style) + 1.0,
                y
            ),
            Some(src.find('后').unwrap())
        );
    }

    #[test]
    fn wrapped_formula_does_not_shift_source_offsets_when_separator_spaces_are_removed() {
        let src = r"prefix $\frac{a+b+c}{d+e+f}$ 中文结尾";
        let width = crate::ui::math_layout::size(
            r"\frac{a+b+c}{d+e+f}",
            TextStyle::Document.font_size() * text::MATH_SCALE,
        )
        .unwrap()
        .0 + 1.0;
        let live = layout(src, None, width, &|_| None);
        let line = live
            .layout
            .lines
            .iter()
            .find(|l| l.runs.iter().any(|r| r.text.starts_with('中')))
            .unwrap();
        assert_eq!(
            live.hit(src, line.x + 0.1, line.y + line.height / 2.0),
            Some(src.find('中').unwrap())
        );
    }

    #[test]
    fn vertical_movement_crosses_block_boundaries() {
        let para_start = SRC.find("这是").unwrap();
        let live = layout(SRC, Some(para_start), W, &|_| None);
        let up = live.move_vertical(SRC, para_start, -1, 0.0);
        assert!(up < para_start, "向上应进入前一个块（空行或标题）");
        let down = live.move_vertical(SRC, para_start, 1, 0.0);
        assert!(down > para_start);
        // 顶部再向上：不动
        let live0 = layout(SRC, Some(0), W, &|_| None);
        assert_eq!(live0.move_vertical(SRC, 0, -1, 0.0), 0);
    }

    #[test]
    fn code_block_lines_have_exact_ranges_even_when_not_active() {
        let live = layout(SRC, Some(0), W, &|_| None);
        let code_line = live
            .layout
            .lines
            .iter()
            .find(|l| {
                l.runs
                    .first()
                    .map(|r| r.text == "let a = 1;")
                    .unwrap_or(false)
            })
            .unwrap();
        let (s, e) = code_line.source.unwrap();
        assert_eq!(&SRC[s..e], "let a = 1;");
    }

    #[test]
    fn home_and_end_use_the_raw_line_bounds() {
        let para_start = SRC.find("这是").unwrap();
        let live = layout(SRC, Some(para_start + 3), W, &|_| None);
        let (s, e) = live.line_bounds(para_start + 3);
        assert_eq!(s, para_start);
        assert_eq!(e, para_start + "这是**粗体**和普通。".len());
    }

    #[test]
    fn the_overlay_paints_a_caret_only_when_focused() {
        let para_start = SRC.find("这是").unwrap();
        let live = layout(SRC, Some(para_start), W, &|_| None);
        let mut buffer = TextBuffer::new(SRC);
        buffer.set_cursor(para_start, false);
        let area = Rect::new(0.0, 0.0, 700.0, 500.0);
        // 只数真正画出来的矩形；裁剪的 push/pop 不算
        let count = |active: bool, caret: bool| {
            let mut list = DrawList::new();
            paint_overlay(
                &mut list,
                area,
                &live,
                &buffer,
                0.0,
                active,
                caret,
                theme::tokens().palette(false),
            );
            list.cmds()
                .iter()
                .filter(|c| matches!(c, crate::ui::draw::DrawCmd::Caret { .. }))
                .count()
        };
        assert!(count(true, true) > 0);
        assert_eq!(count(false, true), 0);
        // 只显示选区不画光标（查找条聚焦时）：没有选区就没东西画
        assert_eq!(count(true, false), 0);
    }

    #[test]
    fn an_empty_document_still_has_somewhere_to_put_the_caret() {
        let live = layout("", Some(0), W, &|_| None);
        assert_eq!(live.parsed.blocks.len(), 1);
        assert!(live.caret(0).is_some());
        assert_eq!(live.hit("", 10.0, 10.0), Some(0));
    }

    #[test]
    fn consecutive_enters_keep_every_blank_line_editable() {
        for source in [
            "",
            "正文",
            "**正文**",
            "# 标题",
            "前文\r\n正文",
            "---\ntitle: 保留\n---\n",
        ] {
            for shift in [false, true] {
                let mut buffer = TextBuffer::new(source);
                buffer.set_cursor(source.len(), false);
                let mut before = layout(buffer.text(), Some(buffer.cursor()), W, &|_| None);
                for count in 1..=4 {
                    super::super::rich::enter(&mut buffer, shift);
                    let after = layout(buffer.text(), Some(buffer.cursor()), W, &|_| None);
                    assert_eq!(
                        after.layout.lines.len(),
                        before.layout.lines.len() + 1,
                        "{source:?}, Enter {count}"
                    );
                    let caret = after.caret(buffer.cursor()).unwrap();
                    let previous = before.layout.lines.last().unwrap();
                    assert!(
                        caret.top >= previous.y + previous.height - 0.01,
                        "Enter {count} did not advance a full line in {source:?}"
                    );
                    if count > 1 {
                        assert!(
                            (after.layout.height
                                - before.layout.height
                                - TextStyle::Document.line_height())
                            .abs()
                                < 0.01
                        );
                    }
                    before = after;
                }
                super::super::rich::insert(&mut buffer, "后文");
                let typed = layout(buffer.text(), Some(buffer.cursor()), W, &|_| None);
                assert_eq!(
                    typed.layout.lines.len(),
                    before.layout.lines.len(),
                    "typing must not collapse the preceding blank lines"
                );
            }
        }
    }

    #[test]
    fn blank_line_geometry_and_navigation_do_not_depend_on_focus() {
        for source in ["前\n\n\n后\n", "前\r\n\r\n\r\n后\r\n", "\n\n\n"] {
            let baseline = layout(source, None, W, &|_| None);
            let offsets = baseline
                .parsed
                .blocks
                .iter()
                .map(|b| b.start)
                .collect::<Vec<_>>();
            assert_eq!(baseline.layout.lines.len(), offsets.len());
            for &cursor in &offsets {
                let live = layout(source, Some(cursor), W, &|_| None);
                assert_eq!(live.layout.height, baseline.layout.height);
                for (i, &offset) in offsets.iter().enumerate() {
                    let caret = live.caret(offset).expect("every blank line needs a caret");
                    assert_eq!(caret, baseline.caret(offset).unwrap());
                    assert_eq!(
                        live.hit(source, 0.0, caret.top + caret.height() / 2.0),
                        Some(offset)
                    );
                    if i > 0 {
                        assert_eq!(live.move_vertical(source, offset, -1, 0.0), offsets[i - 1]);
                    }
                    if i + 1 < offsets.len() {
                        assert_eq!(live.move_vertical(source, offset, 1, 0.0), offsets[i + 1]);
                    }
                }
            }
        }
    }

    #[test]
    fn rich_copy_preserves_selected_blank_lines_and_line_endings() {
        for newline in ["\n", "\r\n"] {
            let source = format!(
                "{newline}**前文**{}{tail}{newline}",
                newline.repeat(3),
                tail = "后文"
            );
            let live = layout(&source, None, W, &|_| None);
            let mut buffer = TextBuffer::new(&source);
            buffer.select_all();
            assert_eq!(live.selected_text(&buffer), source.replace("**", ""));
            let start = source.find("**前文**").unwrap() + "**前文**".len();
            let end = source.find("后文").unwrap();
            buffer.set_cursor(start, false);
            buffer.set_cursor(end, true);
            assert_eq!(live.selected_text(&buffer), newline.repeat(3));
        }
    }

    #[test]
    fn arrows_skip_hidden_syntax_and_selection_overlay_is_translucent() {
        let source = "**中😀**尾";
        let live = layout(source, Some(2), W, &|_| None);
        let mut b = TextBuffer::new(source);
        b.set_cursor(2, false);
        live.move_horizontal(&mut b, true, true);
        assert_eq!(b.cursor(), 5);
        live.move_horizontal(&mut b, true, true);
        assert_eq!(b.cursor(), 9);
        let mut list = DrawList::new();
        paint_overlay(
            &mut list,
            Rect::new(0.0, 0.0, 700.0, 500.0),
            &live,
            &b,
            0.0,
            true,
            false,
            theme::tokens().palette(false),
        );
        assert!(list.cmds().iter().any(
            |c| matches!(c,crate::ui::draw::DrawCmd::RectAlpha{alpha,..} if *alpha>0.0&&*alpha<0.5)
        ));
        assert!(!list
            .cmds()
            .iter()
            .any(|c| matches!(c, crate::ui::draw::DrawCmd::Rect { .. })));
        live.move_horizontal(&mut b, true, true);
        assert_eq!(b.cursor(), source.len());
    }

    #[test]
    fn closed_code_fences_have_exact_editable_body_ranges() {
        for source in [
            "```rust\r\nlet a = 1;\r\nlet b = 2;\r\n```",
            "```rust\nlet a = 1;\n```",
        ] {
            let live = layout(source, Some(source.find("let").unwrap()), W, &|_| None);
            let lines = live
                .layout
                .lines
                .iter()
                .filter(|l| l.source.is_some())
                .collect::<Vec<_>>();
            assert!(!lines.is_empty());
            for line in lines {
                let (a, b) = line.source.unwrap();
                assert_eq!(line.runs[0].text, &source[a..b]);
                assert!(live.caret(a).is_some());
            }
            let body = crate::ui::rich::code_body(&live.parsed.blocks[0], source);
            assert!(source[body].ends_with(';'));
        }
    }

    #[test]
    fn unclosed_fences_keep_the_following_paragraph_visible_and_editable() {
        // 解析器刻意不把笔记剩余部分吞进一个未闭合的代码卡。
        // 它的段落用富文本映射，而不是代码体那种精确的 `line.source` 表示。
        let source = "```rust\nlet a = 1;\n\n末尾中文🙂";
        let live = layout(source, Some(8), W, &|_| None);
        assert!(live
            .parsed
            .blocks
            .iter()
            .all(|rb| !matches!(rb.block, Block::Code { .. })));
        for offset in [
            source.find("let").unwrap(),
            source.find("末尾").unwrap(),
            source.len(),
        ] {
            assert!(live.caret(offset).is_some());
        }
        let mut buffer = TextBuffer::new(source);
        buffer.set_cursor(source.len(), false);
        super::super::rich::insert_parsed(&mut buffer, "追加", &live.parsed);
        assert_eq!(buffer.text(), format!("{source}追加"));
        buffer.undo();
        assert_eq!(buffer.text(), source);
    }

    #[test]
    fn current_block_source_is_an_opt_in_preference() {
        struct Restore(crate::ui::editor_preferences::Preferences);
        impl Drop for Restore {
            fn drop(&mut self) {
                crate::ui::editor_preferences::set(self.0.clone());
            }
        }
        let old = crate::ui::editor_preferences::current();
        let _restore = Restore(old.clone());
        let mut prefs = old;
        prefs.live_line_source = true;
        crate::ui::editor_preferences::set(prefs);
        let live = layout("**text**", Some(2), W, &|_| None);
        assert_eq!(live.layout.lines[0].runs[0].text, "**text**");
        assert_eq!(live.layout.lines[0].source, Some((0, 8)));
    }

    #[test]
    fn mapping_cache_rebases_unchanged_inline_blocks_after_prefix_insertion() {
        let source = "前文\n\n**稳定** 与 `代码`\n\n后文";
        let parsed = document::parse_ranged(source);
        let cache = MappingCache::default();
        let stable = parsed
            .blocks
            .iter()
            .find(|block| matches!(&block.block, Block::Paragraph(text) if text.contains("稳定")))
            .unwrap();
        let before = cache.get(stable, source);
        let before_start = stable.start;

        let edited = format!("新增内容\n{source}");
        let next = document::parse_ranged(&edited);
        let stable_next = next
            .blocks
            .iter()
            .find(|block| matches!(&block.block, Block::Paragraph(text) if text.contains("稳定")))
            .unwrap();
        let after = cache.get(stable_next, &edited);
        assert_eq!(cache.stats().1, 1, "相同块源码应命中映射缓存");
        let delta = stable_next.start - before_start;
        assert_eq!(after.start, before.start + delta);
        assert_eq!(after.end, before.end + delta);
        assert_eq!(
            after
                .units
                .iter()
                .map(|unit| unit.text.as_str())
                .collect::<String>(),
            before
                .units
                .iter()
                .map(|unit| unit.text.as_str())
                .collect::<String>()
        );
        for (old, rebased) in before.units.iter().zip(after.units.iter()) {
            assert_eq!(rebased.source.start, old.source.start + delta);
            assert_eq!(rebased.source.end, old.source.end + delta);
        }
    }

    #[test]
    fn source_mappings_are_lazy_and_still_cover_the_last_block() {
        let source = "**中文🙂** and `code`\n\n".repeat(2_000);
        let live = layout(&source, None, W, &|_| None);
        assert!(live.maps.iter().all(|mapping| mapping.get().is_none()));
        let offset = source.rfind("中文").unwrap();
        let block = live.parsed.block_at(offset).unwrap();
        assert!(live.caret(offset).is_some());
        assert_eq!(
            live.maps
                .iter()
                .filter(|mapping| mapping.get().is_some())
                .count(),
            1
        );
        assert_eq!(live.mapping(block).source_at(0), offset);
        // 新布局持有自己的快照，旧布局仍可用，缓存的映射随之
        // 平移到新的源码位置。
        let edited = format!("prefix\n{source}");
        let next = layout(&edited, None, W, &|_| None);
        assert!(next.caret(offset + 7).is_some());
        assert_eq!(live.mapping(block).source_at(0), offset);
    }

    #[test]
    fn local_inline_reflow_matches_full_layout_and_rebases_later_source_ranges() {
        for width in [160.0, 600.0] {
            let mut source = "# Heading\n\n中文 **stable** and `code` words words words words\n\n## Later\n\n```rust\nlet a = 1;\n```\n\ntail 🙂".to_owned();
            let mut live = layout(&source, None, width, &|_| None);
            assert!(live.caret(source.rfind("tail").unwrap()).is_some());
            for (from, to) in [
                ("stable", "changed text 中文🙂"),
                ("changed text 中文🙂", "x"),
                ("Heading", "Heading updated"),
                ("tail 🙂", "tail **完整**🙂"),
            ] {
                let at = source.find(from).unwrap();
                source.replace_range(at..at + from.len(), to);
                assert!(live.update_inline(&source, width));
                let full = layout(&source, None, width, &|_| None);
                assert_eq!(live.parsed, full.parsed);
                assert_eq!(live.layout.lines.len(), full.layout.lines.len());
                for (actual, expected) in live.layout.lines.iter().zip(&full.layout.lines) {
                    assert!((actual.y - expected.y).abs() < 0.001);
                    let mut actual = actual.clone();
                    actual.y = expected.y;
                    assert_eq!(&actual, expected);
                }
                for (actual, expected) in live.layout.headings.iter().zip(&full.layout.headings) {
                    assert!((actual.y - expected.y).abs() < 0.001);
                    assert_eq!(
                        (&actual.text, actual.level),
                        (&expected.text, expected.level)
                    );
                }
                assert!((live.layout.height - full.layout.height).abs() < 0.01);
                for rb in &full.parsed.blocks {
                    assert_eq!(live.locate(rb.start), full.locate(rb.start));
                }
            }
        }
    }

    #[test]
    fn local_reflow_rejects_edits_that_can_change_neighbouring_markdown_blocks() {
        for (source, replacement) in [
            ("text\n\nnext", "text\nnew\n\nnext"),
            ("text\n\nnext", "```rust\n\nnext"),
            ("| a |\ntext", "| a |\n| - |"),
            ("$$\ntext\nclose", "$$\ntext\n$$"),
            (
                ":::mochi-highlight\ntext\nclose",
                ":::mochi-highlight\ntext\n:::",
            ),
        ] {
            let mut live = layout(source, None, W, &|_| None);
            assert!(!live.update_inline(replacement, W), "{replacement}");
            assert_eq!(live.source.as_ref(), source);
        }
    }

    #[test]
    fn mapping_cache_admission_keeps_existing_entries_when_the_byte_limit_is_reached() {
        // 放进第一个条目后，模拟整个字节预算的分配。这样可以在与映射
        // 压缩无关的情况下测试准入策略（纯文本现在用区间表示，
        // 不再每字符一次大分配）。
        let first_source = "稳定 **中文🙂**".to_owned();
        let second_source = "另一个 `block`".to_owned();
        let first = document::RangedBlock {
            block: Block::Paragraph(first_source.clone()),
            start: 0,
            end: first_source.len(),
        };
        let second = document::RangedBlock {
            block: Block::Paragraph(second_source.clone()),
            start: 0,
            end: second_source.len(),
        };
        let cache = MappingCache::default();
        cache.get(&first, &first_source);
        cache.0.borrow_mut().bytes = MAX_MAPPING_BYTES;
        cache.get(&second, &second_source);
        let before = cache.stats();
        cache.get(&first, &first_source);
        let after = cache.stats();
        assert_eq!(
            before.0, 1,
            "the second block should be rejected once the byte budget is full"
        );
        assert_eq!(
            after.1,
            before.1 + 1,
            "an admitted entry must remain reusable after a rejected miss"
        );
    }

    #[test]
    fn vertical_navigation_skips_bullets_and_code_headers() {
        let source = "- first\n- second\n\n```rust\ncode\n```";
        let live = layout(source, Some(2), W, &|_| None);
        let second = source.find("second").unwrap();
        let code = source.find("code").unwrap();
        assert_eq!(live.move_vertical(source, second, -1, 22.0), 2);
        assert_eq!(live.move_vertical(source, 2, 1, 22.0), second);
        let blank = source.find("\n\n").unwrap() + 1;
        assert_eq!(live.move_vertical(source, second, 1, 0.0), blank);
        assert_eq!(live.move_vertical(source, blank, 1, 0.0), code);
    }

    #[test]
    fn copy_omits_container_delimiters_and_code_metadata() {
        let source = ":::mochi-highlight title=\"Tips\"\n**正文**\n:::\n\n<!-- mochi-code-block title=\"Demo\" collapsed=\"false\" -->\n```rs\ncode\n```";
        let live = layout(source, None, W, &|_| None);
        let mut buffer = TextBuffer::new(source);
        buffer.select_all();
        let copied = live.selected_text(&buffer);
        assert!(copied.contains("Tips\n正文"), "{copied}");
        assert!(copied.contains("code"));
        for syntax in [":::", "<!--", "mochi-code-block", "```", "**"] {
            assert!(!copied.contains(syntax), "{copied}");
        }
    }

    #[test]
    fn ime_underline_follows_every_wrapped_line() {
        let mut b = TextBuffer::new("**前后**");
        b.set_cursor(5, false);
        b.set_composition("中文中文中文中文", 24);
        let shown = b.display_text().0;
        let live = layout(&shown, Some(b.display_cursor()), 65.0, &|_| None);
        let mut list = DrawList::new();
        paint_overlay(
            &mut list,
            Rect::new(0.0, 0.0, 200.0, 600.0),
            &live,
            &b,
            0.0,
            true,
            false,
            theme::tokens().palette(false),
        );
        assert!(
            list.cmds()
                .iter()
                .filter(|c| matches!(c, crate::ui::draw::DrawCmd::RectAlpha { .. }))
                .count()
                > 1
        );
        assert_eq!(b.text(), "**前后**");
    }
}
