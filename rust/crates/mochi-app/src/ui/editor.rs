//! 源码和富文本共用缓冲区及撤销栈。内部偏移按 UTF-8 字节计，必须落在字符边界。
//! 与 IMM32、DirectWrite 交互时再转换 UTF-16 码元。

use super::draw::TextStyle;
use super::text::{self, Run};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

fn next_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// 寻找 UTF-8 安全的替换点，而不让带解释的迭代器逐字节走两份
/// 多 MB 的快照。切片相等走 memcmp；只有边界碎块需要逐字节/逐字符检查。
pub(crate) fn changed_bounds(before: &str, after: &str) -> (usize, usize, usize) {
    let (a, b) = (before.as_bytes(), after.as_bytes());
    let limit = a.len().min(b.len());
    let mut prefix = 0;
    while prefix + 64 <= limit && a[prefix..prefix + 64] == b[prefix..prefix + 64] {
        prefix += 64;
    }
    prefix += a[prefix..limit]
        .iter()
        .zip(&b[prefix..limit])
        .take_while(|(a, b)| a == b)
        .count();
    while !before.is_char_boundary(prefix) || !after.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let mut suffix = 0;
    let remaining = limit - prefix;
    while suffix + 64 <= remaining
        && a[a.len() - suffix - 64..a.len() - suffix] == b[b.len() - suffix - 64..b.len() - suffix]
    {
        suffix += 64;
    }
    suffix += a[prefix..a.len() - suffix]
        .iter()
        .rev()
        .zip(b[prefix..b.len() - suffix].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    while !before.is_char_boundary(before.len() - suffix)
        || !after.is_char_boundary(after.len() - suffix)
    {
        suffix -= 1;
    }
    (prefix, before.len() - suffix, after.len() - suffix)
}

/// 一个可编辑的文本缓冲区。
#[derive(Debug, Clone)]
pub struct TextBuffer {
    text: String,
    saved_text: std::sync::Arc<str>,
    /// 单调递增的内容代数。布局缓存比较这个 O(1) 值，
    /// 而不是每帧都对几 MB 的文档算一次哈希。
    revision: u64,
    /// 光标位置（字节偏移，恒在字符边界上）。
    cursor: usize,
    /// 选区的另一端。与 `cursor` 相等表示没有选区。
    anchor: usize,
    /// 输入法组合串。它**不在** `text` 里——组合期间反复改 `text` 会让撤销栈
    /// 塞满半成品，也会让"未提交的候选"被当成已输入内容存盘。
    composition: Option<Composition>,
    /// 内容是否被改过。
    dirty: bool,
    /// 保存实际变更与选区；连续输入/退格仍合并成一步，但不复制整篇正文。
    undo: Vec<HistoryStep>,
    redo: Vec<HistoryStep>,
    last_edit: Option<EditKind>,
}

#[derive(Debug, Clone)]
struct HistoryStep {
    changes: Vec<TextChange>,
    cursor: usize,
    anchor: usize,
}

#[derive(Debug, Clone)]
struct TextChange {
    at: usize,
    // 退格会前插完整的 UTF-8 字符。按键按住不放时，
    // VecDeque 避免反复拷贝已累计的删除文本。
    removed: VecDeque<u8>,
    inserted: String,
}

/// 上一次编辑的种类，决定下一次要不要另起一步撤销记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    /// 连续插入可打印字符（合并）。
    Typing,
    /// 连续退格（合并）。
    Backspacing,
    /// 其它：换行、粘贴、删除选区、格式化——各自成步。
    Other,
}

const UNDO_LIMIT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordClass {
    Word,
    Space,
    Cjk,
    Punct,
}

fn word_class(c: char) -> WordClass {
    if c.is_whitespace() {
        WordClass::Space
    } else if c.is_alphanumeric() && (c as u32) < 0x2E80 {
        WordClass::Word
    } else if c.is_alphanumeric() {
        WordClass::Cjk
    } else if c == '_' {
        WordClass::Word
    } else {
        WordClass::Punct
    }
}

/// 进行中的输入法组合。
#[derive(Debug, Clone, PartialEq)]
pub struct Composition {
    /// 组合串本身（例如拼音「zhongwen」或候选「中文」）。
    pub text: String,
    /// 光标在组合串里的字节偏移。
    pub cursor: usize,
    /// 在显示文字中的位置。选区替换在 commit 之前只是预览，
    /// 所以取消 IME 绝不会删掉选中的原文。
    pub start: usize,
    preview: Option<String>,
}

impl TextBuffer {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let saved_text = std::sync::Arc::from(text.as_str());
        TextBuffer {
            text,
            saved_text,
            revision: next_revision(),
            cursor: 0,
            anchor: 0,
            composition: None,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
        }
    }

    /// 一次性的富文本编辑/IME 预览只需要当前文字和选区，
    /// 而不是真实撤销历史里成百份旧文档快照。
    pub(super) fn preview(&self) -> Self {
        Self {
            text: self.text.clone(),
            saved_text: self.saved_text.clone(),
            revision: self.revision,
            cursor: self.cursor,
            anchor: self.anchor,
            composition: None,
            dirty: self.dirty,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
        }
    }

    /// 只存替换内容。分组边界与光标恢复与过去的全文快照保持一致，
    /// 包括导航之后的编辑。
    fn change(&mut self, kind: EditKind, range: std::ops::Range<usize>, with: &str) {
        // 格式化/IME 事务有时会提交整篇文档、却只改了很小的区域。
        // 历史里只保留那块区域，调用方最终的光标/选区位置不变。
        let before = &self.text[range.clone()];
        let (prefix, before_end, after_end) = changed_bounds(before, with);
        let range = range.start + prefix..range.start + before_end;
        let with = &with[prefix..after_end];
        let coalesce = kind != EditKind::Other && self.last_edit == Some(kind);
        if !coalesce || self.undo.is_empty() {
            self.undo.push(HistoryStep {
                changes: Vec::new(),
                cursor: self.cursor,
                anchor: self.anchor,
            });
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
        let changes = &mut self.undo.last_mut().unwrap().changes;
        let removed = &self.text[range.clone()];
        let merged = if let Some(previous) = changes.last_mut() {
            if kind == EditKind::Typing
                && previous.removed.is_empty()
                && range.is_empty()
                && range.start == previous.at + previous.inserted.len()
            {
                previous.inserted.push_str(with);
                true
            } else if kind == EditKind::Backspacing
                && previous.inserted.is_empty()
                && with.is_empty()
                && range.end == previous.at
            {
                for byte in removed.bytes().rev() {
                    previous.removed.push_front(byte);
                }
                previous.at = range.start;
                true
            } else {
                false
            }
        } else {
            false
        };
        if !merged {
            changes.push(TextChange {
                at: range.start,
                removed: removed.as_bytes().iter().copied().collect(),
                inserted: with.to_owned(),
            });
        }
        self.text.replace_range(range, with);
        self.revision = next_revision();
        self.last_edit = Some(kind);
        self.redo.clear();
    }

    #[cfg(test)]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(mut step) = self.undo.pop() else {
            return false;
        };
        let (cursor, anchor) = (step.cursor, step.anchor);
        step.cursor = self.cursor;
        step.anchor = self.anchor;
        for change in step.changes.iter_mut().rev() {
            let removed =
                std::str::from_utf8(change.removed.make_contiguous()).expect("recorded UTF-8 edit");
            self.text
                .replace_range(change.at..change.at + change.inserted.len(), removed);
        }
        self.redo.push(step);
        // 历史可能产生于转换或外部刷新文字之前。
        // 恢复的字节偏移绝不能落在 UTF-8 字符中间。
        self.cursor = self.clamp_boundary(cursor);
        self.anchor = self.clamp_boundary(anchor);
        self.composition = None;
        self.dirty = true;
        self.last_edit = None;
        self.revision = next_revision();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(mut step) = self.redo.pop() else {
            return false;
        };
        let (cursor, anchor) = (step.cursor, step.anchor);
        step.cursor = self.cursor;
        step.anchor = self.anchor;
        for change in &step.changes {
            self.text.replace_range(
                change.at..change.at + change.removed.len(),
                &change.inserted,
            );
        }
        self.undo.push(step);
        self.cursor = self.clamp_boundary(cursor);
        self.anchor = self.clamp_boundary(anchor);
        self.composition = None;
        self.dirty = true;
        self.last_edit = None;
        self.revision = next_revision();
        true
    }

    /// 双击选词：向两侧扩到「词」的边界。字母数字下划线连成一词；CJK 一个字一词
    /// （没有分词器，选一个字比把整句选中更接近浏览器的行为）；空白选整段空白。
    pub fn select_word(&mut self) {
        let text = &self.text;
        if text.is_empty() {
            return;
        }
        let at = self.clamp_boundary(self.cursor);
        // 光标在行尾时看前一个字符
        let probe = if at >= text.len() || text[at..].starts_with('\n') {
            match text[..at].chars().next_back() {
                Some(c) => at - c.len_utf8(),
                None => return,
            }
        } else {
            at
        };
        let ch = text[probe..].chars().next().unwrap();
        let class = word_class(ch);
        if class == WordClass::Cjk {
            self.anchor = probe;
            self.cursor = probe + ch.len_utf8();
            return;
        }
        let mut start = probe;
        while let Some(c) = text[..start].chars().next_back() {
            if word_class(c) != class || c == '\n' {
                break;
            }
            start -= c.len_utf8();
        }
        let mut end = probe;
        for c in text[probe..].chars() {
            if word_class(c) != class || c == '\n' {
                break;
            }
            end += c.len_utf8();
        }
        self.anchor = start;
        self.cursor = end;
    }

    /// 用一段新文本替换 `range`，光标落在替换文本之后。格式化操作用它——
    /// 成一步撤销记录。
    pub fn replace_range(&mut self, range: std::ops::Range<usize>, with: &str) {
        let start = self.clamp_boundary(range.start);
        let end = self.clamp_boundary(range.end).max(start);
        self.change(EditKind::Other, start..end, with);
        self.cursor = start + with.len();
        self.anchor = self.cursor;
        self.dirty = true;
    }

    /// 替换 `range` 并把选区设为 `select`（相对新文本起点）。包裹选区成粗体后要保持选中。
    pub fn replace_range_select(
        &mut self,
        range: std::ops::Range<usize>,
        with: &str,
        select: std::ops::Range<usize>,
    ) {
        self.replace_range(range.clone(), with);
        let base = self.clamp_boundary(range.start);
        self.anchor = self.clamp_boundary(base + select.start);
        self.cursor = self.clamp_boundary(base + select.end);
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 隐藏身份与创建它所属块的编辑一起持久化。
    /// 撤销/重做把用户的编辑和新 ID 放在同一个事务里。
    pub fn replace_persisted_source(&mut self, source: &str, merge_undo: bool) {
        if self.text == source {
            return;
        }
        let cursor = super::block_markers::remap_offset(&self.text, source, self.cursor);
        let anchor = super::block_markers::remap_offset(&self.text, source, self.anchor);
        let merge = merge_undo && !self.undo.is_empty();
        self.replace_range_select(0..self.text.len(), source, anchor..cursor);
        if merge && self.undo.len() > 1 {
            let step = self.undo.pop().unwrap();
            self.undo.last_mut().unwrap().changes.extend(step.changes);
        }
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// 存盘成功后清掉脏标记。**只在写盘真的成功之后调**——
    /// 提前清掉的话，写失败时用户会以为已经保存了。
    pub fn mark_saved(&mut self) {
        self.dirty = false;
        if self.saved_text.as_ref() != self.text {
            self.saved_text = std::sync::Arc::from(self.text.as_str());
        }
    }
    /// 正文已写入，但格式迁移尚未完成；更新冲突基线，保留待保存状态以便重试。
    pub fn mark_written_pending_migration(&mut self) {
        self.mark_saved();
        self.dirty = true;
    }
    pub fn matches_saved(&self, text: &str) -> bool {
        self.saved_text.as_ref() == text
    }

    #[allow(dead_code)]
    pub fn composition(&self) -> Option<&Composition> {
        self.composition.as_ref()
    }

    /// 选区的字节范围（起点 <= 终点）。没有选区时两端相等。
    pub fn selection(&self) -> (usize, usize) {
        if self.cursor <= self.anchor {
            (self.cursor, self.anchor)
        } else {
            (self.anchor, self.cursor)
        }
    }

    pub fn has_selection(&self) -> bool {
        self.cursor != self.anchor
    }

    /// 选中的文本。复制/剪切会用它。
    #[allow(dead_code)]
    pub fn selected_text(&self) -> &str {
        let (a, b) = self.selection();
        &self.text[a..b]
    }

    /// 把光标放到 `offset`（会夹到字符边界）。`extend` 为真时保留选区起点。
    pub fn set_cursor(&mut self, offset: usize, extend: bool) {
        let offset = self.clamp_boundary(offset);
        if self.cursor != offset || (!extend && self.anchor != offset) {
            self.last_edit = None;
        }
        self.cursor = offset;
        if !extend {
            self.anchor = self.cursor;
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    /// 插入文本。有选区时先删掉选区——这是所有编辑器的共同行为。
    pub fn insert(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        // 单个可打印字符连着敲合并成一步；换行、粘贴、替换选区各自成步
        let typing =
            !self.has_selection() && s.chars().count() == 1 && !s.chars().all(char::is_whitespace);
        let (start, end) = self.selection();
        self.change(
            if typing {
                EditKind::Typing
            } else {
                EditKind::Other
            },
            start..end,
            s,
        );
        self.cursor = start + s.len();
        self.anchor = self.cursor;
        self.dirty = true;
    }

    /// 剪贴板等显式事务不能与打字合并成一组。
    pub fn break_undo_group(&mut self) {
        self.last_edit = None;
    }

    /// 退格。有选区时删选区，否则删光标前一个**字符**（不是一个字节）。
    pub fn delete_backward(&mut self) {
        if self.has_selection() {
            let (start, end) = self.selection();
            self.change(EditKind::Other, start..end, "");
            self.cursor = start;
            self.anchor = start;
            self.dirty = true;
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let start = self.prev_boundary(self.cursor);
        self.change(EditKind::Backspacing, start..self.cursor, "");
        self.cursor = start;
        self.anchor = start;
        self.dirty = true;
    }

    pub fn delete_forward(&mut self) {
        if self.has_selection() {
            let (start, end) = self.selection();
            self.change(EditKind::Other, start..end, "");
            self.cursor = start;
            self.anchor = start;
            self.dirty = true;
            return;
        }
        if self.cursor >= self.text.len() {
            return;
        }
        let end = self.next_boundary(self.cursor);
        self.change(EditKind::Other, self.cursor..end, "");
        self.dirty = true;
    }

    // IME 组合串不进入 text，只叠加绘制，避免进入撤销栈或被自动保存。
    pub fn set_composition(&mut self, text: &str, cursor: usize) {
        if text.is_empty() {
            self.composition = None;
            return;
        }
        let mut cursor = cursor.min(text.len());
        while !text.is_char_boundary(cursor) {
            cursor -= 1;
        }
        self.composition = Some(Composition {
            text: text.to_owned(),
            cursor,
            start: self.selection().0,
            preview: None,
        });
    }

    pub fn composition_preview(&mut self, preview: String, start: usize) {
        if let Some(c) = self.composition.as_mut() {
            c.preview = Some(preview);
            c.start = start;
        }
    }

    /// 提交组合串：把它真正插进文本。
    pub fn commit_composition(&mut self, text: &str) {
        self.composition = None;
        self.insert(text);
    }

    /// 取消组合（按 Esc 或切走焦点）。组合串直接丢弃，文本不变。
    pub fn cancel_composition(&mut self) {
        self.composition = None;
    }

    /// 绘制用：组合串插进去之后的那一行文本长什么样。
    ///
    /// 返回 (整段文本, 组合串在其中的字节范围)。没有组合时范围为空。
    pub fn display_text(&self) -> (std::borrow::Cow<'_, str>, Option<(usize, usize)>) {
        use std::borrow::Cow;
        match &self.composition {
            Some(c) => {
                let s = if let Some(preview) = &c.preview {
                    Cow::Borrowed(preview.as_str())
                } else {
                    let mut s = self.text.clone();
                    let (a, b) = self.selection();
                    s.replace_range(a..b, &c.text);
                    Cow::Owned(s)
                };
                (s, Some((c.start, c.start + c.text.len())))
            }
            // 光标闪烁和打字不能为了拿一份相同的文字就把整个文档复制一遍。
            // 只有 IME 替换才需要缓冲。
            None => (Cow::Borrowed(self.text.as_str()), None),
        }
    }

    /// 光标在**显示文本**里的位置。组合期间光标在组合串内部。
    pub fn display_cursor(&self) -> usize {
        match &self.composition {
            Some(c) => c.start + c.cursor,
            None => self.cursor,
        }
    }

    /// 把 [`Self::display_text`] 里的字节偏移换算回已提交的源码偏移。
    /// IME 预览可能通过富文本编辑器替换选区，所以显示出的变化
    /// 并不总是 `composition.text` 的原样粘贴；需要从公共的
    /// UTF-8 前缀/后缀推导出真正变化的区间。
    pub fn source_offset_from_display(&self, offset: usize) -> usize {
        if self.composition.is_none() {
            return self.clamp_boundary(offset);
        }
        let (display, _) = self.display_text();
        let display = display.as_ref();
        let source = self.text.as_str();

        let mut prefix = source
            .bytes()
            .zip(display.bytes())
            .take_while(|(left, right)| left == right)
            .count();
        while !source.is_char_boundary(prefix) || !display.is_char_boundary(prefix) {
            prefix -= 1;
        }

        let source_tail = &source[prefix..];
        let display_tail = &display[prefix..];
        let mut suffix = source_tail
            .bytes()
            .rev()
            .zip(display_tail.bytes().rev())
            .take_while(|(left, right)| left == right)
            .count();
        while !source.is_char_boundary(source.len() - suffix)
            || !display.is_char_boundary(display.len() - suffix)
        {
            suffix -= 1;
        }

        let display_end = display.len() - suffix;
        let source_end = source.len() - suffix;
        let offset = offset.min(display.len());
        let mapped = if offset <= prefix {
            offset
        } else if offset >= display_end {
            source_end + offset - display_end
        } else if offset - prefix <= display_end - offset {
            prefix
        } else {
            source_end
        };
        self.clamp_boundary(mapped)
    }

    // ---------- 字符边界 ----------

    fn clamp_boundary(&self, offset: usize) -> usize {
        let offset = offset.min(self.text.len());
        if offset > 0 && self.text.as_bytes().get(offset - 1..offset + 1) == Some(b"\r\n") {
            return offset - 1;
        }
        if self.text.is_char_boundary(offset) {
            offset
        } else {
            self.prev_boundary(offset)
        }
    }

    fn prev_boundary(&self, from: usize) -> usize {
        let mut i = from.saturating_sub(1);
        while i > 0 && !self.text.is_char_boundary(i) {
            i -= 1;
        }
        if i > 0 && self.text.as_bytes().get(i - 1..i + 1) == Some(b"\r\n") {
            i -= 1;
        }
        i
    }

    fn next_boundary(&self, from: usize) -> usize {
        let mut i = (from + 1).min(self.text.len());
        while i < self.text.len() && !self.text.is_char_boundary(i) {
            i += 1;
        }
        if i > 0 && self.text.as_bytes().get(i - 1..i + 1) == Some(b"\r\n") {
            i += 1;
        }
        i
    }

    /// 光标左移一个字符。
    pub fn move_left(&mut self, extend: bool) {
        let next = self.prev_boundary(self.cursor);
        self.set_cursor(next, extend);
    }

    pub fn move_right(&mut self, extend: bool) {
        let next = self.next_boundary(self.cursor);
        self.set_cursor(next, extend);
    }

    /// 字节偏移 → UTF-16 码元偏移。给 IMM32 和 DirectWrite 用。
    #[allow(dead_code)]
    pub fn utf16_offset(&self, byte_offset: usize) -> usize {
        self.text[..byte_offset.min(self.text.len())]
            .encode_utf16()
            .count()
    }
}

/// `utf16_offset` 的逆；IMM32 偏移不能直接用于 Rust 字符串切片。
pub fn byte_offset_from_utf16(text: &str, units: usize) -> usize {
    let mut consumed = 0usize;
    for (byte, ch) in text.char_indices() {
        if consumed >= units {
            return byte;
        }
        consumed += ch.len_utf16();
    }
    text.len()
}

/// 排好版的编辑面：视觉行 + 偏移映射。
#[derive(Debug, Clone, Default)]
pub struct EditorLayout {
    pub lines: Vec<VisualLine>,
    pub height: f32,
}

/// 一个视觉行。`start` / `end` 是它在**显示文本**里的字节范围。
#[derive(Debug, Clone, PartialEq)]
pub struct VisualLine {
    pub runs: Vec<Run>,
    pub start: usize,
    pub end: usize,
    /// 这一行是不是由硬换行结束的（而不是软换行）。
    /// 决定光标能不能停在行尾之后。
    pub hard_break: bool,
    pub y: f32,
}

/// 把文本排成视觉行。
///
/// 与文档视图共用 `text::wrap_runs`，但这里必须**额外记住每行的字节范围**——
/// 没有它就没法把光标偏移映射到屏幕位置，也没法把点击映射回偏移。
pub fn layout(text: &str, style: TextStyle, max_width: f32) -> EditorLayout {
    let mut lines = Vec::new();
    let mut y = 0.0;
    let mut offset = 0;

    // 先按硬换行切段，段内再软换行。两步分开做，才知道哪个行尾是硬换行。
    for (i, para) in text.split('\n').enumerate() {
        if i > 0 {
            offset += 1; // 跳过 '\n' 本身
        }
        // **不做行内解析**：`parse_inline` 会把 `**` 这类标记符吃掉，
        // 于是行的字节范围就和源码对不上了——光标会整体偏移。
        // 编辑器改的是源码，偏移必须与源码 1:1。
        let source_len = para.len();
        let para = para.strip_suffix('\r').unwrap_or(para);
        let wrapped = text::wrap_source(para, style, max_width);
        let count = wrapped.len();
        let mut local = 0usize;
        for (j, line_runs) in wrapped.into_iter().enumerate() {
            let len: usize = line_runs.iter().map(|r| r.text.len()).sum();
            lines.push(VisualLine {
                runs: line_runs,
                start: offset + local,
                end: offset + local + len,
                hard_break: j + 1 == count,
                y,
            });
            local += len;
            y += style.line_height();
        }
        offset += source_len;
    }

    EditorLayout { lines, height: y }
}

impl EditorLayout {
    /// 字节偏移 → (视觉行下标, 行内 x 偏移)。
    pub fn locate(&self, offset: usize, style: TextStyle) -> (usize, f32) {
        for (i, line) in self.lines.iter().enumerate() {
            // 落在行尾时归**这一行**而不是下一行的行首——
            // 否则在软换行处按 End，光标会跳到下一行开头，看着像没反应
            if offset <= line.end {
                let local = offset.saturating_sub(line.start);
                let full = self.lines[i]
                    .runs
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<String>();
                if let Some(shape) =
                    super::measurement::shape(&full, style, super::text::Emphasis::None)
                {
                    return (i, shape.caret(local));
                }
                let prefix = self.prefix_text(i, local);
                return (i, text::measure(&prefix, style));
            }
        }
        let last = self.lines.len().saturating_sub(1);
        let x = self
            .lines
            .last()
            .map(|l| text::measure_runs(&l.runs, style))
            .unwrap_or(0.0);
        (last, x)
    }

    /// (视觉行下标, 行内 x) → 字节偏移。点击定位用。
    pub fn offset_at(&self, line_index: usize, x: f32, style: TextStyle) -> usize {
        let Some(line) = self.lines.get(line_index) else {
            return self.lines.last().map(|l| l.end).unwrap_or(0);
        };
        let full: String = line.runs.iter().map(|r| r.text.as_str()).collect();
        if let Some(shape) = super::measurement::shape(&full, style, super::text::Emphasis::None) {
            return line.start + shape.hit(x);
        }
        let mut walked = 0.0;
        let mut byte = 0;
        for ch in full.chars() {
            let w = style.advance_em(ch) * style.font_size();
            // 过了字符中线就算落在它右边——这是所有编辑器的手感，
            // 按左边界判定的话点字的右半边光标会跑到左边去
            if x < walked + w / 2.0 {
                return line.start + byte;
            }
            walked += w;
            byte += ch.len_utf8();
        }
        line.start + byte
    }

    /// 点击的 y 落在第几行。
    pub fn line_at(&self, y: f32, style: TextStyle) -> usize {
        if y < 0.0 {
            return 0;
        }
        let index = (y / style.line_height()) as usize;
        index.min(self.lines.len().saturating_sub(1))
    }

    /// 上下移动光标：保持 x 不变，换一行重新定位。
    ///
    /// `desired_x` 是**跨多次上下移动保持不变**的目标横坐标。不记住它的话，
    /// 从长行移到短行再移回来，光标会永久停在短行的行尾。
    pub fn move_vertical(
        &self,
        offset: usize,
        delta: i32,
        desired_x: f32,
        style: TextStyle,
    ) -> usize {
        let (line, _) = self.locate(offset, style);
        let target = line as i32 + delta;
        if target < 0 {
            return self.lines.first().map_or(0, |line| line.start);
        }
        if target >= self.lines.len() as i32 {
            return self.lines.last().map_or(0, |line| line.end);
        }
        self.offset_at(target as usize, desired_x, style)
    }

    fn prefix_text(&self, line: usize, local: usize) -> String {
        let full: String = self.lines[line]
            .runs
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        full.get(..local.min(full.len()))
            .unwrap_or(&full)
            .to_owned()
    }

    /// 某一行的行首/行尾偏移。Home / End 用。
    pub fn line_bounds(&self, line: usize) -> (usize, usize) {
        match self.lines.get(line) {
            Some(l) => (l.start, l.end),
            None => (0, 0),
        }
    }
}

#[cfg(test)]
mod source_mapping_regressions {
    use super::*;
    #[test]
    fn navigation_and_delete_never_split_crlf() {
        let source = "中\r\n文";
        let mut buffer = TextBuffer::new(source);
        buffer.set_cursor(3, false);
        buffer.move_right(false);
        assert_eq!(buffer.cursor(), 5);
        buffer.move_left(false);
        assert_eq!(buffer.cursor(), 3);
        buffer.set_cursor(4, false);
        assert_eq!(buffer.cursor(), 3);
        buffer.delete_forward();
        assert_eq!(buffer.text(), "中文");
        buffer.undo();
        assert_eq!(buffer.text(), source);
        buffer.set_cursor(5, false);
        buffer.delete_backward();
        assert_eq!(buffer.text(), "中文");
        buffer.undo();
        assert_eq!(buffer.text(), source);
    }
    #[test]
    fn soft_wrapped_source_preserves_spaces_and_crlf_byte_positions() {
        let source = "  alpha beta  中文 😀 gamma   delta\r\n  第二行 words words  \r\n末尾";
        for width in [42.0, 80.0, 130.0] {
            let laid = layout(source, TextStyle::Document, width);
            for line in &laid.lines {
                let drawn = line
                    .runs
                    .iter()
                    .map(|r| r.text.as_str())
                    .collect::<String>();
                assert_eq!(
                    drawn,
                    &source[line.start..line.end],
                    "range mismatch at {}",
                    line.start
                );
            }
            for (index, line) in laid.lines.iter().enumerate() {
                let clicked = laid.offset_at(index, 0.0, TextStyle::Document);
                assert_eq!(clicked, line.start);
                let mut buffer = TextBuffer::new(source);
                buffer.set_cursor(clicked, false);
                buffer.insert("验");
                assert_eq!(
                    buffer.text(),
                    format!("{}验{}", &source[..line.start], &source[line.start..])
                );
                buffer.undo();
                assert_eq!(buffer.text(), source);
            }
            let drawn = laid
                .lines
                .iter()
                .flat_map(|l| l.runs.iter())
                .map(|r| r.text.as_str())
                .collect::<String>();
            assert_eq!(
                drawn,
                source.replace("\r\n", ""),
                "source whitespace was lost"
            );
        }
    }
}

/// 画编辑面。`scroll` 是已滚过的像素。
///
/// 组合串画成**带下划线的高亮**而不是普通文字：用户必须一眼看出哪几个字还没提交，
/// 否则会以为已经输入完了。这是所有输入法宿主的共同约定。
#[allow(clippy::too_many_arguments)]
pub fn paint(
    list: &mut super::draw::DrawList,
    area: super::layout::Rect,
    buffer: &TextBuffer,
    lay: &EditorLayout,
    style: TextStyle,
    scroll: f32,
    focused: bool,
    p: &super::theme::Palette,
) {
    use super::draw::TextStyle as _TS;
    use super::layout::Rect;
    let _ = _TS::Body;

    if area.is_empty() {
        return;
    }
    list.push_clip(area);

    let origin_x = area.left + EDITOR_PADDING;
    let top = area.top + EDITOR_PADDING - scroll;
    let (_, comp_range) = buffer.display_text();
    let (sel_start, sel_end) = buffer.selection();

    for line in &lay.lines {
        let y = top + line.y;
        if y + style.line_height() < area.top || y > area.bottom {
            continue;
        }

        // 选区底色。逐行画——跨行选区在每一行上各覆盖一段
        if sel_end > sel_start && buffer.composition().is_none() {
            let a = sel_start.max(line.start);
            let b = sel_end.min(line.end);
            if a < b || (sel_start <= line.end && sel_end > line.end) {
                let x1 = origin_x + offset_x(lay, line, a.max(line.start), style);
                // 选区跨过行尾时，高亮延伸半个字宽示意换行也被选中
                let x2 = if sel_end > line.end {
                    origin_x + text::measure_runs(&line.runs, style) + style.font_size() * 0.4
                } else {
                    origin_x + offset_x(lay, line, b, style)
                };
                if x2 > x1 {
                    list.rect(
                        Rect::new(x1, y, x2, y + style.line_height()),
                        super::theme::mix(p.accent, p.area_main_default, 0.22),
                    );
                }
            }
        }

        // 组合串底色 + 下划线
        if let Some((cs, ce)) = comp_range {
            let a = cs.max(line.start);
            let b = ce.min(line.end);
            if a < b {
                let x1 = origin_x + offset_x(lay, line, a, style);
                let x2 = origin_x + offset_x(lay, line, b, style);
                let bottom = y + style.line_height();
                list.rect(
                    Rect::new(x1, y, x2, bottom),
                    super::theme::mix(p.accent, p.area_main_default, 0.14),
                );
                list.rect(Rect::new(x1, bottom - 2.0, x2, bottom - 1.0), p.accent);
            }
        }

        let mut x = origin_x;
        for run in &line.runs {
            if run.text.is_empty() {
                continue;
            }
            let w = text::measure_runs(std::slice::from_ref(run), style);
            list.text(
                Rect::new(x, y, (x + w).min(area.right), y + style.line_height()),
                run.text.clone(),
                style,
                p.foreground,
            );
            x += w;
        }
    }

    // 光标。没有焦点时不画——两个面板同时闪光标会让人不知道往哪打字
    if focused {
        let (line, cx) = lay.locate(buffer.display_cursor(), style);
        if let Some(l) = lay.lines.get(line) {
            let y = top + l.y;
            if y + style.line_height() >= area.top && y <= area.bottom {
                list.caret(
                    Rect::new(
                        origin_x + cx,
                        y,
                        origin_x + cx + CARET_WIDTH,
                        y + style.line_height(),
                    ),
                    p.foreground,
                );
            }
        }
    }

    list.pop_clip();
}

/// 光标/选区在行内的横向偏移。
fn offset_x(_lay: &EditorLayout, line: &VisualLine, offset: usize, style: TextStyle) -> f32 {
    let full: String = line.runs.iter().map(|r| r.text.as_str()).collect();
    let local = offset.saturating_sub(line.start).min(full.len());
    text::caret_x(&full, full.floor_char_boundary_compat(local), style)
}

/// `str::floor_char_boundary` 还没稳定，自己来。
trait FloorBoundary {
    fn floor_char_boundary_compat(&self, index: usize) -> usize;
}

impl FloorBoundary for String {
    fn floor_char_boundary_compat(&self, index: usize) -> usize {
        let mut i = index.min(self.len());
        while i > 0 && !self.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

/// 编辑面的内边距。与只读文档视图同口径。
pub const EDITOR_PADDING: f32 = 24.0;
/// 光标宽度。1px 在高 DPI 下太细看不见，2px 是各家编辑器的常见取值。
pub const CARET_WIDTH: f32 = 2.0;

/// 内容区可用宽度。
pub fn content_width(area: super::layout::Rect) -> f32 {
    (area.width() - EDITOR_PADDING * 2.0).max(40.0)
}

/// 滚动上界。
pub fn max_scroll(lay: &EditorLayout, area: super::layout::Rect) -> f32 {
    (lay.height + EDITOR_PADDING * 2.0 - area.height()).max(0.0)
}

/// 光标在屏幕上的矩形。输入法候选窗要贴着它弹出——
/// 不告诉系统的话，候选框会固定在窗口左上角，离用户打字的地方十万八千里。
pub fn caret_rect(
    area: super::layout::Rect,
    lay: &EditorLayout,
    buffer: &TextBuffer,
    style: TextStyle,
    scroll: f32,
) -> super::layout::Rect {
    let (line, cx) = lay.locate(buffer.display_cursor(), style);
    let y = area.top + EDITOR_PADDING - scroll + lay.lines.get(line).map(|l| l.y).unwrap_or(0.0);
    super::layout::Rect::new(
        area.left + EDITOR_PADDING + cx,
        y,
        area.left + EDITOR_PADDING + cx + CARET_WIDTH,
        y + style.line_height(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_change_bounds_match_bytewise_utf8_safe_replacements() {
        for padding in [0, 1, 63, 64, 65, 257] {
            for (a, b) in [
                ("中文🙂", "中文🙃"),
                ("a", ""),
                ("", "中"),
                ("same", "same"),
                ("abc", "xyz"),
            ] {
                let before = format!("{}{a}{}", "x".repeat(padding), "尾".repeat(padding));
                let after = format!("{}{b}{}", "x".repeat(padding), "尾".repeat(padding));
                let (start, old_end, new_end) = changed_bounds(&before, &after);
                assert_eq!(&before[..start], &after[..start]);
                assert_eq!(&before[old_end..], &after[new_end..]);
                let mut replaced = before.clone();
                replaced.replace_range(start..old_end, &after[start..new_end]);
                assert_eq!(replaced, after);
            }
        }
    }

    const S: TextStyle = TextStyle::Body;

    #[test]
    fn large_notes_keep_two_hundred_undo_steps_without_two_hundred_note_copies() {
        let source = "原文 unchanged\r\n".repeat(16_384);
        let mut buffer = TextBuffer::new(source.clone());
        for _ in 0..250 {
            buffer.set_cursor(0, false);
            buffer.insert("x");
        }
        assert_eq!(buffer.undo.len(), UNDO_LIMIT);
        let retained: usize = buffer
            .undo
            .iter()
            .flat_map(|step| &step.changes)
            .map(|change| change.removed.capacity() + change.inserted.capacity())
            .sum();
        assert!(
            retained < 16 * 1024,
            "small edits retained {retained} bytes of text history"
        );
        for _ in 0..UNDO_LIMIT {
            assert!(buffer.undo());
        }
        assert!(!buffer.undo());
        assert_eq!(buffer.text(), format!("{}{}", "x".repeat(50), source));
        for _ in 0..UNDO_LIMIT {
            assert!(buffer.redo());
        }
        assert!(!buffer.redo());
        assert_eq!(buffer.text(), format!("{}{}", "x".repeat(250), source));
    }

    #[test]
    fn whole_note_transactions_store_the_difference_and_restore_reverse_selections() {
        let source = format!("{}甲😀乙\r\n尾", "正文\n".repeat(4096));
        let mut buffer = TextBuffer::new(&source);
        let at = source.find("甲😀乙").unwrap();
        buffer.set_cursor(at + "甲😀乙".len(), false);
        buffer.set_cursor(at, true);
        let before_selection = (buffer.cursor, buffer.anchor);
        let next = source.replace("甲😀乙", "甲🙂乙");
        buffer.replace_range_select(0..source.len(), &next, at..at + "甲🙂乙".len());
        let after_selection = (buffer.cursor, buffer.anchor);
        let change = &buffer.undo[0].changes[0];
        assert!(change.removed.len() + change.inserted.len() <= 8);
        assert!(buffer.undo());
        assert_eq!(buffer.text(), source);
        assert_eq!((buffer.cursor, buffer.anchor), before_selection);
        assert!(buffer.redo());
        assert_eq!(buffer.text(), next);
        assert_eq!((buffer.cursor, buffer.anchor), after_selection);
    }

    #[test]
    fn long_unicode_typing_and_backspacing_remain_two_compact_history_groups() {
        let mut buffer = TextBuffer::new("前缀\r\n");
        buffer.set_cursor(buffer.text().len(), false);
        for _ in 0..10_000 {
            buffer.insert("🙂");
        }
        let typed = buffer.text().to_owned();
        for _ in 0..10_000 {
            buffer.delete_backward();
        }
        assert_eq!(buffer.text(), "前缀\r\n");
        assert_eq!(buffer.undo.len(), 2);
        assert!(buffer.undo.iter().all(|step| step.changes.len() == 1));
        assert!(buffer.undo());
        assert_eq!(buffer.text(), typed);
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "前缀\r\n");
        assert!(buffer.redo());
        assert_eq!(buffer.text(), typed);
        assert!(buffer.redo());
        assert_eq!(buffer.text(), "前缀\r\n");
    }

    #[test]
    fn consecutive_typing_undoes_as_one_step_but_a_newline_starts_another() {
        let mut b = TextBuffer::new("");
        for ch in ["a", "b", "c"] {
            b.insert(ch);
        }
        b.insert("\n");
        b.insert("d");
        assert_eq!(b.text(), "abc\nd");
        assert!(b.undo());
        assert_eq!(b.text(), "abc\n", "撤销一步只退掉换行之后敲的字");
        assert!(b.undo());
        assert_eq!(b.text(), "abc", "换行自己是一步");
        assert!(b.undo());
        assert_eq!(b.text(), "", "连续敲的 abc 合并成一步");
        assert!(!b.undo());
        // 重做按原路回去
        assert!(b.redo());
        assert_eq!(b.text(), "abc");
        assert_eq!(b.cursor(), 3);
    }

    #[test]
    fn a_fresh_edit_after_undo_discards_the_redo_stack() {
        let mut b = TextBuffer::new("x");
        b.set_cursor(1, false);
        b.insert("y");
        b.undo();
        assert!(b.can_redo());
        b.insert("z");
        assert!(!b.can_redo());
        assert_eq!(b.text(), "xz");
    }

    #[test]
    fn backspaces_coalesce_and_deleting_a_selection_is_its_own_step() {
        let mut b = TextBuffer::new("abcdef");
        b.set_cursor(6, false);
        b.delete_backward();
        b.delete_backward();
        assert_eq!(b.text(), "abcd");
        b.set_cursor(0, false);
        b.set_cursor(2, true);
        b.delete_backward();
        assert_eq!(b.text(), "cd");
        b.undo();
        assert_eq!(b.text(), "abcd");
        b.undo();
        assert_eq!(b.text(), "abcdef", "两次退格是一步");
    }

    #[test]
    fn double_click_selects_a_latin_word_or_a_single_cjk_character() {
        let mut b = TextBuffer::new("hello_world 你好 foo\nbar");
        b.set_cursor(2, false);
        b.select_word();
        assert_eq!(b.selected_text(), "hello_world");
        // 中文：一个字
        let ni = "hello_world ".len();
        b.set_cursor(ni + 3, false);
        b.select_word();
        assert_eq!(b.selected_text(), "好");
        // 行尾：看前一个词，不越过换行
        let eol = "hello_world 你好 foo".len();
        b.set_cursor(eol, false);
        b.select_word();
        assert_eq!(b.selected_text(), "foo");
        // 空白：选整段空白
        b.set_cursor("hello_world".len(), false);
        b.select_word();
        assert_eq!(b.selected_text(), " ");
    }

    #[test]
    fn replace_range_select_keeps_the_wrapped_text_selected() {
        let mut b = TextBuffer::new("加粗这个词");
        b.replace_range_select(6..12, "**这个**", 2..8);
        assert_eq!(b.text(), "加粗**这个**词");
        assert_eq!(b.selection(), (8, 14));
        assert_eq!(b.selected_text(), "这个");
        b.undo();
        assert_eq!(b.text(), "加粗这个词");
    }

    fn buf(text: &str) -> TextBuffer {
        TextBuffer::new(text)
    }

    #[test]
    fn inserting_advances_the_cursor_past_what_was_typed() {
        let mut b = buf("");
        b.insert("你好");
        assert_eq!(b.text(), "你好");
        assert_eq!(b.cursor(), "你好".len());
        assert!(b.dirty());
    }

    #[test]
    fn backspace_removes_one_character_not_one_byte() {
        // 中文一个字三个字节；按字节删会把它劈成乱码
        let mut b = buf("你好");
        b.set_cursor(b.text().len(), false);
        b.delete_backward();
        assert_eq!(b.text(), "你");
        b.delete_backward();
        assert_eq!(b.text(), "");
    }

    #[test]
    fn backspace_at_the_start_does_nothing() {
        let mut b = buf("abc");
        b.set_cursor(0, false);
        b.delete_backward();
        assert_eq!(b.text(), "abc");
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn delete_forward_removes_one_character_and_leaves_the_cursor_put() {
        let mut b = buf("你好");
        b.set_cursor(0, false);
        b.delete_forward();
        assert_eq!(b.text(), "好");
        assert_eq!(b.cursor(), 0);
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut b = buf("abcdef");
        b.set_cursor(1, false);
        b.set_cursor(4, true);
        assert_eq!(b.selected_text(), "bcd");
        b.insert("X");
        assert_eq!(b.text(), "aXef");
        assert!(!b.has_selection());
    }

    #[test]
    fn backspace_with_a_selection_deletes_the_selection_not_one_char() {
        let mut b = buf("abcdef");
        b.set_cursor(1, false);
        b.set_cursor(4, true);
        b.delete_backward();
        assert_eq!(b.text(), "aef");
        assert_eq!(b.cursor(), 1);
    }

    #[test]
    fn a_cursor_landing_mid_character_is_snapped_to_a_boundary() {
        // 光标停在多字节字符中间会让后续所有切片 panic
        let mut b = buf("中文");
        b.set_cursor(1, false); // "中" 的第二个字节
        assert_eq!(b.cursor(), 0);
        b.set_cursor(4, false);
        assert_eq!(b.cursor(), 3);
    }

    #[test]
    fn arrow_keys_move_by_character() {
        let mut b = buf("中a文");
        b.set_cursor(0, false);
        b.move_right(false);
        assert_eq!(b.cursor(), 3);
        b.move_right(false);
        assert_eq!(b.cursor(), 4);
        b.move_left(false);
        assert_eq!(b.cursor(), 3);
    }

    #[test]
    fn shift_arrow_extends_the_selection() {
        let mut b = buf("abcd");
        b.set_cursor(0, false);
        b.move_right(true);
        b.move_right(true);
        assert_eq!(b.selected_text(), "ab");
        // 不带 shift 再动一次，选区收起
        b.move_right(false);
        assert!(!b.has_selection());
    }

    #[test]
    fn select_all_covers_the_whole_buffer() {
        let mut b = buf("中文 mixed");
        b.select_all();
        assert_eq!(b.selected_text(), "中文 mixed");
    }

    // ---------- 输入法 ----------

    #[test]
    fn a_composition_is_not_written_into_the_buffer() {
        // 组合期间写进去的话，自动保存会把未提交的拼音存进文件
        let mut b = buf("");
        b.set_composition("zhongwen", 8);
        assert_eq!(b.text(), "", "组合串不该进缓冲区");
        assert!(!b.dirty(), "只是在组合，还没改动内容");
        assert_eq!(b.composition().map(|c| c.text.as_str()), Some("zhongwen"));
    }

    #[test]
    fn committing_a_composition_inserts_the_final_text() {
        let mut b = buf("");
        b.set_composition("zhongwen", 8);
        b.commit_composition("中文");
        assert_eq!(b.text(), "中文");
        assert_eq!(b.cursor(), "中文".len());
        assert!(b.composition().is_none());
        assert!(b.dirty());
    }

    #[test]
    fn cancelling_a_composition_leaves_the_text_untouched() {
        let mut b = buf("原文");
        b.set_cursor(b.text().len(), false);
        b.set_composition("pinyin", 6);
        b.cancel_composition();
        assert_eq!(b.text(), "原文");
        assert!(b.composition().is_none());
    }

    #[test]
    fn composing_over_selection_defers_replacement_until_commit() {
        // 选中一段再用输入法打字，应该替换掉选中的内容
        let mut b = buf("abcdef");
        b.set_cursor(1, false);
        b.set_cursor(4, true);
        b.set_composition("zw", 2);
        assert_eq!(b.text(), "abcdef", "未提交的输入不能删除原文");
        assert_eq!(b.display_text().0, "azwef");
        assert!(!b.dirty());
        b.commit_composition("中文");
        assert_eq!(b.text(), "a中文ef");
    }

    #[test]
    fn the_display_text_shows_the_composition_inline_at_the_cursor() {
        let mut b = buf("前后");
        b.set_cursor("前".len(), false);
        b.set_composition("zw", 2);
        let (shown, range) = b.display_text();
        assert_eq!(shown, "前zw后");
        assert_eq!(range, Some(("前".len(), "前zw".len())));
        // 光标在组合串内部
        assert_eq!(b.display_cursor(), "前zw".len());
    }

    #[test]
    fn without_a_composition_the_display_text_is_just_the_buffer() {
        let b = buf("原文");
        let (shown, range) = b.display_text();
        assert_eq!(shown, "原文");
        assert!(matches!(shown, std::borrow::Cow::Borrowed(_)));
        assert_eq!(
            shown.as_ptr(),
            b.text().as_ptr(),
            "无组合输入时应借用正文，不能每帧复制全文"
        );
        assert_eq!(range, None);
        assert_eq!(b.display_cursor(), 0);
    }

    #[test]
    fn an_empty_composition_clears_it() {
        // 输入法在退格删光拼音时会发一个空串
        let mut b = buf("");
        b.set_composition("zh", 2);
        b.set_composition("", 0);
        assert!(b.composition().is_none());
    }

    #[test]
    fn utf16_offsets_are_what_imm32_and_directwrite_expect() {
        let b = buf("a中𝄞");
        assert_eq!(b.utf16_offset(0), 0);
        assert_eq!(b.utf16_offset(1), 1); // 'a'
        assert_eq!(b.utf16_offset(4), 2); // + '中'
                                          // 星文平面字符占两个 UTF-16 码元
        assert_eq!(b.utf16_offset(b.text().len()), 4);
    }

    // ---------- 视觉行映射 ----------

    #[test]
    fn hard_breaks_produce_separate_lines() {
        let l = layout("一\n二\n三", S, 1000.0);
        assert_eq!(l.lines.len(), 3);
        assert!(l.lines.iter().all(|x| x.hard_break));
        assert_eq!(l.lines[0].start, 0);
        assert_eq!(l.lines[1].start, "一\n".len());
        assert_eq!(l.lines[2].start, "一\n二\n".len());
    }

    #[test]
    fn a_long_line_soft_wraps_and_only_the_last_piece_is_a_hard_break() {
        let l = layout("一二三四五六七八九十", S, 65.0);
        assert!(l.lines.len() > 1);
        assert!(!l.lines[0].hard_break, "软换行不是硬换行");
        assert!(l.lines.last().unwrap().hard_break);
    }

    #[test]
    fn line_ranges_tile_the_text_without_gaps_or_overlap() {
        // 范围错一个字节，光标定位就会整体偏移
        let src = "第一行\n这是一段很长的中文需要软换行的内容\n末行";
        let l = layout(src, S, 78.0);
        for pair in l.lines.windows(2) {
            let gap = pair[1].start - pair[0].end;
            assert!(
                gap <= 1,
                "行之间空了 {gap} 字节：{:?} → {:?}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(l.lines.last().unwrap().end, src.len());
    }

    #[test]
    fn an_empty_line_still_gets_a_visual_line() {
        // 连续两个换行中间那个空行必须占位，否则光标下不去
        let l = layout("上\n\n下", S, 500.0);
        assert_eq!(l.lines.len(), 3);
        assert_eq!(l.lines[1].start, l.lines[1].end);
    }

    #[test]
    fn locating_the_end_of_a_soft_wrapped_line_stays_on_that_line() {
        // 归到下一行行首的话，在软换行处按 End 光标会跳走，看着像没反应
        let l = layout("一二三四五六七八九十", S, 65.0);
        let first_end = l.lines[0].end;
        let (line, _) = l.locate(first_end, S);
        assert_eq!(line, 0);
    }

    #[test]
    fn locate_and_offset_at_are_inverses() {
        let src = "abc 中文 def\n第二行";
        let l = layout(src, S, 1000.0);
        for offset in [0usize, 1, 4, 7, 11, src.find('第').unwrap()] {
            let (line, x) = l.locate(offset, S);
            assert_eq!(l.offset_at(line, x, S), offset, "偏移 {offset} 往返对不上");
        }
    }

    #[test]
    fn clicking_past_the_middle_of_a_character_lands_after_it() {
        // 按左边界判定的话，点字的右半边光标会跑到左边去
        let l = layout("中文", S, 500.0);
        let w = S.advance_em('中') * S.font_size();
        assert_eq!(l.offset_at(0, w * 0.2, S), 0);
        assert_eq!(l.offset_at(0, w * 0.8, S), "中".len());
    }

    #[test]
    fn clicking_past_the_end_of_a_line_lands_at_its_end() {
        let l = layout("短", S, 500.0);
        assert_eq!(l.offset_at(0, 9999.0, S), "短".len());
    }

    #[test]
    fn line_at_maps_y_to_a_line_and_clamps() {
        let l = layout("一\n二\n三", S, 500.0);
        assert_eq!(l.line_at(0.0, S), 0);
        assert_eq!(l.line_at(S.line_height() * 1.5, S), 1);
        assert_eq!(l.line_at(9999.0, S), 2);
        assert_eq!(l.line_at(-5.0, S), 0);
    }

    #[test]
    fn vertical_movement_keeps_the_desired_x_across_a_short_line() {
        // 不记住目标 x 的话，从长行移到短行再移回来，光标会永久停在短行的行尾
        let src = "很长的一行内容在这里\n短\n另一个很长的行在这里";
        let l = layout(src, S, 1000.0);
        let start = src.find('容').unwrap();
        let (_, x) = l.locate(start, S);

        let down1 = l.move_vertical(start, 1, x, S); // → 短行，落在行尾
        let down2 = l.move_vertical(down1, 1, x, S); // → 第三行，应回到原 x
        let (line, x2) = l.locate(down2, S);
        assert_eq!(line, 2);
        assert!(
            (x2 - x).abs() < S.font_size(),
            "目标 x 没保持住：{x2} vs {x}"
        );
    }

    #[test]
    fn vertical_movement_is_clamped_at_both_ends() {
        let l = layout("一\n二", S, 500.0);
        assert_eq!(l.move_vertical(0, -1, 0.0, S), 0);
        let last = l.lines.last().unwrap().start;
        assert_eq!(l.locate(l.move_vertical(last, 1, 0.0, S), S).0, 1);
    }

    #[test]
    fn line_bounds_give_home_and_end() {
        let l = layout("第一行\n第二行", S, 500.0);
        assert_eq!(l.line_bounds(1), ("第一行\n".len(), "第一行\n第二行".len()));
        assert_eq!(l.line_bounds(99), (0, 0));
    }

    #[test]
    fn utf16_units_convert_back_to_byte_offsets() {
        // 中文一个字是 1 个码元、3 个字节；直接当字节用会把光标插进字的中间
        let s = "a中文";
        assert_eq!(byte_offset_from_utf16(s, 0), 0);
        assert_eq!(byte_offset_from_utf16(s, 1), 1);
        assert_eq!(byte_offset_from_utf16(s, 2), 4);
        assert_eq!(byte_offset_from_utf16(s, 3), 7);
        // 超出范围夹到末尾，不能返回越界的偏移
        assert_eq!(byte_offset_from_utf16(s, 99), s.len());
    }

    #[test]
    fn utf16_conversion_round_trips_through_the_buffer() {
        let b = buf("a中𝄞文");
        for byte in [0usize, 1, 4, 8, b.text().len()] {
            let units = b.utf16_offset(byte);
            assert_eq!(
                byte_offset_from_utf16(b.text(), units),
                byte,
                "{byte} 往返对不上"
            );
        }
    }

    #[test]
    fn a_composition_is_painted_with_a_highlight_and_an_underline() {
        // 组合串必须一眼看得出还没提交，否则用户会以为已经输入完了
        use crate::ui::draw::{DrawCmd, DrawList};
        use crate::ui::layout::Rect;
        use crate::ui::theme;

        let mut b = buf("前后");
        b.set_cursor("前".len(), false);
        b.set_composition("zw", 2);
        let (shown, _) = b.display_text();
        let lay = layout(&shown, S, 500.0);

        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        let area = Rect::new(0.0, 0.0, 600.0, 400.0);
        paint(&mut list, area, &b, &lay, S, 0.0, true, &p);
        assert!(list.finish().is_ok());

        let tint = theme::mix(p.accent, p.area_main_default, 0.14);
        let underline: Vec<&Rect> = list
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Rect { rect, color } if *color == p.accent => Some(rect),
                _ => None,
            })
            .collect();
        assert!(
            list.cmds()
                .iter()
                .any(|c| matches!(c, DrawCmd::Rect { color, .. } if *color == tint)),
            "组合串没有底色"
        );
        // 强调色的矩形有两个：下划线和光标。下划线是扁的
        assert!(
            underline.iter().any(|r| r.height() <= 2.0),
            "组合串没有下划线"
        );
        // 组合串本身画出来了
        assert!(list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Text { text, .. } if text.contains("zw"))));
    }

    #[test]
    fn the_caret_is_not_drawn_without_focus() {
        use crate::ui::draw::{DrawCmd, DrawList};
        use crate::ui::layout::Rect;
        use crate::ui::theme;

        let b = buf("内容");
        let lay = layout(b.text(), S, 500.0);
        let p = *theme::tokens().palette(false);
        let area = Rect::new(0.0, 0.0, 600.0, 400.0);

        let mut focused = DrawList::new();
        paint(&mut focused, area, &b, &lay, S, 0.0, true, &p);
        let mut blurred = DrawList::new();
        paint(&mut blurred, area, &b, &lay, S, 0.0, false, &p);

        let carets = |l: &DrawList| {
            l.cmds()
                .iter()
                .filter(|c| {
                    matches!(c, DrawCmd::Caret { color, rect, .. }
                    if *color == p.foreground && rect.width() <= CARET_WIDTH)
                })
                .count()
        };
        assert_eq!(carets(&focused), 1);
        assert_eq!(carets(&blurred), 0);
    }

    #[test]
    fn a_selection_is_highlighted_across_the_lines_it_covers() {
        use crate::ui::draw::{DrawCmd, DrawList};
        use crate::ui::layout::Rect;
        use crate::ui::theme;

        let mut b = buf("第一行
第二行
第三行");
        b.set_cursor(0, false);
        b.set_cursor(b.text().len(), true);
        let lay = layout(b.text(), S, 500.0);
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(
            &mut list,
            Rect::new(0.0, 0.0, 600.0, 400.0),
            &b,
            &lay,
            S,
            0.0,
            true,
            &p,
        );

        let tint = theme::mix(p.accent, p.area_main_default, 0.22);
        let bands = list
            .cmds()
            .iter()
            .filter(|c| matches!(c, DrawCmd::Rect { color, .. } if *color == tint))
            .count();
        assert_eq!(bands, 3, "三行都该有选区底色");
    }

    #[test]
    fn an_empty_buffer_lays_out_to_one_empty_line() {
        let l = layout("", S, 500.0);
        assert_eq!(l.lines.len(), 1);
        assert_eq!(l.lines[0].start, 0);
        assert_eq!(l.lines[0].end, 0);
        assert_eq!(l.locate(0, S), (0, 0.0));
    }
}

#[cfg(test)]
mod block_identity_tests {
    use super::*;
    #[test]
    fn persisted_identity_is_part_of_the_user_edit_undo_transaction() {
        let path = std::path::Path::new("note.md");
        let initial = mochi_core::document_blocks::prepare_conversion(path, "第一块\n")
            .unwrap()
            .converted_source;
        let mut buffer = TextBuffer::new(&initial);
        buffer.set_cursor(initial.len(), false);
        buffer.insert("\n新的块😀");
        let converted = mochi_core::document_blocks::prepare_conversion(path, buffer.text())
            .unwrap()
            .converted_source;
        buffer.replace_persisted_source(&converted, true);
        assert_eq!(buffer.cursor(), converted.len());
        assert!(buffer.undo());
        assert_eq!(buffer.text(), initial);
        assert!(buffer.redo());
        assert_eq!(buffer.text(), converted);
    }
}
