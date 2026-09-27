//! 解析规则以 electron/metadata-index.ts 的 parseNote 为准。

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use regex::{Captures, Regex};

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedProperty {
    pub key: String,
    pub value: String,
    pub value_type: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTag {
    pub tag: String,
    pub source: String,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLink {
    pub target_raw: String,
    pub target_key: String,
    pub kind: String,
    pub heading: Option<String>,
    pub block_id: Option<String>,
    pub alias: Option<String>,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedHeading {
    pub level: u32,
    pub text: String,
    pub line: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedNote {
    pub title: String,
    pub body: String,
    pub properties: Vec<ParsedProperty>,
    pub tags: Vec<ParsedTag>,
    pub links: Vec<ParsedLink>,
    pub headings: Vec<ParsedHeading>,
}

pub const MAX_INDEXABLE_FILE_SIZE: u64 = 2 * 1024 * 1024;
pub const INDEX_BATCH_SIZE: usize = 40;

pub fn indexable_extensions() -> &'static HashSet<&'static str> {
    static S: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
        [
            ".md", ".mc", ".mcb", ".mdx", ".exam", ".txt", ".json", ".js", ".jsx", ".ts", ".tsx",
            ".css", ".html", ".xml", ".yml", ".yaml",
        ]
        .into_iter()
        .collect()
    });
    &S
}

/// 只有这些格式会解析链接/标签/属性；其余仅进全文索引。
pub fn note_extensions() -> &'static HashSet<&'static str> {
    static S: LazyLock<HashSet<&'static str>> =
        LazyLock::new(|| [".md", ".mc", ".mdx", ".txt"].into_iter().collect());
    &S
}

// ---------- 分词 ----------

/// CJK 范围（与旧版逐字一致）：CJK 扩展A / 基本区 / 兼容表意 + 平假名片假名 + 谚文。
/// 写成码位转义而非字面汉字，避免源文件编码影响。
const CJK_CLASS: &str =
    r"\u{3400}-\u{4DBF}\u{4E00}-\u{9FFF}\u{F900}-\u{FAFF}\u{3040}-\u{30FF}\u{AC00}-\u{D7AF}";

static CJK_RANGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("[{CJK_CLASS}]")).unwrap());

/// 索引和查询时都会将中日韩文字逐字拆开，让每个汉字都能单独参与搜索。
pub fn segment_cjk(text: &str) -> String {
    // 用一个输出缓冲，而不是每个 CJK 字符都做一次正则捕获/格式化的分配。
    // 原分词器的区间和空格保持原样。
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        let cjk = matches!(ch, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' |
            '\u{F900}'..='\u{FAFF}' | '\u{3040}'..='\u{30FF}' | '\u{AC00}'..='\u{D7AF}');
        if cjk {
            out.push(' ');
        }
        out.push(ch);
        if cjk {
            out.push(' ');
        }
    }
    out
}

/// 是否含 CJK 字符。搜索的「全字匹配」用它判断——`\b` 对中文永远不成立。
pub fn contains_cjk(text: &str) -> bool {
    CJK_RANGE.is_match(text)
}

/// FTS5 语法字符降级成分隔符。
static FTS_OPERATORS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"["*^:(){}\[\]]"#).unwrap());
static WHITESPACE_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

fn strip_fts_operators(text: &str) -> String {
    let replaced = FTS_OPERATORS.replace_all(text, " ");
    WHITESPACE_RUN
        .replace_all(replaced.trim(), " ")
        .into_owned()
}

/// 单个查询词 → CJK 逐字切分的 phrase，末尾 `*` 保留前缀手感。
pub fn build_fts_phrase(term: &str) -> String {
    let segmented = strip_fts_operators(&segment_cjk(term));
    if segmented.is_empty() {
        String::new()
    } else {
        format!("\"{segmented}\"*")
    }
}

/// 多词 AND：必须全部包含，可散落不同段落。
pub fn build_fts_query_from_terms<'a>(terms: impl IntoIterator<Item = &'a str>) -> String {
    terms
        .into_iter()
        .map(build_fts_phrase)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" AND ")
}

// ---------- 正则 ----------

/// 标签允许的字符：拉丁字母、数字、下划线，以及西欧/西里尔扩展与 CJK。
const TAG_BODY: &str = r"A-Za-z0-9_\u{00C0}-\u{024F}\u{0370}-\u{04FF}\u{4E00}-\u{9FFF}\u{3400}-\u{4DBF}\u{3040}-\u{30FF}\u{AC00}-\u{D7AF}";

macro_rules! lazy_re {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pattern).unwrap());
    };
}

lazy_re!(
    HTML_BLOCK,
    r"(?is)<script[\s\S]*?</script>|<style[\s\S]*?</style>"
);
lazy_re!(HTML_TAG, r"<[^>]+>");
lazy_re!(FRONTMATTER, r"(?s)^---\r?\n(.*?)\r?\n---(?:\r?\n|$)");
lazy_re!(LIST_ITEM, r"^\s*-\s+(.*)$");
lazy_re!(CODE_BLOCK, r"(?s)```.*?```");
lazy_re!(INLINE_CODE, r"`[^`\n]*`");
lazy_re!(WIKI_LINK, r"(!?)\[\[([^\[\]]+?)\]\]");
lazy_re!(
    MARKDOWN_LINK,
    r#"(!?)\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#
);
lazy_re!(HEADING, r"^(#{1,6})\s+(.+?)\s*$");
lazy_re!(BOOL_VALUE, r"(?i)^(true|false|yes|no)$");
lazy_re!(NUMBER_VALUE, r"^-?\d+(\.\d+)?$");
lazy_re!(DATE_VALUE, r"^\d{4}-\d{2}-\d{2}([T ]\d{2}:\d{2})?");
lazy_re!(EXTERNAL_SCHEME, r"(?i)^(https?|mailto|tel|data):");
lazy_re!(ASCII_DIGITS_ONLY, r"^\d+$");

static FRONTMATTER_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z0-9_\u{4E00}-\u{9FFF}][^:]*?):\s*(.*)$").unwrap());

static TAG_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(^|[\s(（，、.,;:!?])#([{TAG_BODY}][{TAG_BODY}/-]*)"
    ))
    .unwrap()
});

// ---------- 清洗 ----------

pub fn strip_html_tags(html: &str) -> String {
    let no_blocks = HTML_BLOCK.replace_all(html, " ");
    HTML_TAG
        .replace_all(&no_blocks, " ")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

/// 去掉代码块与行内代码，避免把示例代码里的 `#xxx` 误当标签。
///
/// 关键细节：围栏代码块里**只把非换行字符换成空格**（TS: `match.replace(/[^\n]/g,' ')`），
/// 换行必须保留，否则块之后所有行的行号都会偏移。
/// C# 版 `NoteParser.cs:121` 用 `new string(' ', len)` 连换行一起吞了，行号是错的。
pub fn strip_code(text: &str) -> String {
    let blanked = CODE_BLOCK.replace_all(text, |c: &Captures| {
        c[0].chars()
            .map(|ch| if ch == '\n' { '\n' } else { ' ' })
            .collect::<String>()
    });
    INLINE_CODE.replace_all(&blanked, " ").into_owned()
}

pub fn infer_value_type(raw: &str) -> String {
    let value = raw.trim();
    if value.is_empty() {
        return "text".into();
    }
    if BOOL_VALUE.is_match(value) {
        return "boolean".into();
    }
    if NUMBER_VALUE.is_match(value) {
        return "number".into();
    }
    if DATE_VALUE.is_match(value) {
        return "date".into();
    }
    "text".into()
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches(['"', '\'']).trim().to_owned()
}

/// 轻量解析文档头部：支持 `key: value`、`key: [a, b]`，以及 `key:` 后跟 `- item` 列表。
pub fn parse_frontmatter(source: &str) -> Vec<ParsedProperty> {
    let lines: Vec<&str> = split_lines(source);
    let mut properties = Vec::new();
    let mut index = 0usize;

    while index < lines.len() {
        let Some(m) = FRONTMATTER_LINE.captures(lines[index]) else {
            index += 1;
            continue;
        };
        let key = m[1].trim().to_owned();
        let inline = m[2].trim().to_owned();

        if inline.starts_with('[') && inline.ends_with(']') {
            for item in inline[1..inline.len() - 1].split(',') {
                let item = unquote(item);
                if !item.is_empty() {
                    properties.push(ParsedProperty {
                        key: key.clone(),
                        value: item,
                        value_type: "list".into(),
                    });
                }
            }
            index += 1;
            continue;
        }

        if !inline.is_empty() {
            properties.push(ParsedProperty {
                key,
                value: unquote(&inline),
                value_type: infer_value_type(&inline),
            });
            index += 1;
            continue;
        }

        // key: 空值 → 向下收集 "- item"
        let mut cursor = index + 1;
        let mut collected = 0;
        while cursor < lines.len() {
            let Some(item_match) = LIST_ITEM.captures(lines[cursor]) else {
                break;
            };
            let item = unquote(&item_match[1]);
            if !item.is_empty() {
                properties.push(ParsedProperty {
                    key: key.clone(),
                    value: item,
                    value_type: "list".into(),
                });
                collected += 1;
            }
            cursor += 1;
        }
        if collected == 0 {
            properties.push(ParsedProperty {
                key,
                value: String::new(),
                value_type: "text".into(),
            });
        }
        index = cursor.max(index + 1);
    }

    properties
}

/// 与 TS 的 `split(/\r?\n/)` 等价。
fn split_lines(s: &str) -> Vec<&str> {
    s.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

pub fn parse_note(file_path: &str, raw_content: &str) -> ParsedNote {
    let path = Path::new(file_path);
    let extension = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    let title = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    if extension == ".mcb" {
        let body = crate::base::parse_base_document(raw_content)
            .map(|document| {
                let mut lines = Vec::new();
                for table in document.tables {
                    lines.push(table.name);
                    lines.extend(table.fields.iter().map(|field| field.name.clone()));
                    for record in &table.records {
                        for field in &table.fields {
                            if let Some(value) = record.values.get(&field.id) {
                                let formatted = crate::base::format_cell_value(field, value);
                                if !formatted.is_empty() {
                                    lines.push(format!("{}: {}", field.name, formatted));
                                }
                            }
                        }
                    }
                }
                lines.join("\n")
            })
            .unwrap_or_default();
        return ParsedNote {
            title,
            body,
            ..Default::default()
        };
    }

    let fm = FRONTMATTER.captures(raw_content);
    let properties = fm
        .as_ref()
        .map(|m| parse_frontmatter(&m[1]))
        .unwrap_or_default();
    let without_frontmatter = match fm.as_ref() {
        Some(m) => &raw_content[m.get(0).unwrap().end()..],
        None => raw_content,
    };

    let text_body = if extension == ".mc" || extension == ".html" {
        strip_html_tags(without_frontmatter)
    } else {
        without_frontmatter.to_owned()
    };

    let mut result = ParsedNote {
        title,
        body: text_body,
        properties,
        ..Default::default()
    };

    for property in &result.properties {
        if property.key != "tags" && property.key != "tag" {
            continue;
        }
        if !property.value.is_empty() {
            result.tags.push(ParsedTag {
                tag: property.value.clone(),
                source: "frontmatter".into(),
                line: None,
            });
        }
    }

    if !note_extensions().contains(extension.as_str()) {
        return result;
    }

    let scan_source = strip_code(&result.body);
    for (i, line) in split_lines(&scan_source).into_iter().enumerate() {
        let line_number = (i + 1) as u32;

        if let Some(m) = HEADING.captures(line) {
            result.headings.push(ParsedHeading {
                level: m[1].len() as u32,
                text: m[2].trim().to_owned(),
                line: line_number,
            });
        }

        for m in TAG_PATTERN.captures_iter(line) {
            let tag = m[2].to_owned();
            // 纯数字（议题编号/色值）不当标签，与 JS /^\d+$/ 一致
            if ASCII_DIGITS_ONLY.is_match(&tag) {
                continue;
            }
            result.tags.push(ParsedTag {
                tag,
                source: "inline".into(),
                line: Some(line_number),
            });
        }

        for m in WIKI_LINK.captures_iter(line) {
            let is_embed = &m[1] == "!";
            let inner = &m[2];
            let (target_part, alias) = match inner.split_once('|') {
                Some((t, a)) if !a.trim().is_empty() => (t, Some(a.trim().to_owned())),
                Some((t, _)) => (t, None),
                None => (inner, None),
            };

            let mut target = target_part.trim().to_owned();
            let mut heading = None;
            let mut block_id = None;

            if let Some(hash) = target.find('#') {
                let fragment = target[hash + 1..].trim().to_owned();
                target = target[..hash].trim().to_owned();
                if let Some(rest) = fragment.strip_prefix('^') {
                    block_id = Some(rest.to_owned());
                } else if !fragment.is_empty() {
                    heading = Some(fragment);
                }
            }

            if target.is_empty() && block_id.is_none() && heading.is_none() {
                continue;
            }

            let target_key = object_link_target_key(&target)
                .unwrap_or_else(|| normalize_link_target_exact(&target));
            result.links.push(ParsedLink {
                target_raw: target_part.trim().to_owned(),
                target_key,
                kind: if is_embed { "embed" } else { "wiki" }.into(),
                heading,
                block_id,
                alias,
                line: line_number,
            });
        }

        for m in MARKDOWN_LINK.captures_iter(line) {
            let raw_target = m[3].trim();
            if raw_target.is_empty()
                || EXTERNAL_SCHEME.is_match(raw_target)
                || raw_target.starts_with('#')
            {
                continue;
            }

            let mut parts = raw_target.splitn(2, '#');
            let path_part = parts.next().unwrap_or_default();
            if path_part.is_empty() {
                continue;
            }
            let fragment = parts.next().filter(|f| !f.is_empty());
            let alias = m[2].trim();

            let target_key = object_link_target_key(path_part)
                .unwrap_or_else(|| normalize_link_target_exact(&decode_uri(path_part)));
            result.links.push(ParsedLink {
                target_raw: raw_target.to_owned(),
                target_key,
                kind: if &m[1] == "!" { "embed" } else { "markdown" }.into(),
                heading: fragment.map(decode_uri),
                block_id: None,
                alias: (!alias.is_empty()).then(|| alias.to_owned()),
                line: line_number,
            });
        }

        // 对象选择器生成的链接也会以「一行裸 Mochi URL」的形式存在 Markdown
        // 文档里。这些行与包裹式链接进同一个链接索引，之后重命名就能直接
        // 更新未打开的文件，不必全工作区扫描。AI locator 没有文件系统目标，
        // 刻意留给点击路由去处理。
        let standalone = line.trim();
        if let Some(reference) =
            crate::object_reference::ObjectReference::parse(standalone).filter(|reference| {
                matches!(
                    reference.kind,
                    crate::object_reference::ObjectKind::Document
                        | crate::object_reference::ObjectKind::Directory
                        | crate::object_reference::ObjectKind::Record
                        | crate::object_reference::ObjectKind::Block
                )
            })
        {
            if let Some(path) = reference.path {
                result.links.push(ParsedLink {
                    target_raw: standalone.to_owned(),
                    target_key: normalize_link_target_exact(&path),
                    kind: "object".into(),
                    heading: None,
                    block_id: None,
                    alias: reference.label,
                    line: line_number,
                });
            }
        }
    }

    result
}

/// `target_key` 归一化：反斜杠→斜杠、剥开头 `./`、转小写。
pub fn normalize_link_target_exact(target: &str) -> String {
    let t = target.replace('\\', "/");
    let t = t.strip_prefix("./").unwrap_or(&t);
    t.trim().to_lowercase()
}

/// 取出文档/记录/块 Mochi URL 编码的文件系统目标。日程和 AI 引用虽然
/// 同用 `mochi://open` 协议，但刻意不参与文档重命名的维护。
pub fn object_link_target_key(target: &str) -> Option<String> {
    let target = target.split_once('#').map_or(target, |(base, _)| base);
    let reference = crate::object_reference::ObjectReference::parse(target)?;
    if !matches!(
        reference.kind,
        crate::object_reference::ObjectKind::Document
            | crate::object_reference::ObjectKind::Directory
            | crate::object_reference::ObjectKind::Record
            | crate::object_reference::ObjectKind::Block
    ) {
        return None;
    }
    reference
        .path
        .map(|path| normalize_link_target_exact(&path))
}

/// `decodeURIComponent` 的等价物；非法百分号序列原样返回（与 TS 的 try/catch 一致）。
fn decode_uri(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(b) => {
                    out.push(b);
                    i += 3;
                    continue;
                }
                None => return value.to_owned(),
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_cjk_isolates_each_han_character() {
        assert_eq!(segment_cjk("墨池"), " 墨  池 ");
        assert_eq!(segment_cjk("abc"), "abc", "拉丁字母不该被切开");
        assert_eq!(segment_cjk("a墨b"), "a 墨 b");
    }

    #[test]
    fn optimized_segmentation_matches_legacy_regex_for_all_unicode_scalars() {
        // 全 Unicode 标量逐一覆盖，保护分词器/索引的兼容性：
        // 边界、假名、谚文、增补平面汉字和 emoji 都要顾到。
        let input: String = (0..=0x10ffff).filter_map(char::from_u32).collect();
        let expected = CJK_RANGE.replace_all(&input, |c: &Captures| format!(" {} ", &c[0]));
        assert_eq!(segment_cjk(&input), expected);
    }

    #[test]
    fn fts_phrase_wraps_and_suffixes_star() {
        assert_eq!(build_fts_phrase("墨池"), "\"墨 池\"*");
        assert_eq!(build_fts_phrase("hello"), "\"hello\"*");
        assert_eq!(build_fts_phrase("  "), "", "空词应产生空串");
    }

    /// FTS5 语法字符必须降级，否则用户输入 `"` 会让整条查询语法错误。
    #[test]
    fn fts_operators_are_neutralized() {
        assert_eq!(build_fts_phrase("a\"b*c"), "\"a b c\"*");
        assert_eq!(build_fts_phrase("(x)"), "\"x\"*");
    }

    #[test]
    fn multi_term_query_is_anded() {
        assert_eq!(
            build_fts_query_from_terms(["墨池", "笔记"]),
            "\"墨 池\"* AND \"笔 记\"*"
        );
        assert_eq!(build_fts_query_from_terms(["ok", "  "]), "\"ok\"*");
    }

    #[test]
    fn frontmatter_inline_list_and_block_list() {
        let props = parse_frontmatter("title: 我的笔记\ntags: [a, \"b\", '']\nrefs:\n  - x\n  - y");
        let get = |k: &str| -> Vec<&str> {
            props
                .iter()
                .filter(|p| p.key == k)
                .map(|p| p.value.as_str())
                .collect()
        };
        assert_eq!(get("title"), ["我的笔记"]);
        assert_eq!(get("tags"), ["a", "b"], "空项应被丢弃");
        assert_eq!(get("refs"), ["x", "y"]);
    }

    #[test]
    fn frontmatter_empty_value_without_list_becomes_empty_text() {
        let props = parse_frontmatter("author:\nnext: v");
        let author = props.iter().find(|p| p.key == "author").unwrap();
        assert_eq!(author.value, "");
        assert_eq!(author.value_type, "text");
        assert!(props.iter().any(|p| p.key == "next"), "不该吞掉后一行");
    }

    #[test]
    fn value_types() {
        assert_eq!(infer_value_type("true"), "boolean");
        assert_eq!(infer_value_type("NO"), "boolean");
        assert_eq!(infer_value_type("-12.5"), "number");
        assert_eq!(infer_value_type("2026-08-28"), "date");
        assert_eq!(infer_value_type("2026-08-28T10:00"), "date");
        assert_eq!(infer_value_type("随便"), "text");
        assert_eq!(infer_value_type(""), "text");
    }

    #[test]
    fn parse_note_extracts_headings_tags_and_links() {
        let src = "# 标题\n\n正文 #标签 和 [[目标|别名]] 与 [文字](./路径.md#锚)\n";
        let note = parse_note("D:/ws/note.md", src);

        assert_eq!(note.title, "note");
        assert_eq!(
            note.headings,
            vec![ParsedHeading {
                level: 1,
                text: "标题".into(),
                line: 1
            }]
        );

        let tags: Vec<&str> = note.tags.iter().map(|t| t.tag.as_str()).collect();
        assert_eq!(tags, ["标签"]);
        assert_eq!(note.tags[0].line, Some(3));

        let wiki = note.links.iter().find(|l| l.kind == "wiki").unwrap();
        assert_eq!(wiki.target_key, "目标");
        assert_eq!(wiki.alias.as_deref(), Some("别名"));

        let md = note.links.iter().find(|l| l.kind == "markdown").unwrap();
        assert_eq!(md.target_key, "路径.md", "开头的 ./ 应被剥掉");
        assert_eq!(md.heading.as_deref(), Some("锚"));
    }

    #[test]
    fn parse_note_indexes_standalone_mochi_document_links_by_path() {
        let url = "mochi://open?path=%E7%9F%A5%E8%AF%86%E5%BA%93%2F%E6%93%8D%E4%BD%9C%E7%B3%BB%E7%BB%9F.md&label=%E8%87%AA%E5%B7%B1&view=card";
        let note = parse_note("引用.md", &format!("  {url}\n"));
        assert_eq!(note.links.len(), 1);
        assert_eq!(note.links[0].kind, "object");
        assert_eq!(note.links[0].target_key, "知识库/操作系统.md");
        assert_eq!(note.links[0].target_raw, url);
    }

    #[test]
    fn parse_note_indexes_standalone_mochi_block_links_by_document_path() {
        let url = "mochi://block?path=notes%2F引用.md&id=block_123e4567-e89b-12d3-a456-426614174000&label=块";
        let note = parse_note("引用.md", &format!("{url}\n"));
        assert_eq!(note.links.len(), 1);
        assert_eq!(note.links[0].kind, "object");
        assert_eq!(note.links[0].target_key, "notes/引用.md");
        assert_eq!(note.links[0].target_raw, url);
        assert_eq!(
            object_link_target_key(url).as_deref(),
            Some("notes/引用.md")
        );
    }

    #[test]
    fn wiki_link_fragments_split_into_heading_or_block_id() {
        let note = parse_note("a.md", "[[目标#章节]] [[目标#^block1]] [[#^仅块]]");
        let by_kind: Vec<_> = note.links.iter().collect();
        assert_eq!(by_kind[0].heading.as_deref(), Some("章节"));
        assert_eq!(by_kind[1].block_id.as_deref(), Some("block1"));
        assert_eq!(by_kind[2].block_id.as_deref(), Some("仅块"));
        assert_eq!(by_kind[2].target_key, "", "只有块 id 时目标为空但仍要收录");
    }

    #[test]
    fn embeds_are_marked() {
        let note = parse_note("a.md", "![[图片.png]] ![alt](./图.png)");
        assert!(
            note.links.iter().all(|l| l.kind == "embed"),
            "{:?}",
            note.links
        );
    }

    #[test]
    fn external_and_anchor_links_are_skipped() {
        let note = parse_note(
            "a.md",
            "[a](https://x.com) [b](mailto:a@b.c) [c](#锚点) [d](tel:123)",
        );
        assert!(note.links.is_empty(), "{:?}", note.links);
    }

    #[test]
    fn numeric_tags_are_ignored() {
        let note = parse_note("a.md", "issue #123 and #v2 and #标签");
        let tags: Vec<&str> = note.tags.iter().map(|t| t.tag.as_str()).collect();
        assert_eq!(tags, ["v2", "标签"]);
    }

    /// 关键回归：围栏代码块必须保留换行，否则块之后的行号全错。
    /// C# 版在这里失配。
    #[test]
    fn code_blocks_preserve_line_count() {
        let src = "行1\n```\n#假标签\n还是代码\n```\n#真标签\n";
        let note = parse_note("a.md", src);

        let tags: Vec<&str> = note.tags.iter().map(|t| t.tag.as_str()).collect();
        assert_eq!(tags, ["真标签"], "代码块里的 # 不该成为标签");
        assert_eq!(
            note.tags[0].line,
            Some(6),
            "代码块吞掉换行会让行号偏移；实得 {:?}",
            note.tags[0].line
        );
    }

    #[test]
    fn inline_code_is_stripped() {
        let note = parse_note("a.md", "正文 `#不是标签` 和 #是标签");
        let tags: Vec<&str> = note.tags.iter().map(|t| t.tag.as_str()).collect();
        assert_eq!(tags, ["是标签"]);
    }

    #[test]
    fn frontmatter_is_removed_from_body_and_feeds_tags() {
        let note = parse_note("a.md", "---\ntitle: T\ntags: [甲, 乙]\n---\n正文\n");
        assert!(!note.body.contains("title:"), "frontmatter 没从正文里剥掉");
        assert!(note.body.starts_with("正文"));
        let fm_tags: Vec<&str> = note
            .tags
            .iter()
            .filter(|t| t.source == "frontmatter")
            .map(|t| t.tag.as_str())
            .collect();
        assert_eq!(fm_tags, ["甲", "乙"]);
    }

    #[test]
    fn mc_and_html_bodies_are_stripped_of_markup() {
        let note = parse_note("a.mc", "<p>正文</p><script>var x=1</script>");
        assert!(!note.body.contains("<p>"));
        assert!(!note.body.contains("var x"), "script 内容应整块移除");
        assert!(note.body.contains("正文"));
    }

    #[test]
    fn non_note_extensions_skip_link_and_tag_scan() {
        let note = parse_note("a.json", "#标签 [[目标]]");
        assert!(note.tags.is_empty());
        assert!(note.links.is_empty());
        assert!(!note.body.is_empty(), "正文仍应保留以进全文索引");
    }

    #[test]
    fn base_search_indexes_labels_without_treating_content_as_markdown() {
        use crate::base::*;
        let mut document = create_base_document();
        let table = &mut document.tables[0];
        let mut field = create_base_field(FieldType::SingleSelect, "分类");
        field.options.push(BaseOption {
            id: "option-id".into(),
            label: "标签内容".into(),
            color: OptionColor::Blue,
            ..Default::default()
        });
        let mut record = BaseRecord {
            id: "record-id".into(),
            ..Default::default()
        };
        record
            .values
            .insert(field.id.clone(), serde_json::json!("option-id"));
        record.values.insert(
            table.fields[0].id.clone(),
            serde_json::json!("#内容 [[原样文本]]"),
        );
        table.fields.push(field);
        table.records.push(record);
        let note = parse_note("数据.MCB", &serialize_base_document(&document).unwrap());
        assert_eq!(note.title, "数据");
        assert!(note.body.contains("标签内容"));
        assert!(!note.body.contains("option-id"));
        assert!(note.links.is_empty());
        assert!(note.tags.is_empty());
        assert!(parse_note("无效.mcb", "invalid").body.is_empty());
        assert!(indexable_extensions().contains(".mcb"));
    }

    #[test]
    fn link_target_normalization() {
        assert_eq!(normalize_link_target_exact("./A\\B.md"), "a/b.md");
        assert_eq!(normalize_link_target_exact("  X.MD  "), "x.md");
    }

    #[test]
    fn percent_encoded_paths_are_decoded() {
        let note = parse_note("a.md", "[x](./%E7%AC%94%E8%AE%B0.md)");
        assert_eq!(note.links[0].target_key, "笔记.md");
    }

    #[test]
    fn malformed_percent_encoding_is_left_alone() {
        let note = parse_note("a.md", "[x](./%ZZ.md)");
        assert_eq!(note.links[0].target_key, "%zz.md");
    }
}
