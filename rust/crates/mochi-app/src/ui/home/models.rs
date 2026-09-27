//! 定义首页仪表板及其文档、日程和会话条目。
use super::*;

/// 宿主一次性传入数据，首页不逐帧读盘；列表顺序由宿主决定。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dashboard {
    pub recent_documents: Vec<DashboardDocument>,
    pub favorite_documents: Vec<DashboardDocument>,
    pub inbox_items: Vec<DashboardInboxItem>,
    pub libraries: Vec<DashboardLibrary>,
    pub ai_sessions: Vec<DashboardSession>,
    pub schedule_items: Vec<DashboardScheduleItem>,
    /// 当前工作区内真实的固定时间冲突数量。
    pub schedule_conflict_count: usize,
    /// 今天的负载等级：`low` / `medium` / `high`。
    pub schedule_risk: String,
    pub journal: DashboardJournal,
    pub graph: DashboardGraph,
}

/// 首页文档入口。`path` 是工作区内的绝对路径，Action 使用列表下标，
/// 供给侧在处理 Action 时按同一份 Dashboard 取回路径。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardDocument {
    pub path: PathBuf,
    pub title: String,
    /// 库名 / 父目录等一行辅助信息。
    pub subtitle: String,
    pub mtime_ms: i64,
    pub favorite: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardInboxItem {
    pub id: String,
    pub content: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardLibrary {
    pub id: String,
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardSession {
    pub id: String,
    pub title: String,
    pub updated_at_ms: i64,
    pub message_count: usize,
    pub pinned: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardScheduleItem {
    pub id: String,
    pub title: String,
    pub time_label: String,
    pub status: String,
    pub kind: String,
    pub priority: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardJournal {
    /// 由供给侧传入；新建日记应使用 `<workspace>/日记/<YYYY-MM-DD>.md`。
    pub path: PathBuf,
    pub date: String,
    pub exists: bool,
    pub words: usize,
    pub excerpt: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardGraph {
    pub local_link_count: i64,
    pub nodes: Vec<DashboardGraphNode>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardGraphNode {
    pub path: PathBuf,
    pub title: String,
    pub link_count: i64,
}

/// 首页所有可点击入口的结果。
///
/// 下标 Action（文档、库、图谱节点）对应传入 Dashboard 中的原始列表下标；
/// 列表可以比首页显示的数量长，首页仅显示前 3–5 项。宿主负责把 Action
/// 转换为切换视图、打开文件或启动对应的窗口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    NewNote,
    OpenAi,
    NewJournal,
    ImportFile,
    #[allow(dead_code)]
    Capture,
    OpenRecentView,
    OpenFavoritesView,
    OpenInbox,
    OpenSchedule,
    OpenJournal,
    OpenRecent(usize),
    OpenFavorite(usize),
    OpenLibrary(usize),
    OpenAiSession(String),
    OpenGraphNode(usize),
}

/// 面板之间的间距与内边距。对应 TSX 的 `p-4` / `gap-3`。
pub(super) fn gap() -> f32 {
    crate::ui::settings_values::number("dashboard.panelGap", 16.0)
}

pub(super) fn panel_padding() -> f32 {
    crate::ui::settings_values::number("dashboard.panelPadding", 16.0)
}

/// 统计小格的高度。TSX 是内容撑出来的：图标行 + 数值行 + 增量提示。
pub(super) fn tile_height() -> f32 {
    crate::ui::settings_values::number("dashboard.statCardHeight", 88.0)
}

/// 一行放几个统计小格（TSX 是 `lg:grid-cols-4`）。
pub(super) const TILES_PER_ROW: usize = 4;

/// 柱状图高度。
pub(super) fn chart_height() -> f32 {
    crate::ui::settings_values::number("dashboard.chartHeight", 72.0)
}

/// 热力图每格的边长与间隔。
pub(super) fn heat_cell() -> f32 {
    crate::ui::settings_values::number("dashboard.heatCellSize", 11.0)
}

pub(super) fn heat_gap() -> f32 {
    crate::ui::settings_values::number("dashboard.heatCellGap", 3.0)
}

pub(super) fn action_tile_height() -> f32 {
    crate::ui::settings_values::number("dashboard.actionHeight", 48.0)
}

pub(super) fn document_row_height() -> f32 {
    crate::ui::settings_values::number("dashboard.documentRowHeight", 56.0)
}

pub(super) fn inbox_row_height() -> f32 {
    crate::ui::settings_values::number("dashboard.inboxRowHeight", 48.0)
}

pub(super) fn library_row_height() -> f32 {
    crate::ui::settings_values::number("dashboard.libraryRowHeight", 44.0)
}

pub(super) fn session_row_height() -> f32 {
    crate::ui::settings_values::number("dashboard.sessionRowHeight", 44.0)
}

pub(super) fn schedule_row_height() -> f32 {
    crate::ui::settings_values::number("dashboard.scheduleRowHeight", 62.0)
}

pub(super) fn journal_height() -> f32 {
    crate::ui::settings_values::number("dashboard.journalHeight", 126.0)
}

pub(super) fn graph_row_height() -> f32 {
    crate::ui::settings_values::number("dashboard.graphRowHeight", 42.0)
}

/// 排好版的首页。与文档视图同样的做法：先算出所有块的位置，
/// 绘制时按视口切片——首页很长，全画会拖垮每帧。
pub struct Layout {
    pub(super) blocks: Vec<Block>,
    pub height: f32,
}

pub(super) enum Block {
    /// 面板底板。
    Panel { rect: Rect, quiet: bool },
    /// 面板标题。
    SectionTitle { rect: Rect, text: String },
    /// 大号标题（今日战报的那句话）。
    Headline { rect: Rect, text: String },
    /// 首页问候语，与今日战报分层显示。
    Greeting { rect: Rect, text: String },
    /// 统计小格：标签 + 数值 + 单位 + 增量。
    Tile {
        rect: Rect,
        label: String,
        value: String,
        unit: String,
        delta: Option<i64>,
    },
    /// 一行说明文字。
    Caption { rect: Rect, text: String },
    /// 柱状图。`bars` 是 (标签, 高度比例 0..1, 是否高亮)。
    Bars {
        rect: Rect,
        bars: Vec<(String, f32, bool)>,
    },
    /// 活跃热力图。`cells` 是 (列, 行, 强度 0..1)。
    Heatmap {
        rect: Rect,
        cells: Vec<(usize, usize, f32)>,
    },
    /// 最近七天按星期×小时聚合的真实活动热力图。
    HourlyHeatmap {
        rect: Rect,
        cells: Vec<(usize, usize, f32)>,
    },
    /// 文件类型占用的堆叠条。
    StorageBar {
        rect: Rect,
        segments: Vec<(String, u64)>,
        total: u64,
    },
    /// 一枚标签（连续天数徽章、画像标签）。
    Badge {
        rect: Rect,
        text: String,
        accent: bool,
    },
    /// AI 卡片中的一行建议。
    PromptRow {
        rect: Rect,
        text: String,
        action: Action,
    },
    /// 画像中的键值行。
    DetailRow {
        rect: Rect,
        label: String,
        value: String,
    },
    /// 快速开始中的紧凑按钮，只有新建笔记使用主操作色。
    ActionTile {
        rect: Rect,
        icon: Icon,
        title: String,
        action: Action,
    },
    /// 最近文档 / 收藏文档的一行。
    DocumentRow {
        rect: Rect,
        title: String,
        subtitle: String,
        mtime_ms: i64,
        favorite: bool,
        action: Action,
    },
    /// 收件箱的一行预览。
    InboxRow {
        rect: Rect,
        content: String,
        created_at_ms: i64,
        action: Action,
    },
    /// 知识库快捷入口。
    LibraryRow {
        rect: Rect,
        name: String,
        type_name: String,
        action: Action,
    },
    /// AI 最近会话。
    SessionRow {
        rect: Rect,
        title: String,
        meta: String,
        pinned: bool,
        action: Action,
    },
    /// 今日日程的一行预览。
    ScheduleRow {
        rect: Rect,
        time: String,
        title: String,
        status: String,
        kind: String,
        priority: String,
        action: Action,
    },
    /// 今日日记摘要。
    JournalPreview {
        rect: Rect,
        date: String,
        excerpt: String,
        words: usize,
        exists: bool,
        action: Action,
    },
    /// 图谱预览中的中心节点。
    GraphNodeRow {
        rect: Rect,
        title: String,
        link_count: i64,
        action: Action,
    },
    /// 空列表或后台加载提示。
    Empty { rect: Rect, text: String },
    /// 右上角的轻量文字入口（「打开」「查看全部」）。
    ActionLink {
        rect: Rect,
        text: String,
        action: Action,
    },
}
