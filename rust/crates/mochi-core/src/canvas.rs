//! 无限画布：对象引用、独立文字和矢量笔迹。引用不复制原对象内容。

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::object_reference::{ObjectKind, ObjectReference};

pub const EXTENSION: &str = "mcanvas";
pub const VERSION: u32 = 2;
pub const MIN_ZOOM: f64 = 0.01;
pub const MAX_ZOOM: f64 = 4.0;

pub mod drawing;
mod freehand;
pub use freehand::{Bounds, Point, Stroke, TextNote};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasDocument {
    pub version: u32,
    #[serde(default)]
    pub viewport: Viewport,
    #[serde(default)]
    pub cards: Vec<Card>,
    #[serde(default)]
    pub texts: Vec<TextNote>,
    #[serde(default)]
    pub strokes: Vec<Stroke>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default = "default_zoom")]
    pub zoom: f64,
}

fn default_zoom() -> f64 {
    1.0
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    /// 可选的 RGB 强调色；老画布沿用主题底色。
    #[serde(default)]
    pub color: Option<u32>,
    pub id: String,
    pub x: f64,
    pub y: f64,
    #[serde(default = "default_width")]
    pub width: f64,
    #[serde(default = "default_height")]
    pub height: f64,
    pub target: Target,
}

fn default_width() -> f64 {
    280.0
}
fn default_height() -> f64 {
    156.0
}

/// 普通目标是一条完整的持久化 Mochi URL。PDF 批注单独一个形态：
/// PDF 批注保存在单独的批注数据文件中，不属于 PDF 本身的字节内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Target {
    Mochi { url: String },
    PdfAnnotation { path: String, annotation_id: String },
}

impl CanvasDocument {
    pub fn empty() -> Self {
        Self {
            version: VERSION,
            viewport: Viewport::default(),
            cards: Vec::new(),
            texts: Vec::new(),
            strokes: Vec::new(),
        }
    }

    pub fn add_mochi_reference(&mut self, url: impl Into<String>) -> Result<()> {
        let url = url.into();
        let reference = ObjectReference::parse(&url).context("不是有效的 Mochi 对象引用")?;
        if reference.kind == ObjectKind::PdfAnnotation {
            return self.add_pdf_annotation(
                reference.path.unwrap_or_default(),
                reference.item_id.unwrap_or_default(),
            );
        }
        if !matches!(
            reference.kind,
            ObjectKind::Document
                | ObjectKind::Block
                | ObjectKind::Record
                | ObjectKind::Task
                | ObjectKind::Event
                | ObjectKind::Project
        ) {
            bail!("该对象暂不支持放入画布");
        }
        let index = self.cards.len();
        self.cards.push(Card {
            color: None,
            id: self.next_id("card"),
            x: 48.0 + (index % 3) as f64 * 304.0,
            y: 48.0 + (index / 3) as f64 * 180.0,
            width: default_width(),
            height: default_height(),
            target: Target::Mochi { url },
        });
        Ok(())
    }

    pub fn add_pdf_annotation(
        &mut self,
        path: impl Into<String>,
        annotation_id: impl Into<String>,
    ) -> Result<()> {
        let path = path.into().replace('\\', "/");
        let annotation_id = annotation_id.into();
        if !valid_pdf_path(&path) || !valid_id(&annotation_id) {
            bail!("PDF 批注引用无效");
        }
        let index = self.cards.len();
        self.cards.push(Card {
            color: None,
            id: self.next_id("card"),
            x: 48.0 + (index % 3) as f64 * 304.0,
            y: 48.0 + (index / 3) as f64 * 180.0,
            width: default_width(),
            height: default_height(),
            target: Target::PdfAnnotation {
                path,
                annotation_id,
            },
        });
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if !matches!(self.version, 1 | VERSION) {
            bail!("不支持的画布版本：{}", self.version);
        }
        if !self.viewport.x.is_finite()
            || !self.viewport.y.is_finite()
            || !self.viewport.zoom.is_finite()
            || !(MIN_ZOOM..=MAX_ZOOM).contains(&self.viewport.zoom)
        {
            bail!("画布视口无效");
        }
        self.validate_freehand()?;
        for card in &self.cards {
            if card.color.is_some_and(|c| c > 0xffffff)
                || !valid_id(&card.id)
                || !card.x.is_finite()
                || !card.y.is_finite()
                || !card.width.is_finite()
                || !card.height.is_finite()
                || !(80.0..=2000.0).contains(&card.width)
                || !(56.0..=2000.0).contains(&card.height)
            {
                bail!("画布卡片格式无效");
            }
            match &card.target {
                Target::Mochi { url } => {
                    self.validate_reference(url)?;
                }
                Target::PdfAnnotation {
                    path,
                    annotation_id,
                } if valid_pdf_path(path) && valid_id(annotation_id) => {}
                Target::PdfAnnotation { .. } => bail!("PDF 批注引用无效"),
            }
        }
        Ok(())
    }

    fn validate_reference(&self, url: &str) -> Result<()> {
        let reference = ObjectReference::parse(url).context("画布含有无效 Mochi 引用")?;
        if matches!(
            reference.kind,
            ObjectKind::Document
                | ObjectKind::Block
                | ObjectKind::Record
                | ObjectKind::Task
                | ObjectKind::Event
                | ObjectKind::Project
                | ObjectKind::PdfAnnotation
        ) {
            Ok(())
        } else {
            bail!("画布引用的对象类型不受支持")
        }
    }
}

pub fn parse(raw: &str) -> Result<CanvasDocument> {
    let value: serde_json::Value = serde_json::from_str(raw).context("画布 JSON 格式错误")?;
    // 工作流画布与引用画布共用扩展名，但工作流的节点绝不能被
    // 引用画布那个默认为空的 cards 字段悄悄吞掉。
    if value.get("format").and_then(|format| format.as_str()) == Some("mochi.workflow-canvas")
        || value.get("nodes").is_some()
        || value.get("edges").is_some()
    {
        bail!("这是 Electron 工作流画布，请在 Electron 版打开；不能作为普通引用画布保存");
    }
    let mut document: CanvasDocument =
        serde_json::from_value(value).context("画布 JSON 格式错误")?;
    document.validate()?;
    document.version = VERSION;
    Ok(document)
}

pub fn serialize(document: &CanvasDocument) -> Result<String> {
    document.validate()?;
    let mut document = document.clone();
    document.version = VERSION;
    Ok(format!("{}\n", serde_json::to_string_pretty(&document)?))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}
fn valid_path(value: &str) -> bool {
    !value.trim().is_empty() && !value.split('/').any(|part| part == "..")
}
fn valid_pdf_path(value: &str) -> bool {
    valid_path(value) && value.to_ascii_lowercase().ends_with(".pdf")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canvas_persists_links_not_copied_content() {
        let mut canvas = CanvasDocument::empty();
        canvas
            .add_mochi_reference("mochi://open?path=notes%2Fa.md&kind=file&label=A")
            .unwrap();
        canvas
            .add_pdf_annotation("papers/a.pdf", "annotation_1")
            .unwrap();
        let raw = serialize(&canvas).unwrap();
        assert!(raw.contains("mochi://open"));
        assert!(!raw.contains("content"));
        assert_eq!(parse(&raw).unwrap(), canvas);
    }
    #[test]
    fn canvas_rejects_copied_or_unsafe_targets() {
        assert!(parse(r#"{"version":1,"cards":[{"id":"x","x":0,"y":0,"width":280,"height":156,"target":{"type":"mochi","url":"copied text"}}]}"#).is_err());
        assert!(parse(r#"{"version":1,"cards":[{"id":"x","x":0,"y":0,"width":280,"height":156,"target":{"type":"pdfAnnotation","path":"../a.pdf","annotationId":"a"}}]}"#).is_err());
    }
    #[test]
    fn canvas_rejects_workflows_instead_of_discarding_nodes() {
        for raw in [
            r#"{"format":"mochi.workflow-canvas","version":1,"nodes":[],"edges":[]}"#,
            r#"{"version":1,"nodes":[{"id":"start"}],"edges":[]}"#,
        ] {
            assert!(parse(raw).unwrap_err().to_string().contains("工作流"));
        }
        assert!(parse(r#"{"version":1,"cards":[]}"#).is_ok());
    }
}
