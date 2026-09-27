//! 在富文本块中插入、替换和删除内容，并处理代码块文本。
use super::*;

/// 替换可见选区，且不留下不配对的格式包裹。
pub fn insert(buffer: &mut TextBuffer, value: &str) {
    if buffer.has_selection() {
        replace(buffer, buffer.selection(), value);
        return;
    }
    let parsed = crate::ui::document::parse_ranged(buffer.text());
    insert_parsed(buffer, value, &parsed);
}

/// 活动编辑器已持有当前缓冲的解析结果；每敲一个可打印字符
/// 都把整篇笔记重新解析一遍就太浪费了。
pub fn insert_parsed(buffer: &mut TextBuffer, value: &str, parsed: &Parsed) {
    if !buffer.has_selection() {
        // Ctrl+Home 落在字节零，但转换过的文档可能以隐藏的独立标记开头
        //（文档头部也可能位于更前面）。新输入必须写入可见正文，
        // 才不会插到标记之前、把那条身份注释变成另一个块的前缀。
        // 这里复用既有的标记扫描；可打印输入无需重解析编辑器文档。
        if buffer.cursor() < parsed.body_start {
            buffer.set_cursor(parsed.body_start, false);
        }
        if let Some(hidden) =
            crate::ui::block_markers::hidden_range_at(buffer.text(), buffer.cursor())
        {
            buffer.set_cursor(hidden.end, false);
        }
        if let Some(Block::Container(panel)) = parsed
            .block_at(buffer.cursor())
            .map(|i| &parsed.blocks[i].block)
        {
            // 键盘导航可能落在卡片头部。插入到它的正文里，
            // 并在同一撤销事务中把闭合的卡片展开。
            let newline = if buffer.text().contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let mut draft = buffer.preview();
            let mut panel = panel.clone();
            panel.open = true;
            crate::ui::containers::update(&mut draft, &panel);
            let panel = crate::ui::containers::scan(draft.text())
                .into_iter()
                .find(|p| p.header.start == panel.header.start)
                .unwrap();
            draft.set_cursor(panel.body.start, false);
            draft.insert(&format!(
                "{value}{}",
                if panel.body.is_empty() { newline } else { "" }
            ));
            let caret = panel.body.start + value.len();
            buffer.replace_range_select(0..buffer.text().len(), draft.text(), caret..caret);
            return;
        }
        buffer.insert(value);
        return;
    }
    replace(buffer, buffer.selection(), value);
}

pub fn compose(buffer: &mut TextBuffer, value: &str, cursor: usize) {
    buffer.set_composition(value, cursor);
    if !value.is_empty() {
        let mut preview = buffer.preview();
        insert(&mut preview, value);
        let start = preview.cursor() - value.len();
        buffer.composition_preview(preview.text().to_owned(), start);
    }
}

fn replace(buffer: &mut TextBuffer, (mut a, mut b): (usize, usize), value: &str) {
    let source = buffer.text();
    let parsed = crate::ui::document::parse_ranged(source);
    a = a.max(parsed.body_start);
    let mut wrappers = Vec::new();
    for rb in &parsed.blocks {
        if (rb.end < a || rb.start > b || matches!(rb.block, Block::Code { .. } | Block::Math(_)))
            && !matches!(rb.block, Block::Container(_))
        {
            continue;
        }
        if let Block::Container(panel) = &rb.block {
            wrappers.push(Wrapper {
                full: panel.header.start..panel.footer.end,
                body: panel.body.clone(),
            });
            continue;
        }
        let map = Mapping::block(rb, source);
        wrappers.extend(map.wrappers.iter());
    }
    loop {
        let old = (a, b);
        for w in &wrappers {
            if a <= w.body.start && b >= w.body.end && a < w.full.end && b > w.full.start {
                a = a.min(w.full.start);
                b = b.max(w.full.end);
            }
        }
        if old == (a, b) {
            break;
        }
    }
    let mut keep = Vec::new();
    for w in &wrappers {
        if a <= w.full.start && b >= w.full.end {
            continue;
        }
        for r in [w.full.start..w.body.start, w.body.end..w.full.end] {
            let start = r.start.max(a);
            let end = r.end.min(b);
            if start < end {
                keep.push(start..end);
            }
        }
    }
    keep.sort_by_key(|r| r.start);
    let mut replacement = value.to_owned();
    let mut end = a;
    for r in keep {
        let start = r.start.max(end);
        if start < r.end {
            let structural = parsed.blocks.iter().any(|rb| {
                matches!(&rb.block, Block::Container(p)
                if p.header.start == start || p.footer.start == start)
            });
            if structural
                && !replacement.ends_with('\n')
                && (!replacement.is_empty() || (a > 0 && source.as_bytes()[a - 1] != b'\n'))
            {
                replacement.push_str(if source.contains("\r\n") {
                    "\r\n"
                } else {
                    "\n"
                });
            }
            replacement.push_str(&source[start..r.end]);
            end = r.end;
        }
    }
    if value.is_empty()
        && parsed
            .blocks
            .iter()
            .any(|rb| a <= rb.start && b >= rb.end && rb.start < rb.end)
    {
        let range = crate::ui::block_markers::deletion_range(source, a..b);
        a = range.start;
        b = range.end;
    }
    buffer.replace_range_select(a..b, &replacement, value.len()..value.len());
}

pub fn delete(buffer: &mut TextBuffer, backward: bool, parsed: &Parsed) {
    if buffer.has_selection() {
        insert(buffer, "");
        return;
    }
    let cursor = buffer.cursor();
    let Some(bi) = parsed.block_at(cursor) else {
        return;
    };
    let rb = &parsed.blocks[bi];
    if let Block::Container(panel) = &rb.block {
        crate::ui::containers::unwrap(buffer, panel);
        return;
    }
    if matches!(
        rb.block,
        Block::Math(_)
            | Block::Image { .. }
            | Block::Divider
            | Block::AiLocator(_)
            | Block::ObjectReference(_)
    ) {
        let range = crate::ui::block_markers::deletion_range(buffer.text(), rb.start..rb.end);
        buffer.replace_range(range, "");
        return;
    }
    if matches!(rb.block, Block::Code { .. }) {
        let body = code_body(rb, buffer.text());
        if (backward && cursor > body.start) || (!backward && cursor < body.end) {
            if backward {
                buffer.delete_backward();
            } else {
                buffer.delete_forward();
            }
        }
        return;
    }
    let map = Mapping::block(rb, buffer.text());
    let unit = if backward {
        map.units.iter().rev().find(|u| u.source.start < cursor)
    } else {
        map.units.iter().find(|u| u.source.end > cursor)
    };
    if let Some(unit) = unit {
        // 带标记块的最后一个可见单元被删掉后，把整行连同标记一起消费掉。
        // 只留标记的话，下次转换/保存时下一个块会继承这个旧 ID。
        // 这个助手只动语法；上面的 `parsed` 是编辑器现成的解析结果，
        // 普通删除字符依旧不做全文档解析。
        if map.units.len() == 1 {
            let range = crate::ui::block_markers::deletion_range_for_block(
                buffer.text(),
                rb.start..rb.end,
                unit.source.clone(),
            );
            if range != unit.source {
                buffer.replace_range(range, "");
                return;
            }
        }
        replace(buffer, (unit.source.start, unit.source.end), "");
        return;
    }
    // 光标在列表/标题/引用的开头时，先把它提升为段落。
    let body = body_range(rb, buffer.text());
    if backward
        && body.start > rb.start
        && matches!(
            rb.block,
            Block::Heading { .. } | Block::ListItem { .. } | Block::Quote(_)
        )
    {
        buffer.replace_range(rb.start..body.start, "");
        return;
    }
    // 空行也是可编辑的行：只合并相邻块，一次 Backspace/Delete 删掉
    // 一个换行，空文档里同样适用。
    let neighbour = if backward {
        parsed.blocks[..bi].last()
    } else {
        parsed.blocks.get(bi + 1)
    };
    if let Some(other) = neighbour {
        if let Block::Container(panel) = &other.block {
            if backward {
                crate::ui::containers::unwrap(buffer, panel);
            }
            return;
        }
        if matches!(
            other.block,
            Block::Code { .. }
                | Block::Math(_)
                | Block::Table { .. }
                | Block::Image { .. }
                | Block::Divider
                | Block::AiLocator(_)
                | Block::ObjectReference(_)
        ) {
            return;
        }
        let (a, b) = if backward {
            (other.end, body.start)
        } else {
            (rb.end, body_range(other, buffer.text()).start)
        };
        if parsed.blocks.iter().any(|rb| {
            matches!(&rb.block, Block::Container(p)
            if a < p.footer.end && b > p.footer.start || a < p.header.end && b > p.header.start)
        }) {
            return;
        }
        buffer.replace_range(a..b, "");
    }
}

pub fn code_body(rb: &RangedBlock, source: &str) -> Range<usize> {
    let raw = &source[rb.start..rb.end];
    let start = rb.start + raw.find('\n').map(|i| i + 1).unwrap_or(0);
    let close = raw.rfind('\n').filter(|i| {
        let last = raw[i + 1..].trim();
        crate::ui::document::fence_open(raw.lines().next().unwrap_or("").trim()).is_some_and(
            |(marker, n, _)| {
                last.chars().take_while(|c| *c == marker).count() >= n
                    && last.chars().all(|c| c == marker)
            },
        )
    });
    let mut end = close.map(|i| rb.start + i).unwrap_or(rb.end).max(start);
    if end > start && source.as_bytes().get(end - 1) == Some(&b'\r') {
        end -= 1;
    }
    start..end
}

pub fn enter(buffer: &mut TextBuffer, shift: bool) {
    let parsed = crate::ui::document::parse_ranged(buffer.text());
    let Some(rb) = parsed.block_at(buffer.cursor()).map(|i| &parsed.blocks[i]) else {
        insert(buffer, "\n");
        return;
    };
    let newline = if buffer.text().contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    if let Block::Container(panel) = &rb.block {
        let at = if panel.open {
            panel.body.start
        } else {
            panel.footer.end
        };
        buffer.set_cursor(at, false);
        buffer.insert(newline);
        return;
    }
    if matches!(rb.block, Block::Code { .. } | Block::Math(_)) {
        buffer.insert(newline);
        return;
    }
    let body = body_range(rb, buffer.text());
    let prefix = buffer.text()[rb.start..body.start].to_owned();
    let map = Mapping::block(rb, buffer.text());
    if map.units.is_empty() && matches!(rb.block, Block::ListItem { .. } | Block::Quote(_)) {
        buffer.replace_range(rb.start..rb.end, "");
        return;
    }
    let continuation = match &rb.block {
        Block::ListItem { marker, .. } if !shift => {
            if marker == "☑" || marker == "☐" {
                prefix.replace("[x]", "[ ]").replace("[X]", "[ ]")
            } else if let Some(n) = marker.strip_suffix('.').and_then(|n| n.parse::<u64>().ok()) {
                let indent = prefix.len() - prefix.trim_start().len();
                format!("{}{}. ", &prefix[..indent], n.saturating_add(1))
            } else {
                prefix
            }
        }
        Block::Quote(_) if !shift => prefix,
        _ => String::new(),
    };
    // 在带标记块的可见起点拆分时，原内容必须连同旧标记一起保留。
    // 只在 `body.start` 插入换行会留下 `marker\n\nnew text`，
    // 下次解析可能把这个 ID 挂到新出现的空块/续块上。于是把标记行
    // 与新行整体交换位置，做成一次可撤销的编辑。
    // 这个分支刻意只覆盖已知的可见起点；普通打字和块内回车
    // 仍走快速缓存解析路径。
    if !buffer.has_selection() && buffer.cursor() == map.start {
        if let Some(marker_line) =
            crate::ui::block_markers::standalone_marker_before(buffer.text(), rb.start)
        {
            let source = buffer.text();
            let prefix = source[marker_line.start..body.start].to_owned();
            let mut moved = continuation.clone();
            moved.push_str(newline);
            moved.push_str(&prefix);
            let caret = if continuation.is_empty() {
                newline.len()
            } else {
                continuation.len()
            };
            buffer.replace_range_select(marker_line.start..body.start, &moved, caret..caret);
            return;
        }
    }
    // 在段落边界拆分标记，两半才都仍然合法。
    let mut at = buffer.cursor();
    if !buffer.has_selection() {
        for w in &map.wrappers {
            if at == w.body.end {
                at = w.full.end;
            } else if at == w.body.start {
                at = w.full.start;
            }
        }
        buffer.set_cursor(at, false);
    }
    let mut active = map
        .wrappers
        .iter()
        .filter(|w| w.body.start < at && at < w.body.end)
        .collect::<Vec<_>>();
    active.sort_by_key(|w| w.full.start);
    let mut value = String::new();
    for w in active.iter().rev() {
        value.push_str(&buffer.text()[w.body.end..w.full.end]);
    }
    value.push_str(newline);
    value.push_str(&continuation);
    for w in active {
        value.push_str(&buffer.text()[w.full.start..w.body.start]);
    }
    // 光标在带标记 run 的末尾时，插到它的闭合语法之后。
    if !buffer.has_selection() && at >= map.end {
        buffer.set_cursor(rb.end, false);
    }
    insert(buffer, &value);
}

pub fn indent(buffer: &mut TextBuffer, outdent: bool) {
    let parsed = crate::ui::document::parse_ranged(buffer.text());
    let Some(rb) = parsed.block_at(buffer.cursor()).map(|i| &parsed.blocks[i]) else {
        return;
    };
    if matches!(rb.block, Block::ListItem { .. }) {
        let cursor = buffer.cursor();
        if outdent {
            let count = buffer.text()[rb.start..rb.end]
                .chars()
                .take_while(|c| *c == ' ')
                .count()
                .min(2);
            buffer.replace_range(rb.start..rb.start + count, "");
            buffer.set_cursor(cursor - count, false);
        } else {
            buffer.replace_range(rb.start..rb.start, "  ");
            buffer.set_cursor(cursor + 2, false);
        }
    } else {
        insert(buffer, "  ");
    }
}

pub fn apply_format(buffer: &mut TextBuffer, format: crate::ui::format::Format) {
    use crate::ui::format::Format;
    let (open, close) = match format {
        Format::Bold => ("<strong>", "</strong>"),
        Format::Italic => ("<em>", "</em>"),
        Format::Underline => ("<u>", "</u>"),
        Format::Strike => ("<s>", "</s>"),
        Format::Code => ("<code>", "</code>"),
        _ => {
            crate::ui::format::apply(buffer, format);
            return;
        }
    };
    let (a, b) = buffer.selection();
    let parsed = crate::ui::document::parse_ranged(buffer.text());
    let map = parsed
        .block_at(a)
        .map(|i| Mapping::block(&parsed.blocks[i], buffer.text()));
    let wrapper = map.as_ref().and_then(|m| {
        m.wrappers
            .iter()
            .filter(|w| w.body.start <= a && b <= w.body.end)
            .filter(|w| {
                let prefix = &buffer.text()[w.full.start..w.body.start];
                prefix == open
                    || match format {
                        Format::Bold => matches!(prefix, "**" | "__" | "<b>"),
                        Format::Italic => matches!(prefix, "*" | "_" | "<i>"),
                        Format::Strike => matches!(prefix, "~~" | "<del>"),
                        Format::Code => prefix == "`",
                        _ => false,
                    }
            })
            .min_by_key(|w| w.full.len())
    });
    if let Some(w) = wrapper {
        let source = buffer.text();
        let open = &source[w.full.start..w.body.start];
        let close = &source[w.body.end..w.full.end];
        let mut value = String::new();
        if a > w.body.start {
            value.push_str(open);
            value.push_str(&source[w.body.start..a]);
            value.push_str(close);
        }
        let start = value.len();
        value.push_str(&source[a..b]);
        let end = value.len();
        if b < w.body.end {
            value.push_str(open);
            value.push_str(&source[b..w.body.end]);
            value.push_str(close);
        }
        buffer.replace_range_select(w.full, &value, start..end);
        return;
    }
    if buffer.has_selection() {
        // 富文本模式把标记序列化成 HTML。Markdown 的单个 `*` 斜体分隔符
        // 在空白/标点边界有歧义，重新解析可能对应到另一段源码区间，
        // Ctrl+I 按下时选中的字形就会消失。空光标路径用的是无歧义标签，
        // 这里也用它，让可见选区保持稳定。
        if format == Format::Italic {
            let source = buffer.selected_text().to_owned();
            let wrapped = format!("{open}{source}{close}");
            buffer.replace_range_select(a..b, &wrapped, open.len()..open.len() + source.len());
        } else {
            crate::ui::format::apply(buffer, format);
        }
        return;
    }
    let at = buffer.cursor();
    if buffer.text()[..at].ends_with(open) && buffer.text()[at..].starts_with(close) {
        buffer.replace_range(at - open.len()..at + close.len(), "");
        return;
    }
    // HTML 本来就是 Mochi Markdown 格式的一部分。空标记仍是合法的
    // 插入位置，不像 ****（那是一条 Markdown 水平线）。
    buffer.replace_range_select(at..at, &format!("{open}{close}"), open.len()..open.len());
}
