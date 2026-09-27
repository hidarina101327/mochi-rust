//! 定义富文本内容的文本引用、包装结构和存储单元。
use super::*;

/// 一个可见字符的文字，不必为每个字符分配一个 `String`。
/// 引用要么指向映射持有的不可变源码，要么指向自有的公式字符串。
#[derive(Debug, Clone, Copy)]
pub struct TextRef<'a>(pub(super) &'a str);

impl<'a> TextRef<'a> {
    pub fn as_str(self) -> &'a str {
        self.0
    }
}

impl PartialEq for TextRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for TextRef<'_> {}

impl PartialEq<&str> for TextRef<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<str> for TextRef<'_> {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<TextRef<'_>> for &str {
    fn eq(&self, other: &TextRef<'_>) -> bool {
        *self == other.0
    }
}

impl PartialEq<TextRef<'_>> for str {
    fn eq(&self, other: &TextRef<'_>) -> bool {
        self == other.0
    }
}

#[derive(Debug, Clone)]
pub struct Unit<'a> {
    pub source: Range<usize>,
    pub visible: Range<usize>,
    pub text: TextRef<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Wrapper {
    pub(super) full: Range<usize>,
    pub(super) body: Range<usize>,
}

/// 一个可见单元的「相对源码」文字。普通字符指向 `MappingData::storage`
/// 里的区间；公式则共享一份自有字符串。
#[derive(Debug, Clone)]
pub(super) enum UnitText {
    Source(Range<usize>),
    Owned(Rc<str>),
}

#[derive(Debug, Clone)]
pub(super) struct UnitData {
    pub(super) source: Range<usize>,
    pub(super) visible: Range<usize>,
    pub(super) text: UnitText,
}

const UNIT_CHECKPOINT_STRIDE: usize = 256;

/// 一串源码支撑的普通字符。run 内所有字符都可见、在源码中相邻、
/// 且具有相同的「源码/文字」对应关系，因此用一个区间加稀疏 UTF-8
/// 检查点即可取代每字符一个 UnitData。转义字符和公式原子仍按
/// Single 存放——它们的源码与可见边界不是仿射关系。
#[derive(Debug, Clone)]
pub(super) struct SourceChunk {
    pub(super) source: Range<usize>,
    pub(super) visible: Range<usize>,
    pub(super) checkpoints: Vec<usize>,
}

#[derive(Debug, Clone)]
pub(super) enum UnitChunk {
    Source(SourceChunk),
    Single(UnitData),
}

impl SourceChunk {
    pub(super) fn unit_count(&self) -> usize {
        self.visible.end - self.visible.start
    }

    pub(super) fn finish(&mut self, storage: &str) {
        let text = &storage[self.source.clone()];
        self.checkpoints.clear();
        self.checkpoints.push(0);
        for (index, (offset, _)) in text.char_indices().enumerate() {
            if index != 0 && index % UNIT_CHECKPOINT_STRIDE == 0 {
                self.checkpoints.push(offset);
            }
        }
    }

    pub(super) fn byte_at(&self, storage: &str, unit: usize) -> usize {
        if unit >= self.unit_count() {
            return self.source.end;
        }
        let checkpoint = unit / UNIT_CHECKPOINT_STRIDE;
        let offset = self.checkpoints[checkpoint];
        let remainder = unit % UNIT_CHECKPOINT_STRIDE;
        let start = self.source.start + offset;
        let relative = storage[start..self.source.end]
            .char_indices()
            .nth(remainder)
            .map(|(at, _)| at)
            .unwrap_or(self.source.end - start);
        start + relative
    }

    pub(super) fn first_unit_with_end_after(&self, storage: &str, source: usize) -> usize {
        if source <= self.source.start {
            return 0;
        }
        if source >= self.source.end {
            return self.unit_count();
        }
        let checkpoint = self
            .checkpoints
            .partition_point(|offset| self.source.start + *offset <= source)
            .saturating_sub(1);
        let mut unit = checkpoint * UNIT_CHECKPOINT_STRIDE;
        let mut at = self.source.start + self.checkpoints[checkpoint];
        while unit < self.unit_count() {
            let end = at
                + storage[at..self.source.end]
                    .chars()
                    .next()
                    .map(char::len_utf8)
                    .unwrap_or(0);
            if end > source {
                return unit;
            }
            at = end;
            unit += 1;
        }
        unit
    }
}

impl UnitChunk {
    pub(super) fn unit_count(&self) -> usize {
        match self {
            Self::Source(chunk) => chunk.unit_count(),
            Self::Single(_) => 1,
        }
    }

    pub(super) fn visible(&self) -> Range<usize> {
        match self {
            Self::Source(chunk) => chunk.visible.clone(),
            Self::Single(unit) => unit.visible.clone(),
        }
    }

    pub(super) fn source_start(&self) -> usize {
        match self {
            Self::Source(chunk) => chunk.source.start,
            Self::Single(unit) => unit.source.start,
        }
    }

    pub(super) fn source_end(&self) -> usize {
        match self {
            Self::Source(chunk) => chunk.source.end,
            Self::Single(unit) => unit.source.end,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct UnitBuilder {
    pub(super) chunks: Vec<UnitChunk>,
    pub(super) unit_count: usize,
}

impl UnitBuilder {
    pub(super) fn push_source(&mut self, source: Range<usize>, visible: Range<usize>) {
        if let Some(UnitChunk::Source(chunk)) = self.chunks.last_mut() {
            if chunk.source.end == source.start && chunk.visible.end == visible.start {
                chunk.source.end = source.end;
                chunk.visible.end = visible.end;
                self.unit_count += 1;
                return;
            }
        }
        self.chunks.push(UnitChunk::Source(SourceChunk {
            source,
            visible,
            checkpoints: Vec::new(),
        }));
        self.unit_count += 1;
    }

    pub(super) fn push_single(&mut self, unit: UnitData) {
        self.chunks.push(UnitChunk::Single(unit));
        self.unit_count += 1;
    }

    pub(super) fn finish(mut self, storage: &str) -> (Vec<UnitChunk>, usize) {
        for chunk in &mut self.chunks {
            if let UnitChunk::Source(chunk) = chunk {
                chunk.finish(storage);
            }
        }
        (self.chunks, self.unit_count)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WrapperData {
    pub(super) full: Range<usize>,
    pub(super) body: Range<usize>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct MappingData {
    pub(super) storage: Rc<str>,
    pub(super) chunks: Vec<UnitChunk>,
    pub(super) unit_count: usize,
    pub(super) wrappers: Vec<WrapperData>,
}

#[derive(Debug, Clone)]
pub struct UnitList {
    pub(super) data: Rc<MappingData>,
    pub(super) origin: usize,
}

impl Default for UnitList {
    fn default() -> Self {
        Self {
            data: Rc::new(MappingData::default()),
            origin: 0,
        }
    }
}

impl UnitList {
    pub(super) fn new(data: Rc<MappingData>, origin: usize) -> Self {
        Self { data, origin }
    }

    pub fn iter(&self) -> UnitIter<'_> {
        UnitIter::new(&self.data, self.origin)
    }

    pub fn is_empty(&self) -> bool {
        self.data.unit_count == 0
    }

    pub fn len(&self) -> usize {
        self.data.unit_count
    }

    pub fn first(&self) -> Option<Unit<'_>> {
        self.iter().next()
    }

    pub fn last(&self) -> Option<Unit<'_>> {
        self.iter().next_back()
    }

    pub(super) fn rebase(&mut self, delta: isize) {
        self.origin = self.origin.saturating_add_signed(delta);
    }

    pub(super) fn memory_bytes(&self) -> usize {
        self.data.memory_bytes()
    }
}

pub struct UnitIter<'a> {
    pub(super) data: &'a MappingData,
    pub(super) origin: usize,
    pub(super) front_chunk: usize,
    pub(super) front_unit: usize,
    pub(super) front_byte: usize,
    pub(super) back_chunk: usize,
    pub(super) back_unit: usize,
    pub(super) back_byte: usize,
    pub(super) remaining: usize,
}

impl<'a> UnitIter<'a> {
    pub(super) fn new(data: &'a MappingData, origin: usize) -> Self {
        let (back_chunk, back_unit) = data
            .chunks
            .len()
            .checked_sub(1)
            .map(|index| (index, data.chunks[index].unit_count()))
            .unwrap_or((0, 0));
        Self {
            data,
            origin,
            front_chunk: 0,
            front_unit: 0,
            front_byte: data
                .chunks
                .first()
                .map(UnitChunk::source_start)
                .unwrap_or(0),
            back_chunk,
            back_unit,
            back_byte: data.chunks.last().map(UnitChunk::source_end).unwrap_or(0),
            remaining: data.unit_count,
        }
    }
}

impl<'a> Iterator for UnitIter<'a> {
    type Item = Unit<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.remaining > 0
            && self.front_chunk < self.data.chunks.len()
            && self.front_unit >= self.data.chunks[self.front_chunk].unit_count()
        {
            self.front_chunk += 1;
            self.front_unit = 0;
            self.front_byte = self
                .data
                .chunks
                .get(self.front_chunk)
                .map(UnitChunk::source_start)
                .unwrap_or(0);
        }
        if self.remaining == 0 {
            return None;
        }
        let chunk = self.data.chunks.get(self.front_chunk)?;
        let unit = match chunk {
            UnitChunk::Source(chunk) => {
                let start = self.front_byte;
                let end = start
                    + self.data.storage[start..chunk.source.end]
                        .chars()
                        .next()
                        .map(char::len_utf8)?;
                self.front_byte = end;
                let source = start..end;
                Unit {
                    source: shift_range(&source, self.origin),
                    visible: chunk.visible.start + self.front_unit
                        ..chunk.visible.start + self.front_unit + 1,
                    text: TextRef(&self.data.storage[source.clone()]),
                }
            }
            UnitChunk::Single(unit) => unit.view(&self.data, self.origin),
        };
        self.front_unit += 1;
        self.remaining -= 1;
        Some(unit)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl DoubleEndedIterator for UnitIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        while self.remaining > 0 && self.back_unit == 0 {
            if self.back_chunk == 0 {
                return None;
            }
            self.back_chunk -= 1;
            self.back_unit = self.data.chunks[self.back_chunk].unit_count();
            self.back_byte = self.data.chunks[self.back_chunk].source_end();
        }
        if self.remaining == 0 {
            return None;
        }
        let chunk = self.data.chunks.get(self.back_chunk)?;
        self.back_unit -= 1;
        let unit = match chunk {
            UnitChunk::Source(chunk) => {
                let end = self.back_byte;
                let start = end
                    - self.data.storage[chunk.source.start..end]
                        .chars()
                        .next_back()
                        .map(char::len_utf8)?;
                self.back_byte = start;
                let source = start..end;
                Unit {
                    source: shift_range(&source, self.origin),
                    visible: chunk.visible.start + self.back_unit
                        ..chunk.visible.start + self.back_unit + 1,
                    text: TextRef(&self.data.storage[source.clone()]),
                }
            }
            UnitChunk::Single(unit) => unit.view(&self.data, self.origin),
        };
        self.remaining -= 1;
        Some(unit)
    }
}

impl ExactSizeIterator for UnitIter<'_> {}

impl<'a> IntoIterator for &'a UnitList {
    type Item = Unit<'a>;
    type IntoIter = UnitIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Debug, Clone)]
pub(super) struct WrapperList {
    pub(super) data: Rc<MappingData>,
    pub(super) origin: usize,
}

impl Default for WrapperList {
    fn default() -> Self {
        Self {
            data: Rc::new(MappingData::default()),
            origin: 0,
        }
    }
}

impl WrapperList {
    pub(super) fn new(data: Rc<MappingData>, origin: usize) -> Self {
        Self { data, origin }
    }

    pub(super) fn iter(&self) -> WrapperIter<'_> {
        WrapperIter {
            origin: self.origin,
            inner: self.data.wrappers.iter(),
        }
    }

    pub(super) fn rebase(&mut self, delta: isize) {
        self.origin = self.origin.saturating_add_signed(delta);
    }
}

pub(super) struct WrapperIter<'a> {
    pub(super) origin: usize,
    pub(super) inner: std::slice::Iter<'a, WrapperData>,
}

impl Iterator for WrapperIter<'_> {
    type Item = Wrapper;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|wrapper| wrapper.view(self.origin))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl DoubleEndedIterator for WrapperIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner
            .next_back()
            .map(|wrapper| wrapper.view(self.origin))
    }
}

impl ExactSizeIterator for WrapperIter<'_> {}

impl<'a> IntoIterator for &'a WrapperList {
    type Item = Wrapper;
    type IntoIter = WrapperIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl PartialEq for WrapperList {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl Eq for WrapperList {}

impl MappingData {
    pub(super) fn memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.storage.len()
            + self.chunks.capacity() * std::mem::size_of::<UnitChunk>()
            + self
                .chunks
                .iter()
                .map(|chunk| match chunk {
                    UnitChunk::Source(chunk) => {
                        chunk.checkpoints.capacity() * std::mem::size_of::<usize>()
                    }
                    UnitChunk::Single(unit) => match &unit.text {
                        UnitText::Owned(text) => text.len(),
                        UnitText::Source(_) => 0,
                    },
                })
                .sum::<usize>()
            + self.wrappers.capacity() * std::mem::size_of::<WrapperData>()
    }
}

impl UnitData {
    pub(super) fn view<'a>(&'a self, data: &'a MappingData, origin: usize) -> Unit<'a> {
        let text = match &self.text {
            UnitText::Source(range) => TextRef(&data.storage[range.clone()]),
            UnitText::Owned(text) => TextRef(text.as_ref()),
        };
        Unit {
            source: shift_range(&self.source, origin),
            visible: self.visible.clone(),
            text,
        }
    }
}

impl WrapperData {
    pub(super) fn view(&self, origin: usize) -> Wrapper {
        Wrapper {
            full: shift_range(&self.full, origin),
            body: shift_range(&self.body, origin),
        }
    }
}

fn shift_range(range: &Range<usize>, origin: usize) -> Range<usize> {
    origin + range.start..origin + range.end
}
