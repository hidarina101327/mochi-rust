//! 对齐 Electron computeRenamedTarget / rewriteLinkOccurrences；只替换链接目标字节区间。
use crate::metadata_index::NoteLink;
use std::{collections::HashSet, ops::Range, path::Path};
#[derive(Clone, Debug)]
pub struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
}
pub fn renamed_target(raw: &str, path: &Path) -> String {
    let (base, fragment) = raw
        .find('#')
        .map(|i| (&raw[..i], &raw[i..]))
        .unwrap_or((raw, ""));
    let normalized = base.replace('\\', "/");
    let cut = normalized.rfind('/').map(|i| i + 1).unwrap_or(0);
    let name = &normalized[cut..];
    let title = path.file_stem().unwrap_or_default().to_string_lossy();
    let keeps_extension = name.rsplit_once('.').is_some_and(|(_, e)| !e.is_empty());
    let ext = if keeps_extension {
        path.extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!("{}{title}{ext}{fragment}", &normalized[..cut])
}

fn target_key(raw: &str) -> String {
    crate::note_parser::object_link_target_key(raw)
        .unwrap_or_else(|| crate::note_parser::normalize_link_target_exact(raw))
}

fn rename_object_target(
    raw: &str,
    old_path: &Path,
    new_path: &Path,
    workspace_root: &Path,
) -> String {
    let (base, fragment) = raw
        .find('#')
        .map(|i| (&raw[..i], &raw[i..]))
        .unwrap_or((raw, ""));
    let root = workspace_root.to_string_lossy().into_owned();
    let old = old_path.to_string_lossy().into_owned();
    let new = new_path.to_string_lossy().into_owned();
    let renamed = crate::base_reference_paths::rename_base_reference(
        base,
        &crate::base_reference_paths::BaseReferenceRename {
            workspace_path: &root,
            old_path: &old,
            new_path: &new,
        },
    );
    if renamed == base {
        raw.to_owned()
    } else {
        format!("{renamed}{fragment}")
    }
}

pub fn edits(source_path: &Path, source: &str, indexed: &[NoteLink], new_path: &Path) -> Vec<Edit> {
    edits_internal(source_path, source, indexed, None, new_path, None)
}

/// 把 `old_path` 移动到 `new_path` 时改写相关链接。
///
/// 旧版 [`edits`] API 只知道目的地，普通 wikilink 继续用它。
/// Mochi URL 则需要工作区根目录和前后两个绝对路径，才能只改
/// 编码过的 `path` 参数，而不动标签、视图、ID 和未来的其他参数。
pub fn edits_for_move(
    source_path: &Path,
    source: &str,
    indexed: &[NoteLink],
    old_path: &Path,
    new_path: &Path,
    workspace_root: &Path,
) -> Vec<Edit> {
    edits_internal(
        source_path,
        source,
        indexed,
        Some(old_path),
        new_path,
        Some(workspace_root),
    )
}

fn edits_internal(
    source_path: &Path,
    source: &str,
    indexed: &[NoteLink],
    old_path: Option<&Path>,
    new_path: &Path,
    workspace_root: Option<&Path>,
) -> Vec<Edit> {
    let _ = source_path;
    let keys = indexed
        .iter()
        .map(|l| (l.kind.clone(), target_key(&l.target_raw)))
        .collect::<HashSet<_>>();
    // 使用原始 Markdown 取坐标，不能使用 .mc 的 HTML 纯文本索引坐标。
    let parsed = crate::note_parser::parse_note("reference.md", source);
    let prefix = source.len().saturating_sub(parsed.body.len());
    let line_shift = source[..prefix].bytes().filter(|b| *b == b'\n').count();
    let mut masked = source.as_bytes().to_vec();
    let code = regex::Regex::new(r"(?s)```.*?```|~~~.*?~~~|`[^`\r\n]*`").unwrap();
    for m in code.find_iter(source) {
        for b in &mut masked[m.range()] {
            if *b != b'\n' && *b != b'\r' {
                *b = b' ';
            }
        }
    }
    let masked = String::from_utf8(masked).unwrap();
    let mut starts = Vec::new();
    let mut offset = 0;
    for line in masked.split_inclusive('\n') {
        starts.push((offset, line));
        offset += line.len();
    }
    let mut seen = HashSet::new();
    let mut edits = Vec::new();
    for link in parsed.links {
        let is_object = crate::note_parser::object_link_target_key(&link.target_raw).is_some();
        let indexed_match = keys.contains(&(link.kind.clone(), target_key(&link.target_raw)));
        // 脏的打开缓冲里可能有刚插入、还没进元数据索引的对象 URL。
        // 在这里纳入判断是安全的：rename_base_reference 下方仍会
        // 校验确切的旧路径。
        if (!is_object && !indexed_match)
            || !seen.insert((link.line, link.kind.clone(), link.target_raw.clone()))
        {
            continue;
        }
        let Some((offset, line)) = starts.get(link.line.saturating_sub(1) as usize + line_shift)
        else {
            continue;
        };
        let target = if is_object {
            let (Some(old_path), Some(workspace_root)) = (old_path, workspace_root) else {
                continue;
            };
            rename_object_target(&link.target_raw, old_path, new_path, workspace_root)
        } else {
            renamed_target(&link.target_raw, new_path)
        };
        if target == link.target_raw {
            continue;
        }
        let markdown_style = link.kind == "markdown"
            || (link.kind == "embed" && line.contains(&format!("]({}", link.target_raw)));
        if link.kind == "object" {
            for start in line.match_indices(&link.target_raw).map(|(start, _)| start) {
                edits.push(Edit {
                    range: offset + start..offset + start + link.target_raw.len(),
                    replacement: target.clone(),
                });
            }
            continue;
        }
        let pattern = if markdown_style {
            format!(r"\]\({}([)\s])", regex::escape(&link.target_raw))
        } else {
            format!(r"(!?\[\[){}(\||\]\])", regex::escape(&link.target_raw))
        };
        let Ok(pattern) = regex::Regex::new(&pattern) else {
            continue;
        };
        for capture in pattern.captures_iter(line) {
            let start = offset
                + if markdown_style {
                    capture.get(0).unwrap().start() + 2
                } else {
                    capture.get(1).unwrap().end()
                };
            edits.push(Edit {
                range: start..start + link.target_raw.len(),
                replacement: target.clone(),
            });
        }
    }
    edits.sort_by_key(|e| e.range.start);
    edits.dedup_by(|a, b| a.range == b.range);
    edits
}
pub fn apply(source: &str, edits: &[Edit]) -> String {
    let mut result = source.to_owned();
    for edit in edits.iter().rev() {
        result.replace_range(edit.range.clone(), &edit.replacement);
    }
    result
}
pub fn map_offset(offset: usize, edits: &[Edit]) -> usize {
    let mut delta = 0isize;
    for edit in edits {
        if offset < edit.range.start {
            break;
        }
        if offset < edit.range.end {
            return (edit.range.start as isize + delta) as usize
                + (offset - edit.range.start).min(edit.replacement.len());
        }
        delta += edit.replacement.len() as isize - edit.range.len() as isize;
    }
    (offset as isize + delta).max(0) as usize
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_preserves_alias_fragments_and_crlf_even_after_unsaved_line_insertions() {
        let source = "插入的新行\r\n[[旧#标题|显示名]] 和 ![[旧]]\r\n[说明](旧.md#^块)\r\n";
        let parsed = crate::note_parser::parse_note("ref.md", source);
        let links = parsed
            .links
            .iter()
            .map(|l| NoteLink {
                source_path: "ref.md".into(),
                source_title: String::new(),
                target_raw: l.target_raw.clone(),
                target_path: None,
                kind: l.kind.clone(),
                heading: None,
                block_id: None,
                alias: None,
                line: 1,
                context: String::new(),
            })
            .collect::<Vec<_>>();
        let edits = edits(Path::new("ref.md"), source, &links, Path::new("新名字.md"));
        let result = apply(source, &edits);
        assert_eq!(
            result,
            "插入的新行\r\n[[新名字#标题|显示名]] 和 ![[新名字]]\r\n[说明](新名字.md#^块)\r\n"
        );
        assert_eq!(map_offset(source.len(), &edits), result.len());
    }
    #[test]
    fn target_keeps_path_and_extension_style() {
        assert_eq!(
            renamed_target("../folder/old.md#h", Path::new("new.md")),
            "../folder/new.md#h"
        );
        assert_eq!(
            renamed_target("old#^block", Path::new("new.mc")),
            "new#^block"
        );
    }

    #[test]
    fn rename_rewrites_mochi_urls_and_preserves_query_metadata() {
        let old = Path::new("D:/ws/notes/old.md");
        let new = Path::new("D:/ws/notes/new.mc");
        let raw = "mochi://open?path=notes%2Fold.md&label=alias&view=card&future=x%2Fy";
        let source = format!("{raw}\n[卡片]({raw}#标题)\n");
        let edits = edits_for_move(
            Path::new("D:/ws/index.md"),
            &source,
            &[],
            old,
            new,
            Path::new("D:/ws"),
        );
        let result = apply(&source, &edits);
        assert!(result.contains("path=notes%2Fnew.mc"), "{result}");
        assert!(
            result.contains("&label=alias&view=card&future=x%2Fy"),
            "{result}"
        );
        assert!(result.contains("future=x%2Fy#标题"), "{result}");
        assert!(!result.contains("path=notes%2Fold.md"), "{result}");
    }

    #[test]
    fn rename_rewrites_block_urls_without_merging_their_ids() {
        let old = Path::new("D:/ws/notes/old.md");
        let new = Path::new("D:/ws/notes/new.md");
        let id = "block_00000000-0000-4000-8000-000000000001";
        let raw =
            format!("mochi://block?path=notes%2Fold.md&id={id}&label=alias&view=card&future=x%2Fy");
        let source = format!("{raw}\n[卡片]({raw}#标题)\n");
        let edits = edits_for_move(
            Path::new("D:/ws/index.md"),
            &source,
            &[],
            old,
            new,
            Path::new("D:/ws"),
        );
        let result = apply(&source, &edits);
        assert_eq!(result.matches("path=notes%2Fnew.md").count(), 2, "{result}");
        assert_eq!(result.matches(&format!("&id={id}")).count(), 2, "{result}");
        assert!(
            result.contains("&label=alias&view=card&future=x%2Fy"),
            "{result}"
        );
        assert!(result.contains("future=x%2Fy#标题"), "{result}");
    }

    #[test]
    fn frontmatter_and_inline_code_do_not_shift_or_create_edits() {
        let source = "---\r\ntitle: test\r\n---\r\n`[[old]]` and [[old]]\r\n";
        let link = NoteLink {
            source_path: "a.md".into(),
            source_title: String::new(),
            target_raw: "old".into(),
            target_path: None,
            kind: "wiki".into(),
            heading: None,
            block_id: None,
            alias: None,
            line: 1,
            context: String::new(),
        };
        let edits = edits(Path::new("a.md"), source, &[link], Path::new("new.md"));
        assert_eq!(
            apply(source, &edits),
            "---\r\ntitle: test\r\n---\r\n`[[old]]` and [[new]]\r\n"
        );
    }
}
