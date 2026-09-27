//! 绘制、点击和滚动共用 MainContent::resolve，避免内容路由分叉。

mod content;
mod document_decorations;
mod document_geometry;
mod document_input;
mod document_layout;
mod document_paint;
mod home_pane;
mod keyboard;
mod progressive_loading;
mod source_pane;

pub use content::{placeholder, placeholder_text, MainContent, TabFacts};
pub use home_pane::HomePane;
use keyboard::content_hash;
pub use keyboard::KeyOutcome;
pub use source_pane::SourcePane;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use mochi_core::analytics::HomeAnalytics;

use crate::ui::chrome::{ChromeState, WorkspaceView};
use crate::ui::draw::{DrawList, TextStyle};
use crate::ui::editor::{self, TextBuffer};
use crate::ui::layout::Rect;
use crate::ui::theme::{self, Palette};
use crate::ui::{document, home, live};
mod document_chunks;
#[cfg(test)]
mod home_tests;

#[cfg(test)]
mod locator_tests;

// ---------- 首页 ----------

// ---------- 渲染文档 ----------

/// 已渲染且可编辑的文档；富文本编辑事务会保留原始源码。
///
/// 排版按 (标签, 宽度, 内容指纹, 光标所在块) 缓存——遍历整篇逐字断行，几千行的笔记
/// 每帧重排会直接卡住。光标在同一个块里左右移动不重排；跨块、编辑、改宽度才重排。
#[derive(Default)]
pub struct DocPane {
    chunk_plan: Option<crate::ui::large_document::Plan>,
    chunk_index: usize,
    chunk_positions: HashMap<PathBuf, usize>,
    full_documents: std::collections::HashSet<PathBuf>,
    layout_blocks: std::ops::Range<usize>,
    title_hidden: bool,
    table_offsets: HashMap<PathBuf, HashMap<usize, f32>>,
    table_widths: HashMap<PathBuf, HashMap<usize, Vec<f32>>>,
    /// 仅在“横向滚动条”模式下使用的代码块滚动偏移。
    code_offsets: HashMap<PathBuf, HashMap<usize, f32>>,
    tail_height: f32,
    code_document: PathBuf,
    code_state: crate::ui::code_blocks::State,
    live: live::LiveLayout,
    /// 每个块各自缓存换行结果。调用 `invalidate()` 后仍会保留缓存，
    /// 因此重组文档时，输入一个块的内容仍可复用其他未变段落的结果。
    layout_cache: document::LayoutCache,
    mapping_cache: live::MappingCache,
    key: Option<(usize, u32, usize, (bool, u64), Option<usize>)>,
    progressive: bool,
    complete: bool,
    loading: Option<document::LayoutProgress>,
    loading_caret: Option<usize>,
    background_headers: Vec<usize>,
    /// 上下移动光标时保持不变的目标横坐标（相对内容区左缘）。
    desired_x: f32,
    /// 当前文档所在目录：相对路径的图片按它解析。
    base_dir: Option<PathBuf>,
    /// 图片尺寸缓存（按解析后的路径）。`None` 表示读过但读不出来，别每帧重试。
    image_sizes: HashMap<String, Option<(u32, u32)>>,
}

// ---------- 源码编辑面 ----------

#[cfg(test)]
mod tests;

#[cfg(test)]
mod object_link_tests;
