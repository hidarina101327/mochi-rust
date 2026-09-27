//! 识别可查看文件类型，并维护图片预览状态和展示信息。
use std::path::Path;

use super::draw::{Align, DrawList, TextStyle};
use super::highlight;
use super::icons::Icon;
use super::layout::Rect;
use super::text::{self, Emphasis};
use super::theme::{self, Palette};

/// 按文件名判断用哪个查看器。`Text` 交给编辑器。与 `FileViewer.detectFileType` 同一张表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Base,
    Canvas,
    Exam,
    Text,
    Image,
    Pdf,
    Document,
    Spreadsheet,
    Presentation,
    LegacyPresentation,
    Code(String),
    Html,
    Link,
    Unsupported,
}

pub fn detect(path: &Path) -> Kind {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.ends_with(".link.json") {
        return Kind::Link;
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "mcb" => Kind::Base,
        "mcanvas" => Kind::Canvas,
        "exam" => Kind::Exam,
        "md" | "markdown" | "mc" | "txt" => Kind::Text,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" | "ico" => Kind::Image,
        "pdf" => Kind::Pdf,
        "doc" | "docx" => Kind::Document,
        "xls" | "xlsx" => Kind::Spreadsheet,
        "pptx" => Kind::Presentation,
        "ppt" => Kind::LegacyPresentation,
        "html" | "htm" => Kind::Html,
        "js" | "jsx" | "ts" | "tsx" | "py" | "rb" | "java" | "c" | "cpp" | "cs" | "go" | "rs"
        | "php" | "css" | "scss" | "json" | "xml" | "yaml" | "yml" | "sh" | "bash" | "sql"
        | "swift" | "kt" | "r" => Kind::Code(code_language(&ext)),
        _ => Kind::Unsupported,
    }
}

/// `CodeViewer.getLanguageFromExtension`。
pub fn code_language(ext: &str) -> String {
    match ext {
        "js" | "jsx" => "javascript",
        "ts" | "tsx" => "typescript",
        "py" => "python",
        "rb" => "ruby",
        "java" => "java",
        "c" => "c",
        "cpp" => "cpp",
        "cs" => "csharp",
        "go" => "go",
        "rs" => "rust",
        "php" => "php",
        "css" => "css",
        "scss" => "scss",
        "json" => "json",
        "xml" => "xml",
        "yaml" | "yml" => "yaml",
        "md" => "markdown",
        "sh" | "bash" => "bash",
        "sql" => "sql",
        "swift" => "swift",
        "kt" => "kotlin",
        "r" => "r",
        _ => "plaintext",
    }
    .to_owned()
}

/// `formatFileSize`：B / KB / MB，保留两位。
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ---------- 状态 ----------

#[derive(Debug, Clone, PartialEq)]
pub struct ImageState {
    pub scale: f32,
    /// 0 / 90 / 180 / 270。
    pub rotation: u16,
    pub offset: (f32, f32),
    pub natural: Option<(u32, u32)>,
    pub file_size: u64,
    pub dragging: Option<(f32, f32)>,
}

impl ImageState {
    pub fn new(natural: Option<(u32, u32)>, file_size: u64) -> Self {
        ImageState {
            scale: 1.0,
            rotation: 0,
            offset: (0.0, 0.0),
            natural,
            file_size,
            dragging: None,
        }
    }
    pub fn zoom_in(&mut self) {
        self.scale = (self.scale * 1.2).min(5.0);
    }
    pub fn zoom_out(&mut self) {
        self.scale = (self.scale / 1.2).max(0.1);
    }
    pub fn rotate(&mut self) {
        self.rotation = (self.rotation + 90) % 360;
    }
    pub fn reset(&mut self) {
        *self = ImageState {
            scale: 1.0,
            rotation: 0,
            offset: (0.0, 0.0),
            ..self.clone()
        };
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodeState {
    pub language: String,
    pub lines: Vec<String>,
    pub tokens: Vec<Vec<highlight::Span>>,
    pub scroll: f32,
    /// 「Copied!」提示的截止时刻（毫秒时间戳）；`None` 显示「Copy」。
    pub copied_until: Option<u64>,
}

impl CodeState {
    pub fn new(language: &str, content: &str) -> Self {
        let lines: Vec<String> = content
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
            .collect();
        let tokens = highlight::highlight(language, &lines);
        CodeState {
            language: language.to_owned(),
            lines,
            tokens,
            scroll: 0.0,
            copied_until: None,
        }
    }
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkState {
    pub content: String,
    pub scroll: f32,
    pub busy: bool,
    pub title: String,
    pub description: String,
    pub url: String,
    pub cached: bool,
    pub cached_at: Option<String>,
}

impl LinkState {
    /// `.link.json`：`{ url, title, description, favicon, cached, cachedAt }`。
    pub fn parse(json: &str, fallback_title: &str) -> Option<LinkState> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let url = v.get("url")?.as_str()?.to_owned();
        Some(LinkState {
            content: String::new(),
            scroll: 0.0,
            busy: false,
            title: v
                .get("title")
                .and_then(|t| t.as_str())
                .filter(|t| !t.is_empty())
                .unwrap_or(fallback_title)
                .to_owned(),
            description: v
                .get("description")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_owned(),
            url,
            cached: v.get("cached").and_then(|c| c.as_bool()).unwrap_or(false),
            cached_at: v
                .get("cachedAt")
                .and_then(|c| c.as_str())
                .map(str::to_owned),
        })
    }
}

/// PDF：页面尺寸来自工作线程，位图按需渲染（`pdf.rs`）。
#[derive(Debug, Clone, PartialEq)]
pub struct PdfState {
    pub annotations: super::pdf_annotations::State,
    /// 每页的 DIP 尺寸（96 DPI）。为空表示还在加载。
    pub sizes: Vec<(f32, f32)>,
    /// 每页位图是否已渲染好（像素尺寸）。
    pub ready: Vec<Option<(u32, u32)>>,
    /// 已向工作线程要过的页（避免每帧重发）。
    pub requested: Vec<bool>,
    pub loading: bool,
    pub error: Option<String>,
    pub scroll: f32,
    pub zoom: f32,
    pub resume_page: Option<usize>,
    pub document_path: Option<std::path::PathBuf>,
    pub converting: bool,
}

impl PdfState {
    pub fn new() -> Self {
        PdfState {
            annotations: super::pdf_annotations::State::default(),
            sizes: Vec::new(),
            ready: Vec::new(),
            requested: Vec::new(),
            loading: true,
            error: None,
            scroll: 0.0,
            zoom: 1.0,
            resume_page: None,
            document_path: None,
            converting: false,
        }
    }

    pub fn loaded(&mut self, sizes: Vec<(f32, f32)>) {
        let n = sizes.len();
        self.sizes = sizes;
        self.ready = vec![None; n];
        self.requested = vec![false; n];
        self.loading = false;
        self.error = None;
    }

    pub fn page_count(&self) -> usize {
        self.sizes.len()
    }
    pub fn zoom_by(&mut self, body: Rect, factor: f32) {
        let page = pdf_current_page(body, self).saturating_sub(1);
        self.zoom = (self.zoom * factor).clamp(0.5, 3.0);
        self.ready.fill(None);
        self.requested.fill(false);
        if let Some(r) = pdf_page_rects(body, self).get(page) {
            self.scroll = (r.top - body.top - PDF_LABEL_H).max(0.0);
        }
    }

    /// 位图键：`mem://pdf/<路径>#<页>`。
    pub fn page_key(path: &Path, index: usize) -> String {
        format!("mem://pdf/{}#{index}", path.to_string_lossy())
    }
}

impl Default for PdfState {
    fn default() -> Self {
        Self::new()
    }
}

/// 一个打开着的查看器标签的内容。
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Base(super::base_view::State),
    Canvas(super::canvas_view::State),
    Exam(super::exam_view::State),
    Spreadsheet(super::sheet_view::State),
    Image(ImageState),
    Code(CodeState),
    Link(LinkState),
    Pdf(PdfState),
    /// 不支持 / 读不出来：`message` 为 `Some` 时是错误页。
    Unsupported {
        message: Option<String>,
    },
}

/// 读文件并建好内容。读盘只在这里发生一次。
pub fn load(path: &Path, kind: &Kind) -> Content {
    match kind {
        Kind::Base => match std::fs::read_to_string(path)
            .map_err(anyhow::Error::from)
            .and_then(super::base_view::State::parse)
        {
            Ok(state) => Content::Base(state),
            Err(error) => Content::Unsupported {
                message: Some(format!("无法打开多维表格：{error}")),
            },
        },
        Kind::Canvas => match std::fs::read_to_string(path)
            .map_err(anyhow::Error::from)
            .and_then(|raw| super::canvas_view::State::parse(&raw))
        {
            Ok(state) => Content::Canvas(state),
            Err(error) => Content::Unsupported {
                message: Some(format!("无法打开画布：{error}")),
            },
        },
        Kind::Exam => match std::fs::read_to_string(path) {
            Ok(raw) => Content::Exam(super::exam_view::State::new(raw)),
            Err(e) => Content::Unsupported {
                message: Some(e.to_string()),
            },
        },
        Kind::Image => {
            let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            Content::Image(ImageState::new(super::imginfo::dimensions(path), size))
        }
        Kind::Code(lang) => match std::fs::read_to_string(path) {
            Ok(s) => Content::Code(CodeState::new(lang, &s)),
            Err(e) => Content::Unsupported {
                message: Some(format!("Failed to load code: {e}")),
            },
        },
        Kind::Link => {
            let stem = path
                .file_name()
                .map(|n| n.to_string_lossy().replace(".link.json", ""))
                .unwrap_or_default();
            match std::fs::read_to_string(path)
                .ok()
                .and_then(|s| LinkState::parse(&s, &stem))
            {
                Some(l) => Content::Link(l),
                None => Content::Unsupported {
                    message: Some("Failed to load link".to_owned()),
                },
            }
        }
        Kind::Pdf => {
            let mut s = PdfState::new();
            s.annotations.items =
                mochi_core::sidecars::load_pdf_annotations(&path.to_string_lossy()).annotations;
            Content::Pdf(s)
        }
        Kind::Document | Kind::Presentation | Kind::LegacyPresentation => {
            Content::Pdf(PdfState::new())
        }
        Kind::Spreadsheet => Content::Spreadsheet(super::sheet_view::State {
            loading: true,
            ..Default::default()
        }),
        _ => Content::Unsupported { message: None },
    }
}

// ---------- PDF 页面几何 ----------

/// 页面列：`px-6 py-5`，页与页 `gap-4`，每页上方 `text-[11px]` 的「第 N 页」标签 + `gap-1`。
pub const PDF_PAD_X: f32 = 24.0;
pub const PDF_PAD_Y: f32 = 20.0;
pub const PDF_GAP: f32 = 16.0;
pub const PDF_LABEL_H: f32 = 16.0 + 4.0;
/// pdf.js 默认按 800 宽渲染（`pageState?.width ?? 800`）。
pub const PDF_DEFAULT_WIDTH: f32 = 800.0;
const PDF_TOOLBAR_H: f32 = 40.0 + 1.0;

/// 每一页在内容坐标（未滚动）里的矩形，按容器宽度与缩放算。
pub fn pdf_page_rects(body: Rect, s: &PdfState) -> Vec<Rect> {
    let avail = (body.width() - PDF_PAD_X * 2.0).max(100.0);
    let mut y = body.top + PDF_PAD_Y;
    let mut out = Vec::with_capacity(s.sizes.len());
    for (w, h) in &s.sizes {
        let display_w = PDF_DEFAULT_WIDTH.min(avail) * s.zoom;
        let display_h = (h / w.max(1.0)) * display_w;
        let left = (body.left + body.right) / 2.0 - display_w / 2.0;
        y += PDF_LABEL_H;
        out.push(Rect::new(left, y, left + display_w, y + display_h));
        y += display_h + PDF_GAP;
    }
    out
}

pub fn pdf_content_height(body: Rect, s: &PdfState) -> f32 {
    match pdf_page_rects(body, s).last() {
        Some(last) => last.bottom - body.top + PDF_PAD_Y,
        None => 0.0,
    }
}

/// 当前滚动位置下可见的页（含上下各一页的预取）。
pub fn pdf_visible_pages(body: Rect, s: &PdfState) -> Vec<usize> {
    let rects = pdf_page_rects(body, s);
    let mut visible: Vec<usize> = rects
        .iter()
        .enumerate()
        .filter(|(_, r)| r.bottom - s.scroll >= body.top && r.top - s.scroll <= body.bottom)
        .map(|(i, _)| i)
        .collect();
    if let (Some(&first), Some(&last)) = (visible.first(), visible.last()) {
        if first > 0 {
            visible.insert(0, first - 1);
        }
        if last + 1 < rects.len() {
            visible.push(last + 1);
        }
    }
    visible
}

/// 视口中央落在第几页（1 起）。工具栏的页码显示用。
pub fn pdf_current_page(body: Rect, s: &PdfState) -> usize {
    let mid = body.top + body.height() / 2.0 + s.scroll;
    let rects = pdf_page_rects(body, s);
    rects
        .iter()
        .position(|r| mid < r.bottom + PDF_GAP / 2.0)
        .map(|i| i + 1)
        .unwrap_or(rects.len())
}

// ---------- 排版与绘制 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Canvas(super::canvas_view::Hit),
    ZoomIn,
    ZoomOut,
    Rotate,
    Reset,
    /// 图片区：按下开始拖动。
    ImageCanvas,
    Copy,
    OpenExternal,
    OpenLink,
    RefreshCache,
    Body,
    /// PDF 工具栏的标注工具。
    PdfTool(&'static str),
    PdfReload,
    PdfShowInFolder,
    PdfOpenExternal,
    PdfPrev,
    PdfNext,
    PdfEditText,
    PdfPage,
    Sheet(super::sheet_view::Hit),
    Base(super::base_view::Hit),
    Exam(super::exam_view::Hit),
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub entries: Vec<(Rect, Hit)>,
    pub body: Rect,
    pub content_height: f32,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h)
    }
    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.body.height()).max(0.0)
    }
}

const TOOLBAR_H: f32 = 8.0 + 32.0 + 8.0 + 1.0;
const BUTTON: f32 = 32.0;
const CODE_LINE_H: f32 = 24.0;

pub fn layout(area: Rect, content: &Content) -> Layout {
    let mut lay = Layout::default();
    if area.is_empty() {
        return lay;
    }
    match content {
        Content::Base(s) => {
            let l = super::base_view::layout(s, area);
            lay.entries = l
                .entries
                .into_iter()
                .map(|(r, h)| (r, Hit::Base(h)))
                .collect();
            lay.body = l.body;
            lay.content_height = l.max_y + l.body.height();
        }
        Content::Canvas(s) => {
            let l = super::canvas_view::layout(s, area);
            lay.entries = l
                .entries
                .into_iter()
                .map(|(r, h)| (r, Hit::Canvas(h)))
                .collect();
            lay.body = l.body;
        }
        Content::Exam(s) => {
            let l = super::exam_view::layout(s, area);
            lay.entries = l
                .entries
                .into_iter()
                .map(|(r, h)| (r, Hit::Exam(h)))
                .collect();
            lay.body = l.body;
            lay.content_height = l.height;
        }
        Content::Spreadsheet(s) => {
            let l = super::sheet_view::layout(s, area);
            lay.entries = l
                .entries
                .into_iter()
                .map(|(r, h)| (r, Hit::Sheet(h)))
                .collect();
            lay.body = l.body;
            lay.content_height = l.max_y + l.body.height();
        }
        Content::Image(_) => {
            let top = area.top + 8.0;
            let mut x = area.left + 8.0;
            for hit in [Hit::ZoomIn, Hit::ZoomOut, Hit::Rotate, Hit::Reset] {
                lay.entries
                    .push((Rect::new(x, top, x + BUTTON, top + BUTTON), hit));
                x += BUTTON + 8.0;
            }
            lay.body = Rect::new(area.left, area.top + TOOLBAR_H, area.right, area.bottom);
            lay.entries.push((lay.body, Hit::ImageCanvas));
        }
        Content::Code(c) => {
            let copy_w = 16.0 + 8.0 + text::measure("Copied!", TextStyle::Label) + 24.0;
            let top = area.top + 8.0;
            lay.entries.push((
                Rect::new(
                    area.right - 8.0 - copy_w,
                    top,
                    area.right - 8.0,
                    top + BUTTON,
                ),
                Hit::Copy,
            ));
            lay.body = Rect::new(area.left, area.top + TOOLBAR_H, area.right, area.bottom);
            lay.entries.push((lay.body, Hit::Body));
            lay.content_height = 16.0 + c.lines.len() as f32 * CODE_LINE_H + 16.0;
        }
        Content::Link(l) => {
            // 卡片 `p-6`，内容 `max-w-3xl`（768）居中；按钮 `px-4 py-2` → 40 高
            let inner_w = 768.0f32.min(area.width() - 48.0);
            let left = area.left + (area.width() - inner_w) / 2.0;
            let mut y = area.top + 24.0;
            let title_height = TextStyle::Display.line_height().max(36.0);
            y += (title_height + 4.0).max(40.0); // 标题 + 标题后的留白
            if !l.description.is_empty() {
                y += 24.0 + 16.0;
            }
            y += 20.0 + 16.0; // url 行 + mb-4
            let text_left = left + 40.0 + 16.0;
            let open_w = 16.0 + 8.0 + text::measure("Open in Browser", TextStyle::Label) + 32.0;
            let cache_label = if l.cached {
                "Refresh Cache"
            } else {
                "Cache Content"
            };
            let cache_w = 16.0 + 8.0 + text::measure(cache_label, TextStyle::Label) + 32.0;
            lay.entries.push((
                Rect::new(text_left, y, text_left + open_w, y + 40.0),
                Hit::OpenLink,
            ));
            lay.entries.push((
                Rect::new(
                    text_left + open_w + 8.0,
                    y,
                    text_left + open_w + 8.0 + cache_w,
                    y + 40.0,
                ),
                Hit::RefreshCache,
            ));
            y += 40.0;
            if l.cached_at.is_some() {
                y += 8.0 + 16.0;
            }
            lay.body = Rect::new(area.left, y + 24.0 + 1.0, area.right, area.bottom);
            lay.entries.push((lay.body, Hit::Body));
            lay.content_height = 48.0
                + text::wrap_runs(
                    &[text::Run::plain(&l.content)],
                    TextStyle::Body,
                    lay.body.width().min(816.0) - 48.0,
                )
                .len() as f32
                    * TextStyle::Body.line_height();
        }
        Content::Pdf(s) => {
            // 窄主栏分成标题/页码与工具两行，所有命中区都留在主栏内。
            let compact = area.width() < 720.0;
            let count = 8 + usize::from(s.annotations.selected().is_some_and(|a| a.kind == "text"));
            let button = if compact {
                ((area.width() - 33.0 - count as f32 * 4.0) / count as f32).clamp(16.0, 32.0)
            } else {
                32.0
            };
            let top = area.top + 4.0 + if compact { 40.0 } else { 0.0 };
            let mut x = area.right - 12.0;
            for hit in [Hit::PdfOpenExternal, Hit::PdfShowInFolder, Hit::PdfReload] {
                lay.entries
                    .push((Rect::new(x - button, top, x, top + 32.0), hit));
                x -= button + 4.0;
            }
            x -= 4.0 + 1.0 + 4.0; // mx-1 分隔线
            if s.annotations.selected().is_some_and(|a| a.kind == "text") {
                lay.entries
                    .push((Rect::new(x - button, top, x, top + 32.0), Hit::PdfEditText));
                x -= button + 4.0;
            }
            for tool in [
                "删除选中标注",
                "文字标注",
                "矩形标注",
                "圆圈标注",
                "选择标注",
            ] {
                lay.entries.push((
                    Rect::new(x - button, top, x, top + 32.0),
                    Hit::PdfTool(tool),
                ));
                x -= button + 4.0;
            }
            // 页码：「‹ 3 / 12 ›」——Electron 是输入框 + 跳转；原生版先用前后翻页按钮
            x -= 8.0;
            if compact {
                x = area.right - 12.0;
            }
            let page_top = area.top + 6.0;
            lay.entries.push((
                Rect::new(x - 28.0, page_top, x, page_top + 28.0),
                Hit::PdfNext,
            ));
            lay.entries.push((
                Rect::new(x - 88.0, page_top, x - 28.0, page_top + 28.0),
                Hit::PdfPage,
            ));
            x -= 28.0 + 56.0 + 4.0;
            lay.entries.push((
                Rect::new(x - 28.0, page_top, x, page_top + 28.0),
                Hit::PdfPrev,
            ));
            lay.body = Rect::new(
                area.left,
                area.top + PDF_TOOLBAR_H + if compact { 40.0 } else { 0.0 },
                area.right,
                area.bottom,
            );
            lay.entries.push((lay.body, Hit::Body));
            lay.content_height = pdf_content_height(lay.body, s);
        }
        Content::Unsupported { message: None } => {
            // 居中的按钮：`px-4 py-2` 40 高
            let w = 16.0
                + 8.0
                + text::measure("Open with External Application", TextStyle::Label)
                + 32.0;
            let cx = (area.left + area.right) / 2.0;
            let cy = (area.top + area.bottom - 41.0) / 2.0;
            // 图标 80 + mb-4 + 标题 28 + mb-2 + 说明 24 + mb-6 + 按钮 40 = 212 → 从中心往上摆
            let block_top = cy - 106.0;
            lay.entries.push((
                Rect::new(
                    cx - w / 2.0,
                    block_top + 172.0,
                    cx + w / 2.0,
                    block_top + 212.0,
                ),
                Hit::OpenExternal,
            ));
            lay.body = Rect::new(area.left, area.top, area.right, area.bottom - 41.0);
        }
        Content::Unsupported { message: Some(_) } => {
            lay.body = area;
        }
    }
    lay
}

pub struct Model<'a> {
    pub content: &'a Content,
    pub path: &'a Path,
    /// 图片的解析后路径（与 `DrawCmd::Image` 的 src 一致）。
    pub image_src: &'a str,
    pub now_ms: u64,
    pub hover: Option<Hit>,
}

pub fn paint(list: &mut DrawList, area: Rect, lay: &Layout, m: &Model, p: &Palette) {
    if area.is_empty() {
        return;
    }
    list.push_clip(area);
    if !matches!(m.content, Content::Base(_)) {
        list.rect(area, p.background);
    }
    match m.content {
        Content::Base(s) => {
            let l = super::base_view::layout(s, area);
            super::base_view::paint(list, area, s, &l, p);
        }
        Content::Canvas(s) => {
            let l = super::canvas_view::layout(s, area);
            let hover = m.hover.and_then(|hit| match hit {
                Hit::Canvas(hit) => Some(hit),
                _ => None,
            });
            super::canvas_view::paint(list, area, s, &l, hover, p);
        }
        Content::Exam(s) => {
            let l = super::exam_view::layout(s, area);
            super::exam_view::paint(list, area, s, &l, p);
        }
        Content::Spreadsheet(s) => {
            let l = super::sheet_view::layout(s, area);
            let name = m.path.file_name().unwrap_or_default().to_string_lossy();
            super::sheet_view::paint(list, area, s, &l, &name, p);
        }
        Content::Image(s) => paint_image(list, area, lay, s, m, p),
        Content::Code(c) => paint_code(list, area, lay, c, m, p),
        Content::Link(l) => paint_link(list, area, lay, l, m, p),
        Content::Pdf(s) => paint_pdf(list, area, lay, s, m, p),
        Content::Unsupported { message: Some(msg) } => {
            let row = Rect::new(
                area.left,
                (area.top + area.bottom) / 2.0 - 12.0,
                area.right,
                (area.top + area.bottom) / 2.0 + 12.0,
            );
            let w = text::measure(msg, TextStyle::Body) + 28.0;
            let left = (area.left + area.right) / 2.0 - w / 2.0;
            list.icon_centered(
                Rect::new(left, row.top, left + 20.0, row.bottom),
                Icon::ALERT_CIRCLE,
                20.0,
                0xDC2626,
            );
            list.text(
                Rect::new(left + 28.0, row.top, left + w, row.bottom),
                msg.clone(),
                TextStyle::Body,
                0xDC2626,
            );
        }
        Content::Unsupported { message: None } => paint_unsupported(list, area, lay, m, p),
    }
    list.pop_clip();
}

fn toolbar_bg(list: &mut DrawList, area: Rect, p: &Palette) {
    let bar = Rect::new(area.left, area.top, area.right, area.top + TOOLBAR_H);
    list.rect(bar, p.surface);
    list.hline(bar.left, bar.right, bar.bottom - 1.0, p.border);
}

fn paint_image(
    list: &mut DrawList,
    area: Rect,
    lay: &Layout,
    s: &ImageState,
    m: &Model,
    p: &Palette,
) {
    toolbar_bg(list, area, p);
    for (r, hit) in &lay.entries {
        let icon = match hit {
            Hit::ZoomIn => Icon::ZOOM_IN,
            Hit::ZoomOut => Icon::ZOOM_OUT,
            Hit::Rotate => Icon::ROTATE_CW,
            Hit::Reset => Icon::MAXIMIZE2,
            _ => continue,
        };
        if m.hover == Some(*hit) {
            list.rounded_rect(*r, 4.0, theme::mix(p.accent, p.surface, 0.10));
        }
        list.icon_centered(*r, icon, 16.0, p.foreground);
    }
    // 缩放百分比：最后一个按钮之后 px-3
    let last_right = lay
        .entries
        .iter()
        .filter(|(_, h)| *h == Hit::Reset)
        .map(|(r, _)| r.right)
        .next()
        .unwrap_or(area.left);
    let bar_row = Rect::new(
        last_right + 12.0,
        area.top + 8.0,
        last_right + 120.0,
        area.top + 40.0,
    );
    list.text(
        bar_row,
        format!("{}%", (s.scale * 100.0).round() as i32),
        TextStyle::Label,
        p.muted,
    );
    // 右侧：尺寸与文件大小，gap-4
    if let Some((w, h)) = s.natural {
        let info = format!("{w} × {h}");
        let size = format_size(s.file_size);
        let size_w = text::measure(&size, TextStyle::Label);
        let info_w = text::measure(&info, TextStyle::Label);
        let right = area.right - 8.0;
        list.text(
            Rect::new(right - size_w, area.top + 8.0, right, area.top + 40.0),
            size,
            TextStyle::Label,
            p.muted,
        );
        list.text(
            Rect::new(
                right - size_w - 16.0 - info_w,
                area.top + 8.0,
                right - size_w - 16.0,
                area.top + 40.0,
            ),
            info,
            TextStyle::Label,
            p.muted,
        );
    }
    // 画布：`bg-muted/20`
    let body = lay.body;
    list.push_clip(body);
    list.rect(body, theme::mix(p.surface_muted, p.background, 0.2));
    if let Some((w, h)) = s.natural {
        // 初始按「适应」：大图缩到画布里（CSS 里 `max-w-full max-h-full object-contain`），再乘用户缩放
        let fit = (body.width() / w as f32)
            .min(body.height() / h as f32)
            .min(1.0);
        let (dw, dh) = (w as f32 * fit * s.scale, h as f32 * fit * s.scale);
        let cx = (body.left + body.right) / 2.0 + s.offset.0;
        let cy = (body.top + body.bottom) / 2.0 + s.offset.1;
        let rect = Rect::new(cx - dw / 2.0, cy - dh / 2.0, cx + dw / 2.0, cy + dh / 2.0);
        list.image_rotated(rect, m.image_src.to_owned(), String::new(), s.rotation);
    } else {
        list.text_aligned(
            body,
            "Failed to load image",
            TextStyle::Body,
            0xDC2626,
            Align::Center,
        );
    }
    list.pop_clip();
}

fn paint_code(
    list: &mut DrawList,
    area: Rect,
    lay: &Layout,
    c: &CodeState,
    m: &Model,
    p: &Palette,
) {
    toolbar_bg(list, area, p);
    let mut x = area.left + 8.0;
    let row = Rect::new(x, area.top + 8.0, area.right, area.top + 40.0);
    // 语言（首字母大写）、行数、只读标记，gap-4
    let lang = {
        let mut cs = c.language.chars();
        match cs.next() {
            Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
            None => String::new(),
        }
    };
    for label in [lang, format!("{} lines", c.lines.len())] {
        let w = text::measure(&label, TextStyle::Label);
        list.text(
            Rect::new(x, row.top, x + w, row.bottom),
            label,
            TextStyle::Label,
            p.muted,
        );
        x += w + 16.0;
    }
    // `bg-yellow-100 text-yellow-800`；暗色 `bg-yellow-900/30 text-yellow-200`
    let badge_w = text::measure("Read-only", TextStyle::Caption) + 16.0;
    let badge = Rect::new(x, row.top + 4.0, x + badge_w, row.bottom - 4.0);
    let (badge_bg, badge_fg) = if theme::is_dark(p) {
        (theme::mix(0x713F12, p.surface, 0.3), 0xFEF08A)
    } else {
        (0xFEF9C3, 0x854D0E)
    };
    list.rounded_rect(badge, 4.0, badge_bg);
    list.text_aligned(
        badge,
        "Read-only",
        TextStyle::Caption,
        badge_fg,
        Align::Center,
    );
    if let Some((r, _)) = lay.entries.iter().find(|(_, h)| *h == Hit::Copy) {
        if m.hover == Some(Hit::Copy) {
            list.rounded_rect(*r, 4.0, theme::mix(p.accent, p.surface, 0.10));
        }
        let copied = c.copied_until.map(|t| m.now_ms < t).unwrap_or(false);
        let (icon, label, color) = if copied {
            (Icon::CHECK, "Copied!", 0x16A34A)
        } else {
            (Icon::COPY, "Copy", p.foreground)
        };
        list.icon_centered(
            Rect::new(r.left + 12.0, r.top, r.left + 28.0, r.bottom),
            icon,
            16.0,
            color,
        );
        list.text(
            Rect::new(r.left + 36.0, r.top, r.right, r.bottom),
            label,
            TextStyle::Label,
            p.foreground,
        );
    }
    // 正文：行号列 + 代码
    let body = lay.body;
    list.push_clip(body);
    let digits = c.lines.len().max(1).to_string().len();
    let gutter_w = 16.0 + text::measure(&"8".repeat(digits), TextStyle::Mono) + 16.0;
    list.rect(
        Rect::new(body.left, body.top, body.left + gutter_w, body.bottom),
        theme::mix(p.surface_muted, p.background, 0.3),
    );
    let top = body.top + 16.0 - c.scroll;
    for (i, line) in c.lines.iter().enumerate() {
        let y = top + i as f32 * CODE_LINE_H;
        if y + CODE_LINE_H < body.top || y > body.bottom {
            continue;
        }
        list.text_aligned(
            Rect::new(
                body.left + 16.0,
                y,
                body.left + gutter_w - 16.0,
                y + CODE_LINE_H,
            ),
            (i + 1).to_string(),
            TextStyle::Mono,
            p.muted,
            Align::Trailing,
        );
        let mut cx = body.left + gutter_w + 16.0;
        let mut pos = 0usize;
        let mut segments: Vec<(usize, usize, Option<highlight::Token>)> = Vec::new();
        for s in c.tokens.get(i).map(Vec::as_slice).unwrap_or(&[]) {
            if s.start > pos {
                segments.push((pos, s.start, None));
            }
            segments.push((s.start, s.end.min(line.len()), Some(s.kind)));
            pos = s.end.min(line.len());
        }
        if pos < line.len() {
            segments.push((pos, line.len(), None));
        }
        let dark = theme::is_dark(p);
        for (a, b, kind) in segments {
            let Some(piece) = line.get(a..b) else {
                continue;
            };
            if piece.is_empty() {
                continue;
            }
            let w = text::measure(piece, TextStyle::Mono);
            let color = kind
                .map(|k| highlight::color(k, dark))
                .unwrap_or(p.foreground);
            let emphasis = if kind.map(highlight::italic).unwrap_or(false) {
                Emphasis::Italic
            } else {
                Emphasis::Code
            };
            list.text_run(
                Rect::new(cx, y, body.right, y + CODE_LINE_H),
                piece.to_owned(),
                TextStyle::Mono,
                color,
                Align::Leading,
                emphasis,
            );
            cx += w;
            if cx > body.right {
                break;
            }
        }
    }
    list.pop_clip();
}

fn paint_link(
    list: &mut DrawList,
    area: Rect,
    lay: &Layout,
    l: &LinkState,
    m: &Model,
    p: &Palette,
) {
    let inner_w = 768.0f32.min(area.width() - 48.0);
    let left = area.left + (area.width() - inner_w) / 2.0;
    let card_bottom = lay.body.top - 1.0;
    list.rect(
        Rect::new(area.left, area.top, area.right, card_bottom),
        p.surface,
    );
    list.hline(area.left, area.right, card_bottom, p.border);
    // favicon 位：40px 圆角底 + 地球图标
    let icon_box = Rect::new(
        left,
        area.top + 24.0 + 4.0,
        left + 40.0,
        area.top + 24.0 + 44.0,
    );
    list.rounded_rect(icon_box, 4.0, p.surface_muted);
    list.icon_centered(icon_box, Icon::GLOBE, 20.0, p.muted);
    let text_left = left + 56.0;
    let mut y = area.top + 24.0;
    let title_height = TextStyle::Display.line_height().max(36.0);
    list.text_run(
        Rect::new(text_left, y, left + inner_w, y + title_height),
        l.title.clone(),
        TextStyle::Display,
        p.foreground,
        Align::Leading,
        Emphasis::Bold,
    );
    y += (title_height + 4.0).max(40.0);
    if !l.description.is_empty() {
        list.text(
            Rect::new(text_left, y, left + inner_w, y + 24.0),
            l.description.clone(),
            TextStyle::Body16,
            p.muted,
        );
        y += 24.0 + 16.0;
    }
    list.text(
        Rect::new(text_left, y, left + inner_w, y + 20.0),
        l.url.clone(),
        TextStyle::Label,
        p.muted,
    );
    y += 20.0 + 16.0;
    for (r, hit) in &lay.entries {
        match hit {
            Hit::OpenLink => {
                list.glass_button(*r, 6.0, p, m.hover == Some(*hit));
                list.icon_centered(
                    Rect::new(r.left + 16.0, r.top, r.left + 32.0, r.bottom),
                    Icon::EXTERNAL_LINK,
                    16.0,
                    p.button_foreground(),
                );
                list.text(
                    Rect::new(r.left + 40.0, r.top, r.right, r.bottom),
                    "Open in Browser",
                    TextStyle::Label,
                    p.button_foreground(),
                );
            }
            Hit::RefreshCache => {
                if m.hover == Some(*hit) {
                    list.rounded_rect(*r, 6.0, theme::mix(p.accent, p.surface, 0.10));
                }
                list.rounded_border(*r, 6.0, p.border);
                list.icon_centered(
                    Rect::new(r.left + 16.0, r.top, r.left + 32.0, r.bottom),
                    Icon::REFRESH_CW,
                    16.0,
                    p.foreground,
                );
                let label = if l.busy {
                    "Caching..."
                } else if l.cached {
                    "Refresh Cache"
                } else {
                    "Cache Content"
                };
                list.text(
                    Rect::new(r.left + 40.0, r.top, r.right, r.bottom),
                    label,
                    TextStyle::Label,
                    p.foreground,
                );
            }
            _ => {}
        }
    }
    if l.cached {
        let after = lay
            .entries
            .iter()
            .filter(|(_, h)| *h == Hit::RefreshCache)
            .map(|(r, _)| r.right)
            .next()
            .unwrap_or(text_left);
        list.icon_centered(
            Rect::new(after + 8.0, y + 12.0, after + 24.0, y + 28.0),
            Icon::CHECK,
            16.0,
            0x16A34A,
        );
        list.text(
            Rect::new(after + 28.0, y + 10.0, after + 120.0, y + 30.0),
            "Cached",
            TextStyle::Label,
            p.muted,
        );
    }
    y += 40.0;
    if let Some(at) = &l.cached_at {
        list.text(
            Rect::new(text_left, y + 8.0, left + inner_w, y + 24.0),
            format!("Last cached: {at}"),
            TextStyle::Caption,
            p.muted,
        );
    }
    // 没有缓存内容：主体居中提示
    let body = lay.body;
    if !l.content.is_empty() {
        list.push_clip(body);
        let width = (body.width() - 48.0).clamp(40.0, 768.0);
        let left = body.left + (body.width() - width) / 2.0;
        let mut y = body.top + 24.0 - l.scroll;
        for runs in text::wrap_runs(&[text::Run::plain(&l.content)], TextStyle::Body, width) {
            let value = runs.iter().map(|r| r.text.as_str()).collect::<String>();
            list.text(
                Rect::new(left, y, left + width, y + TextStyle::Body.line_height()),
                value,
                TextStyle::Body,
                p.foreground,
            );
            y += TextStyle::Body.line_height();
        }
        list.pop_clip();
        return;
    }
    let msg = if l.cached {
        "Cached content is empty"
    } else {
        "No cached content. Click \"Cache Content\" to save a copy."
    };
    list.text_aligned(
        Rect::new(
            body.left,
            (body.top + body.bottom) / 2.0 - 12.0,
            body.right,
            (body.top + body.bottom) / 2.0 + 12.0,
        ),
        msg,
        TextStyle::Body,
        p.muted,
        Align::Center,
    );
}

fn paint_pdf(list: &mut DrawList, area: Rect, lay: &Layout, s: &PdfState, m: &Model, p: &Palette) {
    // 工具栏：文件名在左（FileText 图标 + text-sm），右侧按钮
    let bar = Rect::new(area.left, area.top, area.right, lay.body.top);
    list.rect(bar, p.surface);
    list.hline(bar.left, bar.right, bar.bottom - 1.0, p.border);
    list.icon_centered(
        Rect::new(bar.left + 12.0, bar.top, bar.left + 28.0, bar.top + 40.0),
        Icon::FILE_TEXT,
        16.0,
        p.muted,
    );
    let name = m
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name_right = lay
        .entries
        .iter()
        .filter(|(_, h)| *h == Hit::PdfPrev)
        .map(|(r, _)| r.left - 16.0)
        .next()
        .unwrap_or(bar.right - 200.0);
    list.push_clip(Rect::new(
        bar.left + 36.0,
        bar.top,
        name_right.max(bar.left + 36.0),
        bar.top + 40.0,
    ));
    list.text(
        Rect::new(
            bar.left + 36.0,
            bar.top,
            name_right.max(bar.left + 36.0),
            bar.top + 40.0,
        ),
        text::ellipsize(
            &name,
            TextStyle::Label,
            (name_right - bar.left - 36.0).max(0.0),
        ),
        TextStyle::Label,
        p.foreground,
    );
    list.pop_clip();
    for (r, hit) in &lay.entries {
        let icon = match hit {
            Hit::PdfOpenExternal => Icon::EXTERNAL_LINK,
            Hit::PdfShowInFolder => Icon::FOLDER_OPEN,
            Hit::PdfReload => Icon::REFRESH_CW,
            Hit::PdfTool("选择标注") => Icon::MOUSE_POINTER2,
            Hit::PdfTool("圆圈标注") => Icon::CIRCLE,
            Hit::PdfTool("矩形标注") => Icon::SQUARE,
            Hit::PdfTool("文字标注") => Icon::TYPE,
            Hit::PdfTool(_) => Icon::TRASH2,
            Hit::PdfEditText => Icon::PENCIL,
            Hit::PdfPrev => Icon::CHEVRON_LEFT,
            Hit::PdfNext => Icon::CHEVRON_RIGHT,
            _ => continue,
        };
        if m.hover == Some(*hit) {
            list.rounded_rect(*r, 4.0, p.background);
        }
        let active = match hit {
            Hit::PdfTool(label) => {
                super::pdf_annotations::Tool::from_label(label) == Some(s.annotations.tool)
            }
            _ => false,
        };
        if active {
            list.rounded_rect(*r, 4.0, theme::mix(p.accent, p.surface, 0.10));
        }
        let color = if *hit == Hit::PdfTool("删除选中标注") && s.annotations.selected.is_none()
        {
            theme::mix(p.muted, p.surface, 0.5)
        } else if active {
            p.accent
        } else {
            p.muted
        };
        list.icon_centered(*r, icon, 16.0, color);
    }
    // 分隔线（mx-1 h-5 border-l）
    if let Some((reload, _)) = lay.entries.iter().find(|(_, h)| *h == Hit::PdfReload) {
        list.rect(
            Rect::new(
                reload.left - 8.0 - 1.0,
                reload.top + 6.0,
                reload.left - 8.0,
                reload.top + 26.0,
            ),
            p.border,
        );
    }
    // 页码 `current / count`
    if let (Some((prev, _)), Some((next, _))) = (
        lay.entries.iter().find(|(_, h)| *h == Hit::PdfPrev),
        lay.entries.iter().find(|(_, h)| *h == Hit::PdfNext),
    ) {
        if s.page_count() > 0 {
            let label = format!("{} / {}", pdf_current_page(lay.body, s), s.page_count());
            list.text_aligned(
                Rect::new(prev.right, prev.top, next.left, prev.bottom),
                label,
                TextStyle::Caption,
                p.muted,
                Align::Center,
            );
        }
    }

    // 页面列
    let body = lay.body;
    list.push_clip(body);
    list.rect(body, p.background);
    if let Some(err) = &s.error {
        let card = Rect::new(
            (body.left + body.right) / 2.0 - 224.0,
            (body.top + body.bottom) / 2.0 - 40.0,
            (body.left + body.right) / 2.0 + 224.0,
            (body.top + body.bottom) / 2.0 + 40.0,
        );
        list.rounded_rect(card, 4.0, p.surface);
        list.rounded_border(card, 4.0, p.border);
        list.icon_centered(
            Rect::new(
                card.left + 16.0,
                card.top + 16.0,
                card.left + 36.0,
                card.top + 36.0,
            ),
            Icon::ALERT_CIRCLE,
            20.0,
            0xEF4444,
        );
        list.text_run(
            Rect::new(
                card.left + 48.0,
                card.top + 14.0,
                card.right - 16.0,
                card.top + 34.0,
            ),
            "PDF 加载失败",
            TextStyle::Label,
            p.foreground,
            Align::Leading,
            Emphasis::Bold,
        );
        list.text(
            Rect::new(
                card.left + 48.0,
                card.top + 38.0,
                card.right - 16.0,
                card.top + 58.0,
            ),
            err.clone(),
            TextStyle::Label,
            p.muted,
        );
    } else if s.loading {
        list.text_aligned(
            body,
            "正在加载 PDF…",
            TextStyle::Body,
            p.muted,
            Align::Center,
        );
    } else {
        for (i, r) in pdf_page_rects(body, s).into_iter().enumerate() {
            let page = Rect::new(r.left, r.top - s.scroll, r.right, r.bottom - s.scroll);
            if page.bottom < body.top || page.top > body.bottom {
                continue;
            }
            list.text_aligned(
                Rect::new(
                    page.left,
                    page.top - PDF_LABEL_H,
                    page.right,
                    page.top - 4.0,
                ),
                format!("第 {} 页", i + 1),
                TextStyle::Caption,
                p.muted,
                Align::Center,
            );
            // 纸面：白底 + 边框 + 一点阴影感
            list.rect(
                Rect::new(
                    page.left + 1.0,
                    page.top + 2.0,
                    page.right + 1.0,
                    page.bottom + 2.0,
                ),
                theme::mix(p.foreground, p.background, 0.06),
            );
            list.rect(page, 0xFFFFFF);
            list.rounded_border(page, 0.0, p.border);
            if s.ready.get(i).copied().flatten().is_some() {
                list.image_rotated(page, PdfState::page_key(m.path, i), String::new(), 0);
            } else {
                list.text_aligned(page, "渲染中…", TextStyle::Caption, p.muted, Align::Center);
            }
            if super::settings_values::boolean("editorLayout.annotationsVisible", true) {
                super::pdf_annotations::paint_page(list, page, i as i64 + 1, &s.annotations);
            }
        }
    }
    list.pop_clip();
}

fn paint_unsupported(list: &mut DrawList, area: Rect, lay: &Layout, m: &Model, p: &Palette) {
    let cx = (area.left + area.right) / 2.0;
    let cy = (area.top + area.bottom - 41.0) / 2.0;
    let top = cy - 106.0;
    list.icon_centered(
        Rect::new(cx - 40.0, top, cx + 40.0, top + 80.0),
        Icon::FILE,
        80.0,
        theme::mix(p.muted, p.background, 0.5),
    );
    list.text_aligned(
        Rect::new(area.left, top + 96.0, area.right, top + 124.0),
        "Unsupported File Type",
        TextStyle::Large,
        p.foreground,
        Align::Center,
    );
    list.text_aligned(
        Rect::new(area.left, top + 132.0, area.right, top + 156.0),
        "This file type cannot be previewed in Mochi",
        TextStyle::Body16,
        p.muted,
        Align::Center,
    );
    if let Some((r, _)) = lay.entries.iter().find(|(_, h)| *h == Hit::OpenExternal) {
        list.glass_button(*r, 6.0, p, m.hover == Some(Hit::OpenExternal));
        list.icon_centered(
            Rect::new(r.left + 16.0, r.top, r.left + 32.0, r.bottom),
            Icon::EXTERNAL_LINK,
            16.0,
            p.button_foreground(),
        );
        list.text(
            Rect::new(r.left + 40.0, r.top, r.right, r.bottom),
            "Open with External Application",
            TextStyle::Label,
            p.button_foreground(),
        );
    }
    // 底部路径条 `border-t p-2 text-xs bg-card`
    let footer = Rect::new(area.left, area.bottom - 41.0, area.right, area.bottom);
    list.rect(footer, p.surface);
    list.hline(footer.left, footer.right, footer.top, p.border);
    list.text(
        Rect::new(
            footer.left + 8.0,
            footer.top + 8.0,
            footer.right - 8.0,
            footer.bottom - 8.0,
        ),
        m.path.to_string_lossy().into_owned(),
        TextStyle::Caption,
        p.muted,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::DrawCmd;
    use std::path::PathBuf;

    fn texts(list: &DrawList) -> Vec<String> {
        list.cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn detection_follows_the_file_viewer_table() {
        assert_eq!(detect(Path::new("a/b.MD")), Kind::Text);
        assert_eq!(detect(Path::new("空间.mcanvas")), Kind::Canvas);
        assert_eq!(detect(Path::new("pic.PNG")), Kind::Image);
        assert_eq!(detect(Path::new("x.pdf")), Kind::Pdf);
        assert_eq!(
            detect(Path::new("site.link.json")),
            Kind::Link,
            "`.link.json` 先于扩展名判断"
        );
        assert_eq!(detect(Path::new("data.json")), Kind::Code("json".into()));
        assert_eq!(detect(Path::new("main.rs")), Kind::Code("rust".into()));
        assert_eq!(detect(Path::new("deck.pptx")), Kind::Presentation);
        assert_eq!(detect(Path::new("old.ppt")), Kind::LegacyPresentation);
        assert_eq!(detect(Path::new("index.htm")), Kind::Html);
        assert_eq!(detect(Path::new("weird.xyz")), Kind::Unsupported);
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.00 KB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.00 MB");
    }

    #[test]
    fn image_zoom_is_clamped_and_reset_restores_defaults() {
        let mut s = ImageState::new(Some((800, 600)), 1234);
        for _ in 0..20 {
            s.zoom_in();
        }
        assert_eq!(s.scale, 5.0);
        for _ in 0..40 {
            s.zoom_out();
        }
        assert!((s.scale - 0.1).abs() < 1e-6);
        s.rotate();
        s.rotate();
        s.rotate();
        s.rotate();
        assert_eq!(s.rotation, 0);
        s.rotate();
        s.offset = (30.0, -10.0);
        s.reset();
        assert_eq!((s.scale, s.rotation, s.offset), (1.0, 0, (0.0, 0.0)));
        assert_eq!(s.natural, Some((800, 600)), "尺寸信息不随重置丢失");
    }

    #[test]
    fn the_image_viewer_paints_toolbar_info_and_the_bitmap() {
        let content = Content::Image(ImageState::new(Some((400, 120)), 958));
        let area = Rect::new(400.0, 68.0, 1200.0, 800.0);
        let lay = layout(area, &content);
        assert_eq!(
            lay.entries
                .iter()
                .filter(|(_, h)| matches!(h, Hit::ZoomIn | Hit::ZoomOut | Hit::Rotate | Hit::Reset))
                .count(),
            4
        );
        let mut list = DrawList::new();
        let path = PathBuf::from("D:/ws/pic.png");
        let model = Model {
            content: &content,
            path: &path,
            image_src: "D:/ws/pic.png",
            now_ms: 0,
            hover: None,
        };
        paint(
            &mut list,
            area,
            &lay,
            &model,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        assert!(
            t.contains(&"100%".to_owned())
                && t.contains(&"400 × 120".to_owned())
                && t.contains(&"958 B".to_owned()),
            "{t:?}"
        );
        assert!(list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Image { src, .. } if src == "D:/ws/pic.png")));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn the_code_viewer_numbers_lines_and_colors_tokens() {
        let content = Content::Code(CodeState::new(
            "rust",
            "fn main() {\n    let x = 1; // hi\n}\n",
        ));
        let area = Rect::new(400.0, 68.0, 1200.0, 800.0);
        let lay = layout(area, &content);
        assert!(lay.entries.iter().any(|(_, h)| *h == Hit::Copy));
        let mut list = DrawList::new();
        let path = PathBuf::from("main.rs");
        let model = Model {
            content: &content,
            path: &path,
            image_src: "",
            now_ms: 0,
            hover: None,
        };
        paint(
            &mut list,
            area,
            &lay,
            &model,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        for expected in [
            "Rust",
            "4 lines",
            "Read-only",
            "Copy",
            "1",
            "2",
            "3",
            "fn",
            "// hi",
        ] {
            assert!(t.iter().any(|s| s == expected), "缺 {expected}: {t:?}");
        }
        assert!(list.cmds().iter().any(|c| matches!(c, DrawCmd::Text { color, .. } if *color == highlight::color(highlight::Token::Keyword, false))));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn link_files_parse_the_json_and_paint_the_card() {
        let l = LinkState::parse(r#"{"url":"https://a.b","title":"","description":"d","cached":true,"cachedAt":"2026-01-01T00:00:00Z"}"#, "fallback").unwrap();
        assert_eq!(l.title, "fallback", "空标题退回文件名");
        assert!(l.cached);
        assert!(
            LinkState::parse(r#"{"title":"x"}"#, "f").is_none(),
            "没有 url 不是链接文件"
        );
        let content = Content::Link(l);
        let area = Rect::new(400.0, 68.0, 1200.0, 800.0);
        let lay = layout(area, &content);
        let mut list = DrawList::new();
        let path = PathBuf::from("site.link.json");
        let model = Model {
            content: &content,
            path: &path,
            image_src: "",
            now_ms: 0,
            hover: None,
        };
        paint(
            &mut list,
            area,
            &lay,
            &model,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        for expected in [
            "fallback",
            "d",
            "https://a.b",
            "Open in Browser",
            "Refresh Cache",
            "Cached",
        ] {
            assert!(t.iter().any(|s| s == expected), "缺 {expected}: {t:?}");
        }
        assert!(list.finish().is_ok());
    }

    #[test]
    fn pdf_pages_stack_vertically_at_800_wide_and_visibility_prefetches_neighbours() {
        let mut s = PdfState::new();
        assert!(s.loading);
        s.loaded(vec![(612.0, 792.0); 5]);
        assert_eq!(s.page_count(), 5);
        let body = Rect::new(0.0, 41.0, 1000.0, 741.0);
        let rects = pdf_page_rects(body, &s);
        assert_eq!(rects[0].width(), 800.0);
        assert_eq!(rects[0].left, 100.0, "居中");
        assert!((rects[0].height() - 800.0 * 792.0 / 612.0).abs() < 0.01);
        assert_eq!(rects[1].top, rects[0].bottom + PDF_GAP + PDF_LABEL_H);
        assert!(pdf_content_height(body, &s) > 5.0 * 1000.0);
        assert_eq!(
            pdf_visible_pages(body, &s),
            vec![0, 1],
            "首屏只见第一页，预取下一页"
        );
        s.scroll = rects[2].top - body.top;
        assert_eq!(pdf_visible_pages(body, &s), vec![1, 2, 3]);
        assert_eq!(pdf_current_page(body, &s), 3);
        assert_eq!(
            PdfState::page_key(Path::new("D:/a.pdf"), 2),
            "mem://pdf/D:/a.pdf#2"
        );

        let content = Content::Pdf(s);
        let area = Rect::new(0.0, 0.0, 1000.0, 741.0);
        let lay = layout(area, &content);
        assert!(lay.entries.iter().any(|(_, h)| *h == Hit::PdfReload));
        assert_eq!(
            lay.entries
                .iter()
                .filter(|(_, h)| matches!(h, Hit::PdfTool(_)))
                .count(),
            5
        );
        let mut list = DrawList::new();
        let path = PathBuf::from("D:/a.pdf");
        let model = Model {
            content: &content,
            path: &path,
            image_src: "",
            now_ms: 0,
            hover: None,
        };
        paint(
            &mut list,
            area,
            &lay,
            &model,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        // 已滚到第三页：标签、页码与占位文字都是第三页的
        assert!(
            t.contains(&"a.pdf".to_owned())
                && t.contains(&"3 / 5".to_owned())
                && t.contains(&"第 3 页".to_owned())
                && t.contains(&"渲染中…".to_owned()),
            "{t:?}"
        );
        assert!(!t.contains(&"第 1 页".to_owned()), "视口外的页不画");
        assert!(list.finish().is_ok());
    }

    #[test]
    fn unsupported_files_offer_to_open_externally_and_show_the_path() {
        let content = Content::Unsupported { message: None };
        let area = Rect::new(400.0, 68.0, 1200.0, 800.0);
        let lay = layout(area, &content);
        let (btn, _) = lay
            .entries
            .iter()
            .find(|(_, h)| *h == Hit::OpenExternal)
            .unwrap();
        assert_eq!(
            lay.hit((btn.left + btn.right) / 2.0, (btn.top + btn.bottom) / 2.0),
            Some(Hit::OpenExternal)
        );
        let mut list = DrawList::new();
        let path = PathBuf::from("D:/ws/weird.xyz");
        let model = Model {
            content: &content,
            path: &path,
            image_src: "",
            now_ms: 0,
            hover: None,
        };
        paint(
            &mut list,
            area,
            &lay,
            &model,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        assert!(t.contains(&"Unsupported File Type".to_owned()));
        assert!(t.iter().any(|s| s.ends_with("weird.xyz")));
        assert!(list.finish().is_ok());
    }
}
