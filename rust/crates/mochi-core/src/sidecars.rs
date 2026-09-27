//! 批注文件名需净化，路径用正斜杠；PDF 标注保留文件名和原路径分隔符。
//! JSON 写入尾换行；损坏的伴生文件按空数据读取。

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{json2, jstime};

// ---------------------------------------------------------------------------
// 批注 / 评论
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentAttachment {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentAnchor {
    #[serde(rename = "type")]
    pub kind: String,
    pub selected_text: String,
    pub block_text: String,
    pub block_type: String,
    pub block_index: i64,
    pub start_offset: i64,
    pub end_offset: i64,
    pub prefix: String,
    pub suffix: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentComment {
    #[serde(default, skip_serializing_if = "is_false")]
    pub resolved: bool,
    pub id: String,
    pub parent_id: Option<String>,
    pub target_type: String,
    pub author: String,
    pub content: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<CommentAnchor>,
    #[serde(default)]
    pub attachments: Vec<CommentAttachment>,
}
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentFile {
    pub version: u32,
    pub source_path: String,
    pub updated_at: String,
    pub comments: Vec<DocumentComment>,
}

impl CommentFile {
    /// 空批注文件。`updatedAt` 取 Unix 纪元，对齐 TS 的 `new Date(0).toISOString()`——
    /// 一个"从没写过"的时间戳，不会被误当成刚刚改过。
    pub fn empty(source_path: &str) -> Self {
        Self {
            version: 1,
            source_path: source_path.to_owned(),
            updated_at: jstime::from_millis(0).expect("纪元时间必然可格式化"),
            comments: Vec::new(),
        }
    }
}

/// 批注文件名净化：Windows 非法字符与控制字符换成 `-`，连续 `-` 折叠成一个。
/// 净化后为空时退回 `document`。对齐 TS 的 `sanitizeCommentFileName`。
pub fn sanitize_comment_file_name(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_dash = false;
    for ch in value.chars() {
        let replaced = if "<>:\"/\\|?*".contains(ch) || (ch as u32) < 0x20 {
            '-'
        } else {
            ch
        };
        // 连续 `-` 折叠：注意原文里本来就有的 `-` 也参与折叠（TS 的 replace(/-+/g,'-') 同理）
        if replaced == '-' {
            if last_was_dash {
                continue;
            }
            last_was_dash = true;
        } else {
            last_was_dash = false;
        }
        out.push(replaced);
    }
    if out.is_empty() {
        "document".to_owned()
    } else {
        out
    }
}

/// `<dir>/.comment/<净化后的文件名>.json`。无目录部分时目录记作 `.`（对齐 TS）。
pub fn comment_sidecar_path(source_path: &str) -> String {
    let normalized = source_path.replace('\\', "/");
    match normalized.rfind('/') {
        Some(slash) => format!(
            "{}/.comment/{}.json",
            &normalized[..slash],
            sanitize_comment_file_name(&normalized[slash + 1..])
        ),
        None => format!(
            "./.comment/{}.json",
            sanitize_comment_file_name(&normalized)
        ),
    }
}

/// 附件目录：将独立数据文件的 `.json` 后缀改为 `.assets`（不区分大小写）。
pub fn comment_assets_dir(source_path: &str) -> String {
    let sidecar = comment_sidecar_path(source_path);
    match sidecar.len().checked_sub(5) {
        Some(cut) if sidecar[cut..].eq_ignore_ascii_case(".json") => {
            format!("{}.assets", &sidecar[..cut])
        }
        _ => sidecar,
    }
}

/// 读取批注。文件不存在、JSON 损坏、字段类型不对，一律返回空批注集——
/// 一个坏掉的伴生文件不该让文档打不开。
pub fn load_comments(source_path: &str) -> CommentFile {
    let empty = CommentFile::empty(source_path);
    let path = comment_sidecar_path(source_path);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return empty;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&text) else {
        return empty;
    };

    // 逐条解析而不是整体反序列化：一条坏批注不该带走其余的
    let comments = parsed
        .get("comments")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<DocumentComment>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    CommentFile {
        version: parsed.get("version").and_then(Value::as_u64).unwrap_or(1) as u32,
        // sourcePath 以调用方给的为准：文件被改名后，伴生文件里记的还是旧路径
        source_path: source_path.to_owned(),
        updated_at: parsed
            .get("updatedAt")
            .and_then(Value::as_str)
            .unwrap_or(&empty.updated_at)
            .to_owned(),
        comments,
    }
}

/// 写入批注。落盘格式是 `JSON.stringify(x, null, 2) + "\n"`——**结尾那个换行是契约的一部分**。
pub fn save_comments(
    source_path: &str,
    comments: Vec<DocumentComment>,
) -> anyhow::Result<CommentFile> {
    let file = CommentFile {
        version: 1,
        source_path: source_path.to_owned(),
        updated_at: jstime::now(),
        comments,
    };
    json2::write_to_file(
        comment_sidecar_path(source_path),
        &format!("{}\n", json2::serialize(&file)?),
    )?;
    Ok(file)
}

// ---------------------------------------------------------------------------
// PDF 标注
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfAnnotation {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub page: i64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl PdfAnnotation {
    /// 列表里显示的标签。对齐 TS 的 `getAnnotationLabel`。
    pub fn label(&self) -> String {
        if self.kind == "text" {
            if let Some(text) = self
                .text
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return text.to_owned();
            }
        }
        match self.kind.as_str() {
            "circle" => "圆圈标注".into(),
            "rect" => "矩形标注".into(),
            _ => "文字标注".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfAnnotationDocument {
    pub version: u32,
    pub source_path: String,
    pub annotations: Vec<PdfAnnotation>,
}

const DEFAULT_ANNOTATION_COLOR: &str = "#e11d48";

/// `<dir><sep>.annotations<sep><文件名>.annotations.json`。
/// **沿用原路径里的分隔符**——反斜杠路径要拼出反斜杠结果，否则两版看不到同一个文件。
pub fn annotation_sidecar_path(source_path: &str) -> String {
    let last_slash = source_path
        .rfind(['/', '\\'])
        .map(|i| (i, source_path.as_bytes()[i] as char));
    match last_slash {
        None => format!(".annotations/{source_path}.annotations.json"),
        Some((idx, sep)) => format!(
            "{}{sep}.annotations{sep}{}.annotations.json",
            &source_path[..idx],
            &source_path[idx + 1..]
        ),
    }
}

/// 旧版把伴生文件直接放在 PDF 旁边。
fn legacy_annotation_sidecar_path(source_path: &str) -> String {
    format!("{source_path}.annotations.json")
}

/// 归一化单条标注。类型不认识就整条丢弃；坐标钳到 [0,1]（页面归一化坐标）。
/// `now_millis` 由调用方给，便于测试。
fn normalize_annotation(raw: &Value, now_millis: i64) -> Option<PdfAnnotation> {
    let kind = raw.get("type").and_then(Value::as_str)?;
    if !matches!(kind, "circle" | "rect" | "text") {
        return None;
    }
    let num = |key: &str| raw.get(key).and_then(js_number);
    let clamp_unit = |v: Option<f64>| v.filter(|n| n.is_finite()).unwrap_or(0.0).clamp(0.0, 1.0);

    let page = num("page")
        .filter(|n| n.is_finite())
        .map(|n| n.floor() as i64)
        .filter(|n| *n != 0)
        .unwrap_or(1)
        .max(1);
    let created_at = num("createdAt")
        .filter(|n| n.is_finite() && *n != 0.0)
        .map(|n| n as i64)
        .unwrap_or(now_millis);
    let updated_at = num("updatedAt")
        .filter(|n| n.is_finite() && *n != 0.0)
        .map(|n| n as i64)
        .unwrap_or(created_at);

    Some(PdfAnnotation {
        // id 缺失时不能凭空造一个：那会让每次读取都得到不同的 id。
        // 缺失的注释 ID 在写入时生成；读取时保留空值，由调用方处理。
        id: raw
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or_default()
            .to_owned(),
        kind: kind.to_owned(),
        page,
        x: clamp_unit(num("x")),
        y: clamp_unit(num("y")),
        width: clamp_unit(num("width")),
        height: clamp_unit(num("height")),
        color: raw
            .get("color")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_ANNOTATION_COLOR)
            .to_owned(),
        text: raw.get("text").and_then(Value::as_str).map(str::to_owned),
        created_at,
        updated_at,
    })
}

/// 对齐 JS 的 `Number(x)`：数字直取，数字字符串也认。
fn js_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// 读取 PDF 标注。新路径优先，回落到旧版的同目录伴生文件。
///
/// **不在读取时做迁移**：TS 版读到旧文件会顺手改写并删除旧文件，那让一次只读操作
/// 变成了写操作（AI 的 `annotation_list` 会因此改动磁盘）。迁移交给显式的
/// [`migrate_legacy_annotations`]。
pub fn load_pdf_annotations(source_path: &str) -> PdfAnnotationDocument {
    let empty = PdfAnnotationDocument {
        version: 1,
        source_path: source_path.to_owned(),
        annotations: Vec::new(),
    };

    let sidecar = annotation_sidecar_path(source_path);
    let legacy = legacy_annotation_sidecar_path(source_path);
    let path = if Path::new(&sidecar).exists() {
        sidecar
    } else if Path::new(&legacy).exists() {
        legacy
    } else {
        return empty;
    };

    let Ok(text) = std::fs::read_to_string(&path) else {
        return empty;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&text) else {
        return empty;
    };

    let now = jstime::now_millis();
    let annotations = parsed
        .get("annotations")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| normalize_annotation(i, now))
                .collect()
        })
        .unwrap_or_default();

    PdfAnnotationDocument {
        annotations,
        ..empty
    }
}

pub fn save_pdf_annotations(
    source_path: &str,
    annotations: Vec<PdfAnnotation>,
) -> anyhow::Result<PdfAnnotationDocument> {
    let document = PdfAnnotationDocument {
        version: 1,
        source_path: source_path.to_owned(),
        annotations,
    };
    json2::write_to_file(
        annotation_sidecar_path(source_path),
        &format!("{}\n", json2::serialize(&document)?),
    )?;
    let _ = std::fs::remove_file(legacy_annotation_sidecar_path(source_path));
    Ok(document)
}

/// 把旧版同目录伴生文件搬到 `.annotations/` 下。没有旧文件时是空操作。
pub fn migrate_legacy_annotations(source_path: &str) -> anyhow::Result<bool> {
    let legacy = legacy_annotation_sidecar_path(source_path);
    if !Path::new(&legacy).exists() || Path::new(&annotation_sidecar_path(source_path)).exists() {
        return Ok(false);
    }
    let document = load_pdf_annotations(source_path);
    save_pdf_annotations(source_path, document.annotations)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    impl Temp {
        fn new(tag: &str) -> Self {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir()
                .join(format!("mochi-sidecar-{}-{tag}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        /// 工作区里某个文档的路径，正斜杠形式。
        fn doc(&self, name: &str) -> String {
            format!("{}/{name}", self.0.to_string_lossy().replace('\\', "/"))
        }
    }

    fn comment(id: &str, content: &str) -> DocumentComment {
        DocumentComment {
            resolved: false,
            id: id.into(),
            parent_id: None,
            target_type: "document".into(),
            author: "我".into(),
            content: content.into(),
            created_at: "2026-08-29T00:00:00.000Z".into(),
            updated_at: None,
            anchor: None,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn comment_sidecar_path_matches_the_ts_rule() {
        assert_eq!(
            comment_sidecar_path("D:/ws/知识库/笔记.md"),
            "D:/ws/知识库/.comment/笔记.md.json"
        );
        // 反斜杠输入统一成正斜杠——批注这边和 PDF 标注不一样
        assert_eq!(
            comment_sidecar_path("D:\\ws\\笔记.md"),
            "D:/ws/.comment/笔记.md.json"
        );
        assert_eq!(comment_sidecar_path("笔记.md"), "./.comment/笔记.md.json");
    }

    #[test]
    fn comment_file_names_are_sanitized_and_dashes_collapse() {
        assert_eq!(sanitize_comment_file_name("a:b?c.md"), "a-b-c.md");
        assert_eq!(
            sanitize_comment_file_name("a<>|b"),
            "a-b",
            "连续非法字符折叠成一个 -"
        );
        assert_eq!(
            sanitize_comment_file_name("a--b"),
            "a-b",
            "原有的连续 - 也要折叠"
        );
        assert_eq!(
            sanitize_comment_file_name("a\u{7}b"),
            "a-b",
            "控制字符也算非法"
        );
        assert_eq!(sanitize_comment_file_name("***"), "-");
        assert_eq!(sanitize_comment_file_name(""), "document");
        assert_eq!(sanitize_comment_file_name("正常文件名.md"), "正常文件名.md");
    }

    #[test]
    fn comment_assets_dir_swaps_the_json_suffix() {
        assert_eq!(
            comment_assets_dir("D:/ws/笔记.md"),
            "D:/ws/.comment/笔记.md.assets"
        );
    }

    #[test]
    fn missing_comment_file_reads_as_empty_at_the_epoch() {
        let t = Temp::new("no-comments");
        let file = load_comments(&t.doc("笔记.md"));
        assert!(file.comments.is_empty());
        assert_eq!(
            file.updated_at, "1970-01-01T00:00:00.000Z",
            "空批注的时间戳该是纪元"
        );
    }

    #[test]
    fn comments_round_trip_through_disk() {
        let t = Temp::new("roundtrip");
        let doc = t.doc("笔记.md");
        save_comments(&doc, vec![comment("c1", "第一条"), comment("c2", "第二条")]).unwrap();

        let loaded = load_comments(&doc);
        assert_eq!(loaded.comments.len(), 2);
        assert_eq!(loaded.comments[0].content, "第一条");
        assert_eq!(loaded.source_path, doc);
        assert_ne!(loaded.updated_at, "1970-01-01T00:00:00.000Z");
    }

    /// 落盘格式是 `JSON.stringify(x, null, 2) + "\n"`，结尾那个换行也是契约。
    #[test]
    fn comment_file_bytes_match_the_electron_format() {
        let t = Temp::new("bytes");
        let doc = t.doc("笔记.md");
        save_comments(&doc, vec![comment("c1", "中文内容")]).unwrap();

        let raw = std::fs::read_to_string(comment_sidecar_path(&doc)).unwrap();
        assert!(raw.ends_with("}\n"), "结尾必须有换行");
        assert!(!raw.contains("\r\n"), "必须是 LF");
        assert!(raw.contains("\"中文内容\""), "中文不该被转义");
        assert!(raw.contains("\n  \"version\": 1"), "2 空格缩进");
        assert!(
            raw.contains("\"parentId\": null"),
            "parentId 为 null 时保留该字段"
        );
        assert!(
            !raw.contains("\"updatedAt\": null"),
            "可选字段为空时整个键省略"
        );
    }

    /// 一条坏批注不该带走其余的。
    #[test]
    fn a_broken_comment_entry_does_not_lose_the_others() {
        let t = Temp::new("partial");
        let doc = t.doc("笔记.md");
        let sidecar = comment_sidecar_path(&doc);
        json2::write_to_file(
            &sidecar,
            &json!({
                "version": 1,
                "sourcePath": doc,
                "updatedAt": "2026-08-29T00:00:00.000Z",
                "comments": [
                    { "缺了所有必填字段": true },
                    { "id": "c2", "parentId": null, "targetType": "document",
                      "author": "我", "content": "好的那条", "createdAt": "2026-08-29T00:00:00.000Z",
                      "attachments": [] },
                ],
            })
            .to_string(),
        )
        .unwrap();

        let loaded = load_comments(&doc);
        assert_eq!(loaded.comments.len(), 1);
        assert_eq!(loaded.comments[0].id, "c2");
    }

    #[test]
    fn corrupt_comment_json_reads_as_empty_instead_of_failing() {
        let t = Temp::new("corrupt");
        let doc = t.doc("笔记.md");
        json2::write_to_file(comment_sidecar_path(&doc), "{ 这不是 JSON").unwrap();
        assert!(load_comments(&doc).comments.is_empty());
    }

    /// 文档被改名后，伴生文件里记的还是旧路径——以调用方给的为准。
    #[test]
    fn loaded_source_path_follows_the_caller_not_the_file() {
        let t = Temp::new("renamed");
        let doc = t.doc("笔记.md");
        save_comments(&doc, vec![comment("c1", "x")]).unwrap();

        let moved = t.doc("新名字.md");
        std::fs::rename(comment_sidecar_path(&doc), comment_sidecar_path(&moved)).unwrap();
        assert_eq!(load_comments(&moved).source_path, moved);
    }

    // --- PDF 标注 ---

    /// PDF 标注**沿用原路径的分隔符**，和批注那边的规则不同。
    #[test]
    fn annotation_sidecar_keeps_the_original_separator() {
        assert_eq!(
            annotation_sidecar_path("D:/ws/论文.pdf"),
            "D:/ws/.annotations/论文.pdf.annotations.json"
        );
        assert_eq!(
            annotation_sidecar_path("D:\\ws\\论文.pdf"),
            "D:\\ws\\.annotations\\论文.pdf.annotations.json"
        );
        assert_eq!(
            annotation_sidecar_path("论文.pdf"),
            ".annotations/论文.pdf.annotations.json"
        );
    }

    fn annotation(id: &str, kind: &str) -> PdfAnnotation {
        PdfAnnotation {
            id: id.into(),
            kind: kind.into(),
            page: 3,
            x: 0.1,
            y: 0.2,
            width: 0.3,
            height: 0.4,
            color: "#e11d48".into(),
            text: None,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn annotations_round_trip_through_disk() {
        let t = Temp::new("ann-roundtrip");
        let pdf = t.doc("论文.pdf");
        save_pdf_annotations(&pdf, vec![annotation("a1", "rect")]).unwrap();

        let loaded = load_pdf_annotations(&pdf);
        assert_eq!(loaded.annotations.len(), 1);
        assert_eq!(loaded.annotations[0].page, 3);
        assert_eq!(loaded.annotations[0].x, 0.1);

        let raw = std::fs::read_to_string(annotation_sidecar_path(&pdf)).unwrap();
        assert!(raw.ends_with("}\n") && !raw.contains("\r\n"));
    }

    #[test]
    fn missing_annotation_file_reads_as_empty() {
        let t = Temp::new("ann-missing");
        assert!(load_pdf_annotations(&t.doc("论文.pdf"))
            .annotations
            .is_empty());
    }

    #[test]
    fn unknown_annotation_types_are_dropped_and_coordinates_clamped() {
        let t = Temp::new("ann-normalize");
        let pdf = t.doc("论文.pdf");
        json2::write_to_file(
            annotation_sidecar_path(&pdf),
            &json!({
                "version": 1,
                "sourcePath": pdf,
                "annotations": [
                    { "id": "bad", "type": "涂鸦", "page": 1 },
                    { "id": "clamp", "type": "rect", "page": -5,
                      "x": -1, "y": 9, "width": "0.5", "height": null,
                      "createdAt": 1000 },
                ],
            })
            .to_string(),
        )
        .unwrap();

        let loaded = load_pdf_annotations(&pdf);
        assert_eq!(loaded.annotations.len(), 1, "不认识的类型整条丢弃");

        let a = &loaded.annotations[0];
        assert_eq!(a.page, 1, "页码至少是 1");
        assert_eq!((a.x, a.y), (0.0, 1.0), "坐标钳到 [0,1]");
        assert_eq!(a.width, 0.5, "数字字符串也认");
        assert_eq!(a.height, 0.0, "缺失/非法当 0");
        assert_eq!(a.color, "#e11d48", "缺颜色用默认色");
        assert_eq!(a.updated_at, 1000, "缺 updatedAt 时跟随 createdAt");
    }

    #[test]
    fn annotation_labels_prefer_text_then_fall_back_by_type() {
        assert_eq!(
            PdfAnnotation {
                text: Some("  批注内容 ".into()),
                ..annotation("a", "text")
            }
            .label(),
            "批注内容"
        );
        assert_eq!(
            PdfAnnotation {
                text: Some("   ".into()),
                ..annotation("a", "text")
            }
            .label(),
            "文字标注",
            "空白文字要退回类型标签"
        );
        assert_eq!(annotation("a", "circle").label(), "圆圈标注");
        assert_eq!(annotation("a", "rect").label(), "矩形标注");
    }

    /// 旧版把伴生文件放在 PDF 旁边，读取时要认，但**不该顺手改写磁盘**。
    #[test]
    fn legacy_annotation_file_is_read_without_being_rewritten() {
        let t = Temp::new("ann-legacy");
        let pdf = t.doc("论文.pdf");
        let legacy = format!("{pdf}.annotations.json");
        json2::write_to_file(
            &legacy,
            &json!({ "version": 1, "sourcePath": pdf,
                     "annotations": [{ "id": "a1", "type": "circle", "page": 2 }] })
            .to_string(),
        )
        .unwrap();

        let loaded = load_pdf_annotations(&pdf);
        assert_eq!(loaded.annotations.len(), 1);
        assert!(Path::new(&legacy).exists(), "只读操作不该删旧文件");
        assert!(
            !Path::new(&annotation_sidecar_path(&pdf)).exists(),
            "只读操作不该建新文件"
        );

        assert!(
            migrate_legacy_annotations(&pdf).unwrap(),
            "显式迁移才动磁盘"
        );
        assert!(Path::new(&annotation_sidecar_path(&pdf)).exists());
        assert!(!Path::new(&legacy).exists());
        assert!(!migrate_legacy_annotations(&pdf).unwrap(), "迁移是幂等的");
    }

    #[test]
    fn saving_annotations_removes_the_legacy_file() {
        let t = Temp::new("ann-save-legacy");
        let pdf = t.doc("论文.pdf");
        let legacy = format!("{pdf}.annotations.json");
        json2::write_to_file(&legacy, "{}").unwrap();

        save_pdf_annotations(&pdf, vec![annotation("a1", "rect")]).unwrap();
        assert!(!Path::new(&legacy).exists());
    }
}
