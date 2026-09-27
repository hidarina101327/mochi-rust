//! 整理富文本选区对应的 Markdown 文本和块范围。
use super::*;

/// 可见选区从标记内部开始或结束时，复制助手负责补回包裹。
/// 复制绝不改动源码缓冲和选区本身。
///
/// 选区是相对源码的，而文档视图隐藏了少量语法。这里把两件事分开处理：
/// 先去掉隐藏的容器/代码元数据、重建块级片段，再给选中的可见文字
/// 补上行内包裹。
pub fn selected_markdown(buffer: &TextBuffer) -> String {
    let (start, end) = buffer.selection();
    if start == end {
        return String::new();
    }
    let source = buffer.text();
    let parsed = crate::ui::document::parse_ranged(source);
    let start = start.max(parsed.body_start);
    if start >= end {
        return String::new();
    }
    let omitted = selected_omitted_ranges(source, &parsed);
    let (opens, closes) = selected_wrapper_edges(source, &parsed, start, end, &omitted);
    let mut result = String::new();
    let mut cursor = start;
    let mut opening_at = None;

    for (block_index, block) in parsed.blocks.iter().enumerate() {
        let span = selected_block_span(block);
        if span.end <= start || span.start >= end {
            continue;
        }
        let segment_start = start.max(span.start);
        let segment_end = end.min(span.end);
        if segment_end <= cursor {
            continue;
        }
        let segment_start = segment_start.max(cursor);
        let paragraph_break = (block_index > 0)
            .then(|| {
                let previous = &parsed.blocks[block_index - 1];
                (matches!(previous.block, Block::Paragraph(_))
                    && matches!(block.block, Block::Paragraph(_)))
                .then(|| source.get(cursor..segment_start))
                .flatten()
                .filter(|gap| matches!(*gap, "\n" | "\r\n"))
            })
            .flatten();
        if cursor < segment_start {
            append_selected_source(&mut result, source, cursor..segment_start, &omitted);
            // `parse_ranged` 把每个物理段落行当作独立的块。所以单个
            // 源码换行在这里就是语义上的段落边界，而 CommonMark 会把
            // 它解析成软换行。把行尾重复一遍，HTML 往返才不会把
            // 两个相邻的编辑器段落合并成一个。
            if let Some(newline) = paragraph_break {
                result.push_str(newline);
            }
        }
        if segment_start >= segment_end {
            cursor = cursor.max(segment_end);
            continue;
        }
        match &block.block {
            Block::Code { .. } => {
                result.push_str(&selected_code_block(
                    block,
                    source,
                    segment_start..segment_end,
                ));
            }
            Block::Container(panel) => {
                let header_selected =
                    segment_start < panel.header.end && segment_end > panel.header.start;
                let body_selected = start < panel.body.end && end > panel.body.start;
                if header_selected {
                    if !result.is_empty() {
                        let newline = preferred_newline(source);
                        if !result.ends_with(newline) {
                            result.push_str(newline);
                        }
                        if !result.ends_with(&format!("{newline}{newline}")) {
                            result.push_str(newline);
                        }
                    }
                    result.push_str(&crate::ui::containers::escape(&panel.title));
                    if body_selected {
                        // 头部拥有自己的源码行尾。移除这个隐藏外壳后，
                        // 要重建一个段落边界。
                        let newline = preferred_newline(source);
                        result.push_str(newline);
                        result.push_str(newline);
                    }
                }
                append_selected_source(&mut result, source, segment_start..segment_end, &omitted);
            }
            Block::Heading { .. } | Block::ListItem { .. } | Block::Quote(_) => {
                let before = result.len();
                let prefix = selected_block_prefix(block, source, segment_start);
                result.push_str(&prefix);
                result.push_str(&source[segment_start..segment_end]);
                if opening_at.is_none() && before == 0 {
                    opening_at = Some(before + prefix.len());
                }
            }
            _ => append_selected_source(&mut result, source, segment_start..segment_end, &omitted),
        }
        cursor = segment_end;
    }
    if cursor < end {
        append_selected_source(&mut result, source, cursor..end, &omitted);
    }

    // 列表/标题/引用的前缀不属于行内标记。其余首块的插入点
    // 就是复制的起点。
    result.insert_str(opening_at.unwrap_or(0), &opens);
    result.push_str(&closes);
    result
}

fn selected_block_span(block: &RangedBlock) -> Range<usize> {
    block.start..block.end
}

fn selected_block_prefix(block: &RangedBlock, source: &str, at: usize) -> String {
    if !matches!(
        block.block,
        Block::Heading { .. } | Block::ListItem { .. } | Block::Quote(_)
    ) || at <= block.start
    {
        return String::new();
    }
    let body = body_range(block, source);
    if at < body.start {
        String::new()
    } else {
        source[block.start..body.start].to_owned()
    }
}

fn selected_wrapper_edges(
    source: &str,
    parsed: &Parsed,
    start: usize,
    end: usize,
    omitted: &[Range<usize>],
) -> (String, String) {
    let mut opens = Vec::new();
    let mut closes = Vec::new();
    for block in &parsed.blocks {
        let span = selected_block_span(block);
        if span.start >= end || span.end <= start {
            continue;
        }
        let wrappers = selected_block_wrappers(block, source, omitted);
        for wrapper in &wrappers {
            if start >= wrapper.body.end || end <= wrapper.body.start {
                continue;
            }
            if start >= wrapper.body.start && start > wrapper.full.start {
                opens.push((
                    wrapper.full.start,
                    &source[wrapper.full.start..wrapper.body.start],
                ));
            }
            if end <= wrapper.body.end && end < wrapper.full.end {
                closes.push((
                    wrapper.full.end,
                    &source[wrapper.body.end..wrapper.full.end],
                ));
            }
        }
    }
    opens.sort_by_key(|(at, _)| *at);
    closes.sort_by_key(|(at, _)| *at);
    opens.dedup_by(|left, right| left.0 == right.0 && left.1 == right.1);
    closes.dedup_by(|left, right| left.0 == right.0 && left.1 == right.1);
    let opens = opens
        .into_iter()
        .fold(String::new(), |mut result, (_, value)| {
            result.push_str(value);
            result
        });
    let closes = closes
        .into_iter()
        .fold(String::new(), |mut result, (_, value)| {
            result.push_str(value);
            result
        });
    (opens, closes)
}

fn selected_block_wrappers(
    block: &RangedBlock,
    source: &str,
    omitted: &[Range<usize>],
) -> Vec<Wrapper> {
    let mut wrappers = match &block.block {
        // 容器的子元素由 `containers::project` 保留，并在下面独立映射。
        // 在这里映射整个正文会把嵌套的分隔符或围栏代码误判为行内包裹。
        Block::Container(_) => Vec::new(),
        _ => Mapping::block(block, source).wrappers.iter().collect(),
    };
    wrappers.retain(|wrapper| {
        !omitted
            .iter()
            .any(|hidden| wrapper.full.start < hidden.end && wrapper.full.end > hidden.start)
    });
    wrappers.sort_by_key(|wrapper| {
        (
            wrapper.full.start,
            wrapper.full.end,
            wrapper.body.start,
            wrapper.body.end,
        )
    });
    wrappers.dedup();
    wrappers
}

fn selected_omitted_ranges(source: &str, parsed: &Parsed) -> Vec<Range<usize>> {
    let mut omitted = crate::ui::block_markers::hidden_ranges(source);
    for block in &parsed.blocks {
        match &block.block {
            Block::Container(panel) => {
                omitted.push(panel.header.clone());
                omitted.push(panel.footer.clone());
                // 闭合分隔符之后的换行属于容器外壳。只有选区越过
                // 尾部时才丢弃它；选区恰好结束在尾部时，
                // 自己最后的正文换行保持原样。
                if let Some(length) = line_break_len(source, panel.footer.end) {
                    omitted.push(panel.footer.end..panel.footer.end + length);
                }
            }
            Block::Code { .. } => {
                let metadata = crate::ui::code_blocks::metadata(source, block.start).0;
                if !metadata.is_empty() {
                    omitted.push(metadata);
                }
            }
            _ => {}
        }
    }
    omitted.sort_by_key(|range| (range.start, range.end));
    omitted
}

fn line_break_len(source: &str, at: usize) -> Option<usize> {
    let tail = source.get(at..)?;
    if tail.starts_with("\r\n") {
        Some(2)
    } else if tail.starts_with('\n') || tail.starts_with('\r') {
        Some(1)
    } else {
        None
    }
}

fn preferred_newline(source: &str) -> &'static str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn append_selected_source(
    output: &mut String,
    source: &str,
    range: Range<usize>,
    omitted: &[Range<usize>],
) {
    let mut at = range.start;
    for hidden in omitted {
        if hidden.end <= at {
            continue;
        }
        if hidden.start >= range.end {
            break;
        }
        let hidden_start = hidden.start.max(range.start);
        if at < hidden_start {
            output.push_str(&source[at..hidden_start.min(range.end)]);
        }
        at = at.max(hidden.end.min(range.end));
        if at >= range.end {
            break;
        }
    }
    if at < range.end {
        output.push_str(&source[at..range.end]);
    }
}

fn selected_code_block(block: &RangedBlock, source: &str, selection: Range<usize>) -> String {
    let body = code_body(block, source);
    let start = selection.start.max(body.start).min(body.end);
    let end = selection.end.min(body.end).max(start);
    let body = &source[start..end];
    let (marker, original_count) = source
        .get(block.start..)
        .and_then(|raw| raw.split('\n').next())
        .map(|line| line.trim_end_matches('\r').trim())
        .and_then(crate::ui::document::fence_open)
        .map(|(marker, count, _)| (marker, count))
        .unwrap_or(('`', 3));
    let language = match &block.block {
        Block::Code { lang, .. } => lang.as_str(),
        _ => "",
    };
    let longest = body
        .chars()
        .fold((0usize, 0usize), |(longest, run), ch| {
            if ch == marker {
                (longest.max(run + 1), run + 1)
            } else {
                (longest, 0)
            }
        })
        .0;
    let count = original_count.max(longest.saturating_add(1)).max(3);
    let fence = marker.to_string().repeat(count);
    let newline = preferred_newline(source);
    let mut output = format!("{fence}{language}{newline}{body}");
    if !output.ends_with('\n') {
        if output.ends_with('\r') {
            output.push('\n');
        } else {
            output.push_str(newline);
        }
    }
    output.push_str(&fence);
    output
}
