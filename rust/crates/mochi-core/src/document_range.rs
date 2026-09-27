//! 行号从 1 开始，范围两端都包含；应用提案前核对 original_hash。

use serde::{Deserialize, Serialize};

/// 待确认的局部修改提案，在 UI 与工具层之间传递。
/// 序列化字段名与共享协议中的 `PendingDocumentEdit` 保持一致。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDocumentEdit {
    pub id: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub old_text: String,
    pub new_text: String,
    pub summary: String,
    pub original_hash: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffLineType {
    Added,
    Deleted,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    #[serde(rename = "type")]
    pub kind: DiffLineType,
    pub old_line_num: Option<usize>,
    pub new_line_num: Option<usize>,
    pub content: String,
}

/// 内容指纹：FNV-1a over **UTF-16 码元**。
///
/// 必须按码元而不是 Rust 的 `char` 迭代——JS 的 `charCodeAt` 给的是 UTF-16 码元，
/// emoji 等辅助平面字符编码为两个 UTF-16 码元。按 Unicode 标量值计算会
/// 与 JavaScript 生成不同指纹，使未修改的文本被判定为冲突。
pub fn hash_text(value: &str) -> String {
    let mut hash: u32 = 2166136261;
    for unit in value.encode_utf16() {
        hash ^= u32::from(unit);
        hash = hash.wrapping_mul(16777619);
    }
    format!("{hash:x}")
}

/// 行尾统一成 LF。`\r\n` 和裸 `\r` 都归一，对齐 TS 的 `normalizeTextForSearch`。
pub fn normalize_text(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

fn lines_of(content: &str) -> Vec<String> {
    normalize_text(content)
        .split('\n')
        .map(str::to_owned)
        .collect()
}

/// 取 `[start_line, end_line]`（1-based 闭区间）的原文。
/// 越界部分自动截断，完全越界时返回空串——调用方据此判断行号是否有效。
pub fn line_range_text(content: &str, start_line: usize, end_line: usize) -> String {
    let lines = lines_of(content);
    let start = start_line.max(1);
    let end = end_line.max(start);
    if start > lines.len() {
        return String::new();
    }
    lines[start - 1..end.min(lines.len())].join("\n")
}

/// 文档总行数（按归一化后的 LF 计）。
pub fn line_count(content: &str) -> usize {
    lines_of(content).len()
}

/// 用 `replacement` 替换 `[start_line, end_line]`。
pub fn replace_line_range(
    content: &str,
    start_line: usize,
    end_line: usize,
    replacement: &str,
) -> String {
    let mut lines = lines_of(content);
    let start = start_line.max(1);
    let end = end_line.max(start);
    if start > lines.len() {
        // 越界追加：TS 的 splice 在起点超界时等价于追加到末尾
        lines.extend(lines_of(replacement));
        return lines.join("\n");
    }
    let end = end.min(lines.len());
    let replacement_lines = lines_of(replacement);
    lines.splice(start - 1..end, replacement_lines);
    lines.join("\n")
}

/// 原生源码缓冲区使用：保留未编辑行的原始 CRLF/LF/CR，不重新序列化全文。
pub fn replace_line_range_preserving_endings(
    content: &str,
    start_line: usize,
    end_line: usize,
    replacement: &str,
) -> String {
    let bytes = content.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' || bytes[i] == b'\n' {
            let size = if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            lines.push((start, &content[i..i + size]));
            i += size;
            start = i;
        } else {
            i += 1
        }
    }
    lines.push((start, ""));
    let s = start_line.max(1) - 1;
    let e = end_line
        .max(start_line.max(1))
        .saturating_sub(1)
        .min(lines.len() - 1);
    let ending = lines
        .get(s)
        .map(|(_, e)| *e)
        .filter(|e| !e.is_empty())
        .or_else(|| {
            lines
                .iter()
                .find_map(|(_, e)| (!e.is_empty()).then_some(*e))
        })
        .unwrap_or("\n");
    let replacement = normalize_text(replacement).replace('\n', ending);
    if s >= lines.len() {
        return format!("{content}{ending}{replacement}");
    }
    let after = lines
        .get(e + 1)
        .map(|(offset, _)| *offset)
        .unwrap_or(content.len());
    let separator = if e + 1 < lines.len() { lines[e].1 } else { "" };
    format!(
        "{}{}{}{}",
        &content[..lines[s].0],
        replacement,
        separator,
        &content[after..]
    )
}

/// 逐行比较差异。行号从 `start_line` 开始计数（提案中使用文档的真实行号，而不是从 1 开始）。
pub fn diff_lines(old_text: &str, new_text: &str, start_line: usize) -> Vec<DiffLine> {
    let old_lines = lines_of(old_text);
    let new_lines = lines_of(new_text);
    let mut out = Vec::new();

    for i in 0..old_lines.len().max(new_lines.len()) {
        let old_line = old_lines.get(i);
        let new_line = new_lines.get(i);
        match (old_line, new_line) {
            (Some(a), Some(b)) if a == b => out.push(DiffLine {
                kind: DiffLineType::Unchanged,
                old_line_num: Some(start_line + i),
                new_line_num: Some(start_line + i),
                content: a.clone(),
            }),
            (None, Some(b)) => out.push(DiffLine {
                kind: DiffLineType::Added,
                old_line_num: None,
                new_line_num: Some(start_line + i),
                content: b.clone(),
            }),
            (Some(a), None) => out.push(DiffLine {
                kind: DiffLineType::Deleted,
                old_line_num: Some(start_line + i),
                new_line_num: None,
                content: a.clone(),
            }),
            // 两边都有但不同：删一行、加一行，顺序固定为先删后加
            (Some(a), Some(b)) => {
                out.push(DiffLine {
                    kind: DiffLineType::Deleted,
                    old_line_num: Some(start_line + i),
                    new_line_num: None,
                    content: a.clone(),
                });
                out.push(DiffLine {
                    kind: DiffLineType::Added,
                    old_line_num: None,
                    new_line_num: Some(start_line + i),
                    content: b.clone(),
                });
            }
            (None, None) => unreachable!("循环上界是两者较大值"),
        }
    }
    out
}

/// 命中的行范围与原文。
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionRange {
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
}

/// 在 Markdown 里定位一段选中文本对应的行范围。三级回退：
/// 1. 精确子串
/// 2. 压缩空白后再找（编辑器复制出来的文本常多一个空格）
/// 3. 抹掉 Markdown 标记与标点后按窗口滑动比对（`\(`、`$$`、`**` 之类只在源码里有）
///
/// 全找不到时返回 `None`——**宁可让调用方失败也不要猜一个范围**，
/// 猜错就是把用户没选中的段落改掉。
pub fn find_selection_line_range(markdown: &str, selected_text: &str) -> Option<SelectionRange> {
    let source = normalize_text(markdown);
    let target = normalize_text(selected_text).trim().to_owned();
    if target.is_empty() {
        return None;
    }

    if let Some(index) = source.find(&target) {
        let start_line = source[..index].split('\n').count();
        let end_line = start_line + target.split('\n').count() - 1;
        let lines: Vec<&str> = source.split('\n').collect();
        return Some(SelectionRange {
            start_line,
            end_line,
            text: lines[start_line - 1..end_line.min(lines.len())].join("\n"),
        });
    }

    let compact_target = compact_spaces(&target);
    let compact_source = compact_spaces(&source);
    if let Some(index) = compact_source.find(&compact_target) {
        let approx_line = compact_source[..index].split('\n').count();
        let lines: Vec<&str> = source.split('\n').collect();
        let selected_line_count = compact_target.split('\n').count();
        let start_line = approx_line.max(1);
        let end_line = lines.len().min(start_line + selected_line_count - 1);
        return Some(SelectionRange {
            start_line,
            end_line,
            text: lines[start_line - 1..end_line].join("\n"),
        });
    }

    find_loose(&source, &target)
}

fn compact_spaces(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut in_run = false;
    for ch in value.chars() {
        if ch == ' ' || ch == '\t' {
            if !in_run {
                out.push(' ');
                in_run = true;
            }
        } else {
            in_run = false;
            out.push(ch);
        }
    }
    out
}

/// 抹掉 Markdown 标记、数学定界符与中英标点，再折叠空白并转小写。
fn normalize_loose(value: &str) -> String {
    const DROPPED: &str = "`*_~#>-[](){}|:;,.，。！？、";
    let normalized = normalize_text(value);

    let mut spaced = String::with_capacity(normalized.len());
    let mut chars = normalized.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            // `\(` `\)` `\[` `\]` 是行内公式定界符，整对抹掉
            '\\' if matches!(chars.peek(), Some('(' | ')' | '[' | ']')) => {
                chars.next();
                spaced.push(' ');
            }
            '$' => {
                // `$$` 与 `$` 都当作定界符
                if chars.peek() == Some(&'$') {
                    chars.next();
                }
                spaced.push(' ');
            }
            c if DROPPED.contains(c) => spaced.push(' '),
            c => spaced.push(c),
        }
    }

    spaced
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn find_loose(source: &str, target: &str) -> Option<SelectionRange> {
    let normalized_target = normalize_loose(target);
    if normalized_target.is_empty() {
        return None;
    }

    let source_lines: Vec<&str> = source.split('\n').collect();
    let selected_line_count = normalize_text(target).split('\n').count().max(1);
    // 窗口比选中行数多给 2 行余量：编辑器复制常吞掉首尾的空行
    let max_window = source_lines.len().min(selected_line_count + 2);

    for window_size in selected_line_count..=max_window {
        if window_size == 0 || window_size > source_lines.len() {
            break;
        }
        for start in 0..=source_lines.len() - window_size {
            let text = source_lines[start..start + window_size].join("\n");
            let normalized = normalize_loose(&text);
            if normalized == normalized_target
                || normalized.contains(&normalized_target)
                || normalized_target.contains(&normalized)
            {
                return Some(SelectionRange {
                    start_line: start + 1,
                    end_line: start + window_size,
                    text,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_range_edits_preserve_unedited_line_endings() {
        assert_eq!(
            replace_line_range_preserving_endings("a\r\nb\nc\r\nd", 2, 2, "x\ny"),
            "a\r\nx\ny\nc\r\nd"
        );
        assert_eq!(
            replace_line_range_preserving_endings("a\r\nb\r\n", 2, 2, "中"),
            "a\r\n中\r\n"
        );
        assert_eq!(replace_line_range_preserving_endings("a", 1, 1, ""), "");
    }

    const DOC: &str = "# 标题\n第一段\n第二段\n第三段";

    #[test]
    fn hash_matches_the_js_implementation() {
        // 期望值由 Node 实跑取得，不是手算：
        // node -e "…FNV-1a…" 对下列输入
        assert_eq!(hash_text(""), "811c9dc5");
        assert_eq!(hash_text("a"), "e40c292c");
        assert_eq!(hash_text("hello"), "4f9f2cab");
        assert_eq!(hash_text("中文内容"), "90594953");
    }

    /// emoji 在 JS 里是两个 UTF-16 码元。按 Rust 的 char 迭代会算出不同的指纹，
    /// 于是同一段文本在两版对不上，每个提案都被判成冲突。
    #[test]
    fn hash_iterates_utf16_code_units_not_chars() {
        assert_eq!(hash_text("😀"), "cb31c4b8");
        assert_ne!(
            hash_text("😀"),
            {
                // 按 char（码点）算出来的值，作为反例
                let mut h: u32 = 2166136261;
                for c in "😀".chars() {
                    h ^= c as u32;
                    h = h.wrapping_mul(16777619);
                }
                format!("{h:x}")
            },
            "按码点算和按码元算必须不同，否则这个测试没意义"
        );
    }

    #[test]
    fn line_endings_are_normalized() {
        assert_eq!(normalize_text("a\r\nb\rc\nd"), "a\nb\nc\nd");
        assert_eq!(line_count("a\r\nb"), 2);
        assert_eq!(hash_text(&normalize_text("a\r\nb")), hash_text("a\nb"));
    }

    #[test]
    fn line_range_is_one_based_and_inclusive() {
        assert_eq!(line_range_text(DOC, 2, 3), "第一段\n第二段");
        assert_eq!(line_range_text(DOC, 1, 1), "# 标题");
        assert_eq!(line_range_text(DOC, 4, 4), "第三段");
    }

    #[test]
    fn out_of_range_lines_clamp_or_return_empty() {
        assert_eq!(
            line_range_text(DOC, 3, 99),
            "第二段\n第三段",
            "尾部越界应截断"
        );
        assert_eq!(line_range_text(DOC, 99, 100), "", "整体越界返回空串");
        assert_eq!(line_range_text(DOC, 0, 1), "# 标题", "0 视作 1");
        assert_eq!(
            line_range_text(DOC, 3, 1),
            "第二段",
            "终点小于起点时只取起点那行"
        );
    }

    #[test]
    fn replace_swaps_the_given_range() {
        assert_eq!(
            replace_line_range(DOC, 2, 3, "新内容"),
            "# 标题\n新内容\n第三段"
        );
        assert_eq!(
            replace_line_range(DOC, 2, 2, "甲\n乙"),
            "# 标题\n甲\n乙\n第二段\n第三段",
            "多行替换应整体插入"
        );
        assert_eq!(replace_line_range(DOC, 1, 4, ""), "", "整篇替换成空");
    }

    #[test]
    fn replacing_past_the_end_appends() {
        assert_eq!(
            replace_line_range(DOC, 99, 99, "追加"),
            format!("{DOC}\n追加")
        );
    }

    #[test]
    fn diff_marks_unchanged_added_and_deleted_lines() {
        let diff = diff_lines("甲\n乙", "甲\n丙\n丁", 10);
        assert_eq!(diff.len(), 4);

        assert_eq!(diff[0].kind, DiffLineType::Unchanged);
        assert_eq!(diff[0].old_line_num, Some(10));
        assert_eq!(diff[0].new_line_num, Some(10));

        assert_eq!(diff[1].kind, DiffLineType::Deleted, "改动行先删");
        assert_eq!(diff[1].content, "乙");
        assert_eq!(diff[1].new_line_num, None);

        assert_eq!(diff[2].kind, DiffLineType::Added, "再加");
        assert_eq!(diff[2].content, "丙");
        assert_eq!(diff[2].old_line_num, None);

        assert_eq!(diff[3].kind, DiffLineType::Added);
        assert_eq!(diff[3].content, "丁");
        assert_eq!(diff[3].new_line_num, Some(12));
    }

    #[test]
    fn diff_of_identical_text_is_all_unchanged() {
        let diff = diff_lines("甲\n乙", "甲\n乙", 1);
        assert!(diff.iter().all(|l| l.kind == DiffLineType::Unchanged));
    }

    #[test]
    fn selection_is_located_by_exact_match() {
        let found = find_selection_line_range(DOC, "第二段").unwrap();
        assert_eq!((found.start_line, found.end_line), (3, 3));
        assert_eq!(found.text, "第二段");
    }

    #[test]
    fn multiline_selection_spans_its_lines() {
        let found = find_selection_line_range(DOC, "第一段\n第二段").unwrap();
        assert_eq!((found.start_line, found.end_line), (2, 3));
    }

    /// 从编辑器复制出来的文本常多出空格。
    #[test]
    fn selection_survives_extra_whitespace() {
        let doc = "# 标题\n这是  一段   正文";
        let found = find_selection_line_range(doc, "这是 一段 正文").unwrap();
        assert_eq!(found.start_line, 2);
    }

    /// 渲染后的文本没有 `**`、`$` 这些源码标记，靠宽松匹配兜住。
    #[test]
    fn selection_falls_back_to_loose_matching() {
        let doc = "# 标题\n这里有 **加粗** 和公式 $E=mc^2$。";
        let found = find_selection_line_range(doc, "这里有 加粗 和公式 E=mc^2").unwrap();
        assert_eq!(found.start_line, 2);
        assert_eq!(found.end_line, 2);
    }

    /// 找不到就返回 None——猜一个范围等于把用户没选中的段落改掉。
    #[test]
    fn an_unfindable_selection_returns_none() {
        assert!(find_selection_line_range(DOC, "文档里根本没有这句话啊哈哈").is_none());
        assert!(find_selection_line_range(DOC, "   ").is_none());
        assert!(find_selection_line_range(DOC, "").is_none());
    }

    /// 定位 → 替换 → 指纹，三者必须自洽。
    #[test]
    fn locate_replace_and_hash_agree() {
        let found = find_selection_line_range(DOC, "第二段").unwrap();
        let old_text = line_range_text(DOC, found.start_line, found.end_line);
        assert_eq!(old_text, found.text);
        assert_eq!(hash_text(&old_text), hash_text("第二段"));

        let updated = replace_line_range(DOC, found.start_line, found.end_line, "改过的第二段");
        assert_eq!(updated, "# 标题\n第一段\n改过的第二段\n第三段");
    }
}
