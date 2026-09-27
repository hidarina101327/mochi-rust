//! 隐藏块 ID 时保留源码偏移；按键路径复用解析结果，不重新解析整篇文档。
use std::ops::Range;

use mochi_blocks::model::{self, BlockId, Marker};

#[derive(Debug, Clone)]
struct ValidMarker {
    comment: Range<usize>,
    line: Range<usize>,
    standalone: bool,
}

pub fn prefix_len(source: &str) -> Option<usize> {
    let tail = source.strip_prefix(model::BLOCK_MARKER_PREFIX)?;
    let end = tail.find(model::BLOCK_MARKER_SUFFIX)?;
    BlockId::parse(&tail[..end]).ok()?;
    Some(model::BLOCK_MARKER_PREFIX.len() + end + model::BLOCK_MARKER_SUFFIX.len())
}

pub fn standalone(line: &str) -> bool {
    let line = line.trim();
    prefix_len(line) == Some(line.len())
}

fn valid_markers(source: &str) -> Vec<ValidMarker> {
    let Ok(markers) = model::scan_markers(source) else {
        // 核心转换路径会报告畸形/未闭合的标记。用户修复源码期间，
        // 编辑器仍要隐藏所有能安全识别的标记，因此当一条畸形注释让
        // 严格扫描器整体失败时，退回有界的尽力扫描。
        return best_effort_markers(source);
    };
    markers
        .into_iter()
        .filter_map(|marker| valid_marker(source, marker))
        .collect()
}

fn valid_marker(source: &str, marker: Marker) -> Option<ValidMarker> {
    BlockId::parse(&marker.token).ok()?;
    let line = source.get(marker.line_span.start..marker.line_span.end)?;
    Some(ValidMarker {
        comment: marker.span.start..marker.span.end,
        line: marker.line_span.start..marker.line_span.end,
        standalone: standalone(line),
    })
}

/// `scan_markers` 刻意从严：发现未闭合标记时不返回任何部分结果。
/// 报错之前的合法注释应当保持隐藏。这个兜底用同一套扫描原语屏蔽
/// 代码块和文档头部之后，只接受完整的规范注释和合法 ID。
fn best_effort_markers(source: &str) -> Vec<ValidMarker> {
    let mut masked_ranges = model::code_ranges(source);
    // `scan_markers` 也会屏蔽文档头部，但那个辅助函数是 blocks 语法
    // 模块的私有实现。文档头部行本身无害，除非其中包含合法标记——
    // 那种情况按可见文本处理会把元数据泄漏进编辑器。这里自行定位
    // 封闭的 YAML 包络。
    if let Some(frontmatter) = closed_frontmatter(source) {
        masked_ranges.push(frontmatter);
    }
    masked_ranges.sort_by_key(|range| (range.start, range.end));
    let mut cursor = 0usize;
    let mut out = Vec::new();
    while let Some(relative) = source[cursor..].find(model::BLOCK_MARKER_PREFIX) {
        let start = cursor + relative;
        if masked_ranges
            .iter()
            .any(|range| range.start <= start && start < range.end)
        {
            cursor = start + model::BLOCK_MARKER_PREFIX.len();
            continue;
        }
        let Some(close) = source[start..].find("-->") else {
            break;
        };
        let end = start + close + 3;
        let raw = &source[start..end];
        let Some(token) = raw
            .strip_prefix(model::BLOCK_MARKER_PREFIX)
            .and_then(|tail| tail.strip_suffix(model::BLOCK_MARKER_SUFFIX))
        else {
            cursor = end;
            continue;
        };
        let Ok(id) = BlockId::parse(token) else {
            cursor = end;
            continue;
        };
        let line_start = source[..start].rfind('\n').map_or(0, |at| at + 1);
        let line_end = source[end..]
            .find('\n')
            .map_or(source.len(), |at| end + at + 1);
        let marker = Marker {
            token: id.into_string(),
            span: model::SourceSpan::new(start, end),
            line_span: model::SourceSpan::new(line_start, line_end),
            raw: raw.to_owned(),
        };
        if let Some(marker) = valid_marker(source, marker) {
            out.push(marker);
        }
        cursor = end;
    }
    out
}

fn closed_frontmatter(source: &str) -> Option<model::SourceSpan> {
    if source
        .lines()
        .next()
        .is_none_or(|line| line.trim_end() != "---")
    {
        return None;
    }
    let mut offset = source.split_inclusive('\n').next().map_or(0, str::len);
    let mut end = None;
    for line in source.split_inclusive('\n').skip(1) {
        offset += line.len();
        if line.trim_end_matches(['\r', '\n']).trim_end() == "---" {
            end = Some(offset);
            break;
        }
    }
    let end = end?;
    Some(model::SourceSpan::new(0, end))
}

pub fn hidden_ranges(source: &str) -> Vec<Range<usize>> {
    let mut ranges = valid_markers(source)
        .into_iter()
        .map(|marker| {
            if marker.standalone {
                marker.line
            } else {
                marker.comment
            }
        })
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| (range.start, range.end));
    ranges
}

/// 找到包含某个源码偏移的隐藏标记区间。
///
/// 普通打字绝大多数落在没有持久化块标记的行上，这种行不要重新扫描
/// 整篇文档；只有包含规范标记前缀的行才需要走感知语法的完整扫描。
/// 后者仍然不可省略：围栏代码和文档头部里外观相同的注释也是可见的，
/// 而畸形源码会走 [`hidden_ranges`] 的尽力兜底。
pub fn hidden_range_at(source: &str, offset: usize) -> Option<Range<usize>> {
    if offset > source.len() {
        return None;
    }
    let bytes = source.as_bytes();
    let mut line_start = offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    let mut line_end = offset;
    while line_end < bytes.len() && bytes[line_end] != b'\n' {
        line_end += 1;
    }
    if line_end < bytes.len() {
        line_end += 1;
    }
    let line = source.get(line_start..line_end)?;
    let relative_offset = offset - line_start;
    if standalone(line) {
        return hidden_ranges(source)
            .into_iter()
            .find(|range| range.start <= offset && offset < range.end);
    }
    let mut cursor = 0usize;
    let mut candidate = false;
    while let Some(relative) = line[cursor..].find(model::BLOCK_MARKER_PREFIX) {
        let marker_start = cursor + relative;
        let Some(marker_len) = prefix_len(&line[marker_start..]) else {
            cursor = marker_start + model::BLOCK_MARKER_PREFIX.len();
            continue;
        };
        if marker_start <= relative_offset && relative_offset < marker_start + marker_len {
            candidate = true;
            break;
        }
        cursor = marker_start + marker_len;
    }
    if !candidate {
        return None;
    }
    hidden_ranges(source)
        .into_iter()
        .find(|range| range.start <= offset && offset < range.end)
}

/// 把源码偏移转换为移除隐藏身份注释之后的偏移。隐藏区间内部的偏移
/// 折叠到区间的可见边界。把交集判断写显式，可以避免过去标记紧跟
/// 光标时出现的那种无符号下溢。
fn visible_offset(source: &str, offset: usize) -> usize {
    let offset = offset.min(source.len());
    let mut visible = offset;
    for range in hidden_ranges(source) {
        if range.start >= offset {
            break;
        }
        let hidden_end = offset.min(range.end);
        visible = visible.saturating_sub(hidden_end.saturating_sub(range.start));
    }
    visible
}

/// 把无标记可见文本中的偏移换算回源码偏移。落在标记边界上时，
/// 选择隐藏区间之后的位置。这样光标停在持久化块的开头/结尾时，
/// 仍与块正文贴合。
fn source_offset(source: &str, visible: usize) -> usize {
    let visible = visible.min(source.len());
    let mut source_at = 0usize;
    let mut visible_at = 0usize;
    for range in hidden_ranges(source) {
        if range.start < source_at {
            continue;
        }
        let visible_segment = range.start - source_at;
        if visible < visible_at + visible_segment {
            return source_at + visible - visible_at;
        }
        visible_at += visible_segment;
        source_at = range.end;
    }
    source_at
        + visible
            .saturating_sub(visible_at)
            .min(source.len().saturating_sub(source_at))
}

pub fn remap_offset(before: &str, after: &str, offset: usize) -> usize {
    source_offset(after, visible_offset(before, offset))
}

fn whitespace_only(source: &str, range: Range<usize>) -> bool {
    source.get(range).is_some_and(|text| text.trim().is_empty())
}

fn line_break_end(source: &str, end: usize) -> usize {
    let end = end.min(source.len());
    if source[end..].starts_with("\r\n") {
        end + 2
    } else if source[end..].starts_with(['\r', '\n']) {
        end + 1
    } else {
        end
    }
}

fn marker_before(source: &str, block_start: usize) -> Option<ValidMarker> {
    valid_markers(source)
        .into_iter()
        .filter(|marker| {
            marker.standalone
                && marker.line.end <= block_start
                && whitespace_only(source, marker.line.end..block_start)
        })
        .max_by_key(|marker| marker.line.start)
}

fn marker_for_block(source: &str, block: Range<usize>) -> Option<ValidMarker> {
    let block = block.start.min(source.len())..block.end.min(source.len());
    valid_markers(source)
        .into_iter()
        .filter(|marker| {
            // 块内部的标记是常见的引用/HTML 形式。
            (marker.comment.start >= block.start && marker.comment.end <= block.end)
                // 独立标记注释的是下一个块。
                || (marker.standalone
                    && marker.line.end <= block.start
                    && whitespace_only(source, marker.line.end..block.start))
                // 通用 HTML 标记追加在块的最后一行。
                || (!marker.standalone
                    && marker.comment.start >= block.end
                    && source
                        .get(block.end..marker.comment.start)
                        .is_some_and(|gap| !gap.contains(['\r', '\n']) && gap.trim().is_empty()))
        })
        .min_by_key(|marker| marker.comment.start.abs_diff(block.start))
}

/// 删除整个语义块时一并消费其持久化标记。这个辅助函数只扫描标记区间；
/// 块边界由调用方传入已解析的结果，输入一个可打印字符不会触发
/// 全文档的 Markdown 解析。
pub fn deletion_range(source: &str, mut range: Range<usize>) -> Range<usize> {
    if source.is_empty() {
        return range;
    }
    let markers = valid_markers(source);
    if markers.is_empty() {
        return range;
    }
    for marker in markers {
        if marker.comment.start < range.end && marker.comment.end > range.start {
            range.start = range.start.min(if marker.standalone {
                marker.line.start
            } else {
                marker.comment.start
            });
            range.end = range.end.max(if marker.standalone {
                marker.line.end
            } else {
                marker.comment.end
            });
            continue;
        }
        // 紧邻目标块之前的独立标记不在编辑器块范围内。连同 CRLF
        // 一起吃掉整行，免得它粘附到下一个块上。
        if marker.standalone
            && marker.line.end <= range.start
            && whitespace_only(source, marker.line.end..range.start)
        {
            range.start = marker.line.start;
            continue;
        }
        // 块末尾的行内 HTML/引用标记位于解析器 span 之外（该 span 不含
        // 尾部注释）。只有与它在同一源码行时才并入，绝不把下一个块的
        // 独立标记误收进来。
        if !marker.standalone
            && marker.comment.start >= range.end
            && source
                .get(range.end..marker.comment.start)
                .is_some_and(|gap| !gap.contains(['\r', '\n']) && gap.trim().is_empty())
        {
            range.end = marker.comment.end;
        }
    }
    range
}

/// 当块的最后一个可见单元被删掉后，删除 `block` 所代表的整个块。
/// 一定要连后面的换行一起删：留下 `<!-- id -->\n\n` 会让后续解析
/// 把旧 ID 挂到新块上。
pub fn deletion_range_for_block(
    source: &str,
    block: Range<usize>,
    range: Range<usize>,
) -> Range<usize> {
    let Some(marker) = marker_for_block(source, block.clone()) else {
        return range;
    };
    let mut result = deletion_range(source, range);
    result.start = result.start.min(if marker.standalone {
        marker.line.start
    } else {
        marker.comment.start.min(block.start)
    });
    result.end = result
        .end
        .max(marker.comment.end)
        .max(block.end)
        .min(source.len());
    result.end = line_break_end(source, result.end);
    result
}

/// 返回紧邻块之前的独立标记行。在带标记块的可见开头处回车时，
/// 用它把标记移到新建的空行/续行之后，保留原 ID。
pub fn standalone_marker_before(source: &str, block_start: usize) -> Option<Range<usize>> {
    marker_before(source, block_start).map(|marker| marker.line)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKER: &str = "<!-- mochi:block block_00000000-0000-4000-8000-000000000001 -->";
    const MARKER_TWO: &str = "<!-- mochi:block block_00000000-0000-4000-8000-000000000002 -->";

    #[test]
    fn only_valid_identity_comments_outside_code_are_hidden() {
        assert!(standalone(MARKER));
        assert!(!standalone("<!-- mochi:block example -->"));
        let source = format!("{MARKER}\n文字\n\n```html\n{MARKER}\n```\n");
        assert_eq!(hidden_ranges(&source), vec![0..MARKER.len() + 1]);
    }

    #[test]
    fn remap_does_not_underflow_when_a_marker_follows_the_caret() {
        let before = "甲\n乙";
        let after = format!("甲\n{MARKER}\n乙");
        assert_eq!(remap_offset(before, &after, 0), 0);
        assert_eq!(
            remap_offset(before, &after, "甲\n".len()),
            "甲\n".len() + MARKER.len() + 1
        );
    }

    #[test]
    fn remap_collapses_carets_inside_removed_marker_lines() {
        let before = format!("甲\n{MARKER}\n乙");
        let after = "甲\n乙";
        let marker_start = before.find(MARKER).unwrap();
        assert_eq!(remap_offset(&before, after, marker_start), "甲\n".len());
        assert_eq!(
            remap_offset(&before, after, marker_start + MARKER.len()),
            "甲\n".len()
        );
    }

    #[test]
    fn deleting_marked_structural_blocks_consumes_crlf_marker_line() {
        let source = format!("{MARKER}\r\n$$x$$\r\n后文");
        let block_start = source.find("$$x$$").unwrap();
        let range = deletion_range(&source, block_start..block_start + "$$x$$".len());
        assert_eq!(
            &source[range.clone()],
            format!("{MARKER}\r\n$$x$$").as_str()
        );
        assert_eq!(&source[range.end..], "\r\n后文");
    }

    #[test]
    fn last_unit_deletion_removes_marker_block_and_its_line_break() {
        let source = format!("{MARKER}\r\n中\r\n{MARKER_TWO}\r\n后");
        let block_start = source.find("中").unwrap();
        let range = deletion_range_for_block(
            &source,
            block_start..block_start + "中".len(),
            block_start..block_start + "中".len(),
        );
        assert_eq!(
            &source[range.clone()],
            format!("{MARKER}\r\n中\r\n").as_str()
        );
        assert_eq!(&source[range.end..], format!("{MARKER_TWO}\r\n后").as_str());
    }

    #[test]
    fn marker_before_api_finds_only_the_nearest_marked_line() {
        let source = format!("{MARKER}\n甲\n{MARKER_TWO}\n乙");
        let second = source.find("乙").unwrap();
        let line_start = source[..second].rfind('\n').unwrap() + 1;
        assert!(standalone_marker_before(&source, line_start).is_some());
        assert!(standalone_marker_before(&source, source.find('甲').unwrap()).is_some());
    }

    #[test]
    fn hidden_range_at_matches_hidden_ranges_for_every_offset_and_syntax_context() {
        let sources = [
            format!("前\r\n{MARKER}\r\n后\r\n行尾"),
            format!("行内 甲 {MARKER} 乙\r\n后文"),
            format!("左 {MARKER} 中 {MARKER_TWO} 右"),
            format!("{} {MARKER}", "普通正文 ".repeat(512)),
            format!("```markdown\r\n{MARKER}\r\n```\r\n{MARKER}\r\n正文"),
            format!("---\r\nmarker: {MARKER}\r\n---\r\n{MARKER}\r\n正文"),
        ];
        for source in sources {
            let ranges = hidden_ranges(&source);
            for offset in 0..=source.len() {
                let expected = ranges
                    .iter()
                    .find(|range| range.start <= offset && offset < range.end)
                    .cloned();
                assert_eq!(
                    hidden_range_at(&source, offset),
                    expected,
                    "offset {offset} in {source:?}"
                );
            }
        }
    }
}
