//! 桌面卡片的持久化协议、模块模板和只读快照。
//!
//! 配置只保存布局和展示选择，不保存笔记正文、凭据或可执行内容。快照
//! 提供器在 desktop_cards/providers.rs 中，并对卡片、读取字节和行数设上限。

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{jstime, paths};

pub mod collection;
#[cfg(test)]
mod collection_tests;
#[path = "desktop_cards/extra_provider.rs"]
mod extra_provider;
#[path = "desktop_cards/folder.rs"]
pub mod folder;
pub mod folder_auto;
#[path = "desktop_cards/folder_operations.rs"]
pub mod folder_operations;
#[cfg(test)]
#[path = "desktop_cards/folder_tests.rs"]
mod folder_tests;
pub mod folder_watch;
#[path = "desktop_cards/mutations.rs"]
mod mutations;
#[path = "desktop_cards/providers.rs"]
mod providers;
pub mod schedule_rules;
#[path = "desktop_cards/source_provider.rs"]
mod source_provider;
pub mod studio;
#[path = "desktop_cards/weather.rs"]
pub mod weather;

pub use mutations::toggle_schedule_item;
pub use providers::{build_snapshot, build_snapshot_with_limits, SnapshotBuilder, SnapshotLimits};

pub const DESKTOP_CARDS_VERSION: u32 = 1;
pub const DESKTOP_CARDS_FILE_NAME: &str = "desktop-cards.json";
pub const MAX_CONFIG_BYTES: usize = 512 * 1024;
pub const MAX_CARDS: usize = 12;
pub const MAX_PAGES_PER_CARD: usize = 8;
pub const MAX_OPTIONS_PER_PAGE: usize = 64;
pub const MAX_ROWS_PER_PAGE: usize = 100_000;
pub const MAX_TITLE_CHARS: usize = 120;
pub const MAX_ID_CHARS: usize = 160;
pub const MAX_SOURCE_CHARS: usize = 1024;
pub const MIN_CARD_WIDTH: u32 = 280;
pub const MIN_CARD_HEIGHT: u32 = 220;
pub const MAX_CARD_WIDTH: u32 = 720;
pub const MAX_CARD_HEIGHT: u32 = 900;
pub const DEFAULT_CARD_WIDTH: u32 = 480;
pub const DEFAULT_CARD_HEIGHT: u32 = 480;
pub const DEFAULT_PAGE_LIMIT: usize = 8;

static ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Module {
    Home,
    Schedule,
    Inbox,
    QuickNote,
    Recent,
    Favorites,
    Knowledge,
    Document,
    Base,
    Canvas,
    QuickNav,
    Folder,
    Weather,
    Music,
    Search,
    English,
    Exam,
    Ai,
    Templates,
    Automations,
    Pomodoro,
    AgentConfig,
    Custom,
    Clock,
    Shortcuts,
}

impl Module {
    pub const ALL: &'static [Module] = &[
        Module::Home,
        Module::Schedule,
        Module::Inbox,
        Module::Recent,
        Module::Favorites,
        Module::Knowledge,
        Module::Base,
        Module::Canvas,
        Module::Document,
        Module::Folder,
        Module::Weather,
        Module::Music,
        Module::Search,
        Module::Exam,
        Module::Ai,
        Module::Templates,
        Module::Automations,
        Module::Pomodoro,
        Module::Custom,
        Module::Clock,
        Module::Shortcuts,
    ];

    pub fn all() -> &'static [Module] {
        Self::ALL
    }

    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Custom => "custom",
            Self::Clock => "clock",
            Self::Shortcuts => "shortcuts",
            Self::Home => "home",
            Self::Schedule => "schedule",
            Self::Inbox => "inbox",
            Self::QuickNote => "quickNote",
            Self::Recent => "recent",
            Self::Favorites => "favorites",
            Self::Knowledge => "knowledge",
            Self::Document => "document",
            Self::Base => "base",
            Self::Canvas => "canvas",
            Self::QuickNav => "quickNav",
            Self::English => "english",
            Self::Exam => "exam",
            Self::Ai => "ai",
            Self::Templates => "templates",
            Self::Automations => "automations",
            Self::Pomodoro => "pomodoro",
            Self::AgentConfig => "agentConfig",
            Self::Folder => "folder",
            Self::Weather => "weather",
            Self::Music => "music",
            Self::Search => "search",
        }
    }

    pub fn from_wire(value: &str) -> Option<Self> {
        [
            Self::QuickNote,
            Self::QuickNav,
            Self::English,
            Self::AgentConfig,
        ]
        .iter()
        .chain(Self::ALL.iter())
        .copied()
        .find(|module| module.wire_name() == value)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Custom => "自定义工作台",
            Self::Clock => "桌面时钟",
            Self::Shortcuts => "图标收纳器",
            Self::Home => "工作台",
            Self::Schedule => "日程待办",
            Self::Inbox => "收件箱",
            Self::QuickNote => "速记",
            Self::Recent => "最近",
            Self::Favorites => "收藏",
            Self::Knowledge => "知识库",
            Self::Document => "文档",
            Self::Base => "多维表格",
            Self::Canvas => "画布",
            Self::QuickNav => "快速导航",
            Self::English => "英语学习",
            Self::Exam => "试卷",
            Self::Ai => "墨池 AI",
            Self::Templates => "模板",
            Self::Automations => "自动化",
            Self::Pomodoro => "番茄钟",
            Self::AgentConfig => "Agent 配置",
            Self::Folder => "文件夹映射",
            Self::Weather => "桌面天气",
            Self::Music => "音乐控制",
            Self::Search => "桌面搜索",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Custom => "在画布自由组合组件、参数与事件",
            Self::Clock => "可自定义字号和格式的大时钟",
            Self::Shortcuts => "拖入文件创建快捷方式，双击打开",
            Self::Home => "常用功能入口，可使用列表或网格布局",
            Self::Schedule => "日期列表、月历或自定义规则分组",
            Self::Inbox => "待处理的速记和 Agent 审批",
            Self::QuickNote => "快速记录入口和最近速记",
            Self::Recent => "最近打开或修改的文档",
            Self::Favorites => "收藏的文档和目录",
            Self::Knowledge => "知识库、文档和最近内容",
            Self::Document => "所选知识库、文件夹或文件中的文档",
            Self::Base => "多维表格和最近记录",
            Self::Canvas => "画布和最近修改",
            Self::QuickNav => "快速打开的链接和程序",
            Self::English => "待复习词汇和学习进度",
            Self::Exam => "最近试卷和待完成练习",
            Self::Ai => "AI 会话和待处理事项",
            Self::Templates => "可用的文档模板",
            Self::Automations => "自动化状态和下一次运行",
            Self::Pomodoro => "当前专注状态和今日次数",
            Self::AgentConfig => "Agent、技能和快捷操作",
            Self::Folder => "映射本地文件夹的一层内容，不读取文件正文",
            Self::Weather => "当前天气、逐小时与七日预报",
            Self::Music => "控制 Windows 当前媒体会话，切换播放来源",
            Self::Search => "通过 Everything 查找电脑文件，并搜索墨池内容",
        }
    }

    pub fn options(self) -> Vec<ModuleOption> {
        use ModuleOption as O;
        match self {
            Self::Weather | Self::Music | Self::Search => {
                vec![O::new("content", "内容", "显示当前配置的数据", true)]
            }
            Self::Custom | Self::Clock | Self::Shortcuts => vec![O::new(
                "widgets",
                "组件画布",
                "在设计工作台中自由编辑",
                true,
            )],
            Self::Home => vec![
                O::new("home", "首页", "打开首页", true),
                O::new("inbox", "收件箱", "打开收件箱", true),
                O::new("favorites", "收藏", "打开收藏", true),
                O::new("schedule", "日程", "打开日程", true),
                O::new("ai", "墨池 AI", "打开 AI", true),
                O::new("quickNote", "小记", "打开小记", true),
                O::new("recent", "最近", "最近使用的文档", true),
                O::new("templates", "模板中心", "打开模板中心", true),
                O::new("automations", "自动化", "打开自动化", true),
                O::new("knowledge", "知识库", "打开知识库", true),
                O::new("settings", "设置", "打开设置", true),
                O::new("desktopCards", "桌面卡片", "管理桌面卡片", true),
                O::new("notifications", "通知中心", "查看通知", true),
                O::new("marketplace", "官方市场", "打开官方市场", true),
            ],
            Self::Schedule => vec![
                O::new("tasks", "待办", "展示未完成的待办任务", true),
                O::new("events", "日程", "展示未来一周的日程与时间块", true),
                O::new(
                    "completed",
                    "已完成",
                    "包含已完成的任务和已执行的日程",
                    false,
                ),
            ],
            Self::Inbox => vec![
                O::new("pending", "待处理", "未归档的收件箱条目", true),
                O::new("latest", "已归档", "已归档的收件箱条目", false),
            ],
            Self::QuickNote => vec![
                O::new("capture", "新建速记", "直接打开快速记录", true),
                O::new("latest", "最近速记", "最近的几条速记内容摘要", true),
            ],
            Self::Recent => vec![
                O::new("documents", "文档", "最近使用的文档", true),
                O::new("modifiedAt", "修改时间", "显示文档最近修改时间", true),
            ],
            Self::Favorites => vec![O::new("documents", "收藏内容", "收藏的文档和目录", true)],
            Self::Knowledge => vec![O::new("libraries", "知识库", "知识库入口", true)],
            Self::Document => vec![
                O::new("documents", "文档", "展示所选来源中的文档", true),
                O::new("modifiedAt", "修改时间", "在名称下显示最近修改时间", false),
            ],
            Self::Base => vec![
                O::new("tables", "表格", "多维表格入口", true),
                O::new("records", "记录", "最近更新的记录", false),
            ],
            Self::Canvas => vec![
                O::new("canvases", "画布", "画布入口", true),
                O::new("cards", "画布对象", "画布中的对象摘要", false),
                O::new("modifiedAt", "修改时间", "画布的修改时间", false),
            ],
            Self::QuickNav => vec![
                O::new("shortcuts", "快捷项", "快速导航里的项目", true),
                O::new("recent", "最近打开", "最近打开的快捷项", false),
            ],
            Self::English => vec![
                O::new("review", "待复习", "当前需要复习的词汇", true),
                O::new("stats", "学习进度", "词汇和正确率统计", true),
                O::new("recent", "最近学习", "最近复习的词汇", false),
            ],
            Self::Exam => vec![
                O::new("recent", "最近试卷", "最近打开的试卷", true),
                O::new("due", "待完成", "尚未完成的练习", true),
            ],
            Self::Ai => vec![
                O::new("sessions", "最近会话", "最近的 AI 会话", true),
                O::new("inbox", "待处理", "需要用户确认的操作", true),
            ],
            Self::Templates => vec![O::new("templates", "模板列表", "可用的文档模板", true)],
            Self::Automations => vec![
                O::new("enabled", "启用状态", "自动化是否启用", true),
                O::new("nextRun", "下次运行", "下一次计划运行时间", true),
                O::new("recent", "最近运行", "最近一次运行结果", false),
            ],
            Self::Pomodoro => vec![
                O::new("status", "当前状态", "专注、休息或空闲", true),
                O::new("today", "今日次数", "今日完成的专注次数", true),
            ],
            Self::AgentConfig => vec![
                O::new("agents", "Agents", "可用 Agent 配置", true),
                O::new("skills", "Skills", "可用技能配置", false),
                O::new("quickActions", "快捷操作", "Agent 快捷操作", true),
                O::new("mcps", "MCP", "MCP 服务配置", false),
            ],
            Self::Folder => vec![
                O::new("files", "文件", "显示所选文件夹中的文件", true),
                O::new(
                    "folders",
                    "子文件夹",
                    "显示所选文件夹中的直接子文件夹",
                    true,
                ),
            ],
        }
    }

    pub fn default_page(self) -> Page {
        Page::new(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModuleOption {
    pub key: String,
    pub label: String,
    pub description: String,
    pub default_enabled: bool,
}

/// 卡片上可直接执行轻量动作，还是统一跳转到墨池主窗口。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Interaction {
    Smart,
    OpenOnly,
}

impl Default for Interaction {
    fn default() -> Self {
        Self::Smart
    }
}

impl ModuleOption {
    fn new(key: &str, label: &str, description: &str, default_enabled: bool) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            description: description.into(),
            default_enabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum Action {
    DesktopUtility(String),
    OpenModule(String),
    OpenFile(String),
    OpenFolderEntry(String),
    ToggleTask { id: String, checked: bool },
    Capture,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub action: Option<Action>,
    pub checked: Option<bool>,
    pub meta: RowMeta,
}

impl Default for Row {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            detail: String::new(),
            action: None,
            checked: None,
            meta: RowMeta::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemStyle {
    pub foreground: Option<u32>,
    pub background: Option<u32>,
}
impl ItemStyle {
    pub fn over(&self, fallback: &Self) -> Self {
        Self {
            foreground: self.foreground.or(fallback.foreground),
            background: self.background.or(fallback.background),
        }
    }
}
pub fn item_key(id: &str, path: &str) -> String {
    if path.is_empty() {
        id.into()
    } else {
        format!("path:{}", path.replace('\\', "/"))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PageSnapshot {
    pub title: String,
    pub subtitle: String,
    pub rows: Vec<Row>,
    pub empty_message: String,
}

impl PageSnapshot {
    pub fn empty(page: &Page) -> Self {
        Self {
            title: page.title.clone(),
            subtitle: page.module.description().into(),
            rows: Vec::new(),
            empty_message: format!("{}暂无内容", page.title),
        }
    }
}

impl Default for PageSnapshot {
    fn default() -> Self {
        Self {
            title: String::new(),
            subtitle: String::new(),
            rows: Vec::new(),
            empty_message: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    pub generated_at: String,
    pub pages: BTreeMap<String, PageSnapshot>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            generated_at: String::new(),
            pages: BTreeMap::new(),
        }
    }
}

pub fn page_key(card_id: &str, page_id: &str) -> String {
    format!("{card_id}/{page_id}")
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RowMeta {
    pub date: String,
    pub group: String,
    pub icon: String,
    pub path: String,
    pub depth: usize,
    pub directory: bool,
    pub always_detail: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompletedBehavior {
    Hide,
    Strike,
    Keep,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Presentation {
    /// None 表示沿用已有页面上的旧版 `completed` 选项。
    pub completed_behavior: Option<CompletedBehavior>,
    pub show_names: bool,
    pub show_modified: bool,
    pub item_foreground: Option<u32>,
    pub item_background: Option<u32>,
    pub show_checks: bool,
    pub show_icons: bool,
    pub show_extensions: bool,
    pub show_numbers: bool,
    pub show_groups: bool,
    pub heading_size: u8,
    /// 0：日期列表；1：月历；2：自定义分组。
    pub schedule_view: u8,
    pub calendar_expanded: bool,
    pub groups: Vec<ScheduleGroup>,
    pub grid: bool,
    pub grid_lines: bool,
    /// 0 表示贴合配置的行数；否则为 DIP 单元格高度。
    pub grid_height: u16,
    pub columns: u8,
    pub rows: u8,
    pub expand_libraries: bool,
}
impl Default for Presentation {
    fn default() -> Self {
        Self {
            completed_behavior: None,
            show_names: true,
            show_modified: false,
            item_foreground: None,
            item_background: None,
            show_checks: true,
            show_icons: true,
            show_extensions: false,
            show_numbers: false,
            show_groups: true,
            heading_size: 12,
            schedule_view: 0,
            calendar_expanded: true,
            groups: vec![
                ScheduleGroup {
                    name: "逾期".into(),
                    rule: "日期 < 今日".into(),
                },
                ScheduleGroup {
                    name: "待办".into(),
                    rule: "日期 >= 今日".into(),
                },
            ],
            grid: false,
            grid_lines: false,
            grid_height: 96,
            columns: 3,
            rows: 3,
            expand_libraries: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleGroup {
    pub name: String,
    pub rule: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct UtilityConfig {
    pub location: String,
    pub query: String,
    pub media_source: String,
}
impl UtilityConfig {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Page {
    #[serde(default, skip_serializing_if = "UtilityConfig::is_default")]
    pub utility: UtilityConfig,
    pub sources: Vec<String>,
    pub item_styles: BTreeMap<String, ItemStyle>,
    pub studio: studio::Studio,
    pub presentation: Presentation,
    pub id: String,
    pub title: String,
    pub module: Module,
    pub options: Vec<String>,
    pub source: Option<String>,
    pub limit: usize,
    #[serde(default, skip_serializing_if = "folder::FolderConfig::is_default")]
    pub folder: folder::FolderConfig,
    #[serde(default)]
    pub interaction: Interaction,
}

impl Page {
    pub fn new(module: Module) -> Self {
        Self {
            utility: UtilityConfig::default(),
            sources: vec![],
            item_styles: BTreeMap::new(),
            studio: {
                let mut s = studio::Studio::default();
                if module == Module::Clock {
                    s.nodes.push(studio::Node::new(studio::Kind::Clock));
                    let mut d = studio::Node::new(studio::Kind::Date);
                    d.y = 3500;
                    d.width = 9000;
                    s.nodes.push(d);
                }
                s
            },
            presentation: Presentation {
                grid: matches!(module, Module::Shortcuts | Module::Folder),
                ..Default::default()
            },
            id: new_id("page"),
            title: module.label().into(),
            module,
            options: module
                .options()
                .into_iter()
                .filter(|option| option.default_enabled)
                .map(|option| option.key)
                .collect(),
            source: None,
            limit: if matches!(
                module,
                Module::Home
                    | Module::Schedule
                    | Module::Knowledge
                    | Module::Recent
                    | Module::Favorites
                    | Module::Document
                    | Module::Shortcuts
                    | Module::Base
                    | Module::Canvas
                    | Module::Exam
                    | Module::Folder
            ) {
                0
            } else {
                DEFAULT_PAGE_LIMIT
            },
            folder: folder::FolderConfig::default(),
            interaction: Interaction::Smart,
        }
    }

    pub fn with_id(module: Module, id: impl Into<String>) -> Self {
        let mut page = Self::new(module);
        page.id = id.into();
        page
    }

    pub fn selected(&self, key: &str) -> bool {
        self.options.iter().any(|option| option == key)
    }

    pub fn completed_behavior(&self) -> CompletedBehavior {
        self.presentation.completed_behavior.unwrap_or_else(|| {
            if self.selected("completed") {
                CompletedBehavior::Keep
            } else {
                CompletedBehavior::Hide
            }
        })
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::new(Module::Home)
    }
}

/// 轻磨砂玻璃：模糊的壁纸透过一层浅色中性色调显现，配深色文字；
/// 绘制时加柔和的玻璃高光和阴影。
pub const GLASS_BACKGROUND: u32 = 0xe9edf1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Appearance {
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub capsule: bool,
    pub opacity: u8,
    pub font_size: u8,
    pub font_color: Option<u32>,
    pub background_color: Option<u32>,
    /// 0 = 紧凑，1 = 舒适，2 = 宽敞。
    pub spacing: u8,
    pub show_details: bool,
    pub show_border: bool,
    pub show_separators: bool,
    pub row_padding: u8,
    pub show_single_page_name: bool,
    pub tabs_left: bool,
    /// 0 保持自动导航尺寸；否则为分割轴上的百分比。
    pub tabs_ratio: u8,
    pub tabs_divider: bool,
    pub edge_dock: bool,
    pub dock_speed: u8,
    pub pinned: bool,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            capsule: false,
            opacity: 85,
            font_size: 18,
            font_color: None,
            background_color: Some(GLASS_BACKGROUND),
            spacing: 1,
            show_details: false,
            show_border: false,
            show_separators: true,
            row_padding: 4,
            show_single_page_name: true,
            tabs_left: false,
            tabs_ratio: 0,
            tabs_divider: false,
            edge_dock: false,
            dock_speed: 1,
            pinned: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Card {
    pub appearance: Appearance,
    pub id: String,
    pub title: String,
    pub enabled: bool,
    pub locked: bool,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub active_page: String,
    pub pages: Vec<Page>,
}

impl Card {
    pub fn new(title: impl Into<String>, module: Module) -> Self {
        let page = module.default_page();
        Self {
            appearance: Appearance::default(),
            id: new_id("card"),
            title: title.into(),
            enabled: true,
            locked: false,
            x: 24,
            y: 24,
            width: DEFAULT_CARD_WIDTH,
            height: DEFAULT_CARD_HEIGHT,
            active_page: page.id.clone(),
            pages: vec![page],
        }
    }

    pub fn active_page_mut(&mut self) -> Option<&mut Page> {
        self.pages.iter_mut().find(|p| p.id == self.active_page)
    }
    pub fn active_page(&self) -> Option<&Page> {
        self.pages
            .iter()
            .find(|page| page.id == self.active_page)
            .or_else(|| self.pages.first())
    }
}

impl Default for Card {
    fn default() -> Self {
        Self::new("我的卡片", Module::Home)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopConfig {
    pub version: u32,
    pub cards: Vec<Card>,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            version: DESKTOP_CARDS_VERSION,
            cards: Vec::new(),
        }
    }
}

impl DesktopConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn path(workspace: impl AsRef<Path>) -> PathBuf {
        paths::mochi_dir(workspace).join(DESKTOP_CARDS_FILE_NAME)
    }

    pub fn load(workspace: impl AsRef<Path>) -> Result<Self> {
        Self::load_from_path(Self::path(workspace))
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => {
                return Err(error).with_context(|| format!("读取桌面卡片失败: {}", path.display()))
            }
        };
        if metadata.len() > MAX_CONFIG_BYTES as u64 {
            bail!("桌面卡片配置超过 {} 字节上限", MAX_CONFIG_BYTES);
        }
        let text = fs::read_to_string(path)
            .with_context(|| format!("读取桌面卡片失败: {}", path.display()))?;
        let mut config: Self = serde_json::from_str(&text)
            .with_context(|| format!("桌面卡片配置损坏，原文件已保留: {}", path.display()))?;
        config.migrate_templates();
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, workspace: impl AsRef<Path>) -> Result<()> {
        self.save_to_path(Self::path(workspace))
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self)?;
        if bytes.len() > MAX_CONFIG_BYTES {
            bail!("桌面卡片配置超过 {} 字节上限", MAX_CONFIG_BYTES);
        }
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| anyhow::anyhow!("桌面卡片配置没有父目录"))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("创建桌面卡片目录失败: {}", parent.display()))?;
        atomic_write(path, &bytes)
    }

    pub fn export_json(&self) -> Result<String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self)?;
        if bytes.len() > MAX_CONFIG_BYTES {
            bail!("导出的桌面卡片配置超过 {} 字节上限", MAX_CONFIG_BYTES);
        }
        String::from_utf8(bytes).context("桌面卡片配置不是有效 UTF-8")
    }

    pub fn import_json(text: &str) -> Result<Self> {
        if text.len() > MAX_CONFIG_BYTES {
            bail!("导入文件超过 {} 字节上限", MAX_CONFIG_BYTES);
        }
        let mut config: Self = serde_json::from_str(text).context("桌面卡片导入文件格式错误")?;
        config.migrate_templates();
        config.validate()?;
        for card in &mut config.cards {
            card.enabled = false;
            for page in &mut card.pages {
                page.folder.auto_organize = false;
            }
        }
        Ok(config)
    }

    pub fn merge_import(&mut self, text: &str) -> Result<Vec<String>> {
        let mut imported = Self::import_json(text)?;
        regenerate_ids(&mut imported);
        let mut candidate = self.clone();
        let ids = imported
            .cards
            .iter()
            .map(|card| card.id.clone())
            .collect::<Vec<_>>();
        candidate.cards.extend(imported.cards);
        candidate.validate()?;
        *self = candidate;
        Ok(ids)
    }

    pub fn migrate_templates(&mut self) {
        for card in &mut self.cards {
            // 更早的玻璃预设先是蓝色、再是烟熏暗色；统一迁到当前色调。
            if matches!(
                card.appearance.background_color,
                Some(0x1f5a9e | 0x1664c8 | 0x474c54)
            ) {
                card.appearance.background_color = Some(GLASS_BACKGROUND);
            }
            for page in &mut card.pages {
                if page.module == Module::Shortcuts {
                    page.presentation.grid = true;
                    if page.presentation.grid_height == 0 {
                        page.presentation.grid_height = 96;
                    }
                    for node in &mut page.studio.nodes {
                        if node.kind == studio::Kind::Shortcut
                            && (node.foreground.is_some() || node.background.is_some())
                        {
                            page.item_styles
                                .entry(format!("node:{}", node.id))
                                .or_insert(ItemStyle {
                                    foreground: node.foreground,
                                    background: node.background,
                                });
                            node.foreground = None;
                            node.background = None;
                        }
                    }
                }

                if page.module == Module::Home
                    && page
                        .options
                        .iter()
                        .any(|key| matches!(key.as_str(), "today" | "stats"))
                {
                    page.options = Module::Home.default_page().options;
                    page.limit = 0;
                }
                if page.module == Module::Schedule
                    && page.options.iter().any(|key| {
                        matches!(
                            key.as_str(),
                            "today" | "overdue" | "upcoming" | "projects" | "habits"
                        )
                    })
                {
                    page.options = Module::Schedule.default_page().options;
                    page.limit = 0;
                }
                if page.module == Module::Knowledge
                    && page.options.iter().any(|key| key == "recent")
                {
                    page.options.retain(|key| key != "recent");
                    if !page.options.iter().any(|key| key == "libraries") {
                        page.options.push("libraries".into());
                    }
                    page.limit = 0;
                }
            }
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != DESKTOP_CARDS_VERSION {
            bail!(
                "不支持的桌面卡片版本 {}，当前版本为 {}",
                self.version,
                DESKTOP_CARDS_VERSION
            );
        }
        if self.cards.len() > MAX_CARDS {
            bail!("桌面卡片数量超过 {} 张上限", MAX_CARDS);
        }
        let mut card_ids = HashSet::new();
        for card in &self.cards {
            validate_id(&card.id, "卡片 ID")?;
            if !card_ids.insert(card.id.as_str()) {
                bail!("卡片 ID 重复: {}", card.id);
            }
            validate_title(&card.title, "卡片标题")?;
            anyhow::ensure!(
                (35..=100).contains(&card.appearance.opacity),
                "不透明度须为 35–100%"
            );
            anyhow::ensure!(
                (8..=72).contains(&card.appearance.font_size),
                "字号须为 8–72 px"
            );
            anyhow::ensure!(
                card.appearance.font_color.is_none_or(|c| c <= 0xffffff),
                "字体颜色须为 RGB 色值"
            );
            anyhow::ensure!(
                card.appearance
                    .background_color
                    .is_none_or(|c| c <= 0xffffff),
                "背景颜色须为 RGB 色值"
            );
            anyhow::ensure!(card.appearance.spacing <= 2, "行距须为紧凑、舒适或宽松");
            anyhow::ensure!(card.appearance.row_padding <= 64, "上下留白须为 0–64 px");
            anyhow::ensure!(card.appearance.tabs_ratio <= 60, "分页区域比例须为 0–60%");
            if !(MIN_CARD_WIDTH..=MAX_CARD_WIDTH).contains(&card.width) {
                bail!("卡片 {} 宽度超出范围", card.id);
            }
            if !(MIN_CARD_HEIGHT..=MAX_CARD_HEIGHT).contains(&card.height) {
                bail!("卡片 {} 高度超出范围", card.id);
            }
            if card.pages.is_empty() || card.pages.len() > MAX_PAGES_PER_CARD {
                bail!("卡片 {} 的分页数量无效", card.id);
            }
            let mut page_ids = HashSet::<String>::new();
            for page in &card.pages {
                folder::validate(&page.folder)?;
                for value in [
                    &page.utility.location,
                    &page.utility.query,
                    &page.utility.media_source,
                ] {
                    if value.chars().count() > 512 || value.chars().any(char::is_control) {
                        bail!("桌面工具配置过长或包含控制字符");
                    }
                }
                anyhow::ensure!(
                    page.sources.len() <= 256 && page.item_styles.len() <= 10000,
                    "来源最多 256 项，单项样式最多 10000 项"
                );
                for source in &page.sources {
                    validate_relative_path(source, "分页来源")?;
                }
                for color in [
                    page.presentation.item_foreground,
                    page.presentation.item_background,
                ]
                .into_iter()
                .chain(
                    page.item_styles
                        .values()
                        .flat_map(|v| [v.foreground, v.background]),
                ) {
                    anyhow::ensure!(color.is_none_or(|c| c <= 0xffffff), "条目颜色必须为 RGB");
                }
                page.studio.validate()?;
                let style = &page.presentation;
                anyhow::ensure!(
                    style.grid_height <= 600 && card.appearance.dock_speed <= 3,
                    "网格高度或收纳速度无效"
                );
                anyhow::ensure!(
                    (8..=36).contains(&style.heading_size) && style.schedule_view <= 2,
                    "分页显示格式无效"
                );
                anyhow::ensure!(
                    (1..=12).contains(&style.columns) && (1..=12).contains(&style.rows),
                    "网格行列须为 1–12"
                );
                anyhow::ensure!(style.groups.len() <= 32, "自定义分组最多 32 个");
                for group in &style.groups {
                    validate_title(&group.name, "分组名")?;
                    schedule_rules::validate(&group.rule)?;
                }

                validate_page(page, &mut page_ids)?;
            }
            if !card.pages.iter().any(|page| page.id == card.active_page) {
                bail!("卡片 {} 的 activePage 不存在", card.id);
            }
        }
        Ok(())
    }
}

fn validate_page(page: &Page, page_ids: &mut HashSet<String>) -> Result<()> {
    validate_id(&page.id, "分页 ID")?;
    if !page_ids.insert(page.id.clone()) {
        bail!("分页 ID 重复: {}", page.id);
    }
    validate_title(&page.title, "分页标题")?;
    if page.options.len() > MAX_OPTIONS_PER_PAGE {
        bail!("分页 {} 的选项过多", page.id);
    }
    let descriptors = page.module.options();
    let known = descriptors
        .iter()
        .map(|item| item.key.as_str())
        .collect::<HashSet<_>>();
    let mut options = HashSet::new();
    for option in &page.options {
        validate_id(option, "模块选项")?;
        if !known.contains(option.as_str())
            && !(page.module == Module::Home && matches!(option.as_str(), "today" | "stats"))
            && !(page.module == Module::Schedule
                && matches!(
                    option.as_str(),
                    "today" | "overdue" | "upcoming" | "projects" | "habits"
                ))
            && !(page.module == Module::Knowledge && option == "recent")
            && !(page.module == Module::Document
                && matches!(
                    option.as_str(),
                    "title" | "headings" | "excerpt" | "wordCount"
                ))
        {
            bail!(
                "分页 {} 的选项 {} 不属于模块 {}",
                page.id,
                option,
                page.module.label()
            );
        }
        if !options.insert(option.as_str()) {
            bail!("分页 {} 的选项重复: {}", page.id, option);
        }
    }
    if let Some(source) = &page.source {
        validate_relative_path(source, "分页来源")?;
    }
    Ok(())
}

fn validate_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || value.chars().count() > MAX_ID_CHARS {
        bail!("{label}为空或过长");
    }
    if value
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || b"_-.:".contains(&byte)))
    {
        bail!("{label}含有非法字符");
    }
    Ok(())
}

fn validate_title(value: &str, label: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_TITLE_CHARS {
        bail!("{label}为空或过长");
    }
    if value.chars().any(|ch| ch == '\0' || ch.is_control()) {
        bail!("{label}含有控制字符");
    }
    Ok(())
}

pub(crate) fn validate_relative_path(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || value.chars().count() > MAX_SOURCE_CHARS {
        bail!("{label}为空或过长");
    }
    if value.contains('\0') || value.contains('\\') || value.starts_with('/') {
        bail!("{label}必须是工作区内的正斜杠相对路径");
    }
    let path = Path::new(value);
    if path.is_absolute() {
        bail!("{label}不能是绝对路径");
    }
    let mut count = 0;
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let text = part.to_string_lossy();
                if text.is_empty() || text == "." || text == ".." || text.contains(':') {
                    bail!("{label}含有非法路径段");
                }
                count += 1;
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => bail!("{label}含有非法路径段"),
        }
    }
    if count == 0 {
        bail!("{label}不能为空");
    }
    Ok(())
}

/// 解析配置的来源之前，先做词法和工作区规范路径两层包含检查。
/// 指向工作区外的符号链接一律拒绝，Windows 的大小写不敏感文件系统也不例外。
pub fn safe_source_path(workspace: impl AsRef<Path>, source: &str) -> Result<PathBuf> {
    validate_relative_path(source, "分页来源")?;
    let workspace = workspace.as_ref();
    let root = workspace
        .canonicalize()
        .with_context(|| format!("工作区不存在或无法读取: {}", workspace.display()))?;
    let candidate = workspace.join(source.replace('/', std::path::MAIN_SEPARATOR_STR));
    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("分页来源不存在或无法读取: {}", source))?;
    if !paths::path_is_within(&root, &canonical) {
        bail!("分页来源越出工作区: {source}");
    }
    Ok(canonical)
}

fn regenerate_ids(config: &mut DesktopConfig) {
    for card in &mut config.cards {
        card.id = new_id("card");
        let old_active = card.active_page.clone();
        let mut active = None;
        let mut page_ids = std::collections::BTreeMap::new();
        for page in &mut card.pages {
            let old_id = page.id.clone();
            page.id = new_id("page");
            page_ids.insert(old_id.clone(), page.id.clone());
            if old_id == old_active {
                active = Some(page.id.clone());
            }
        }
        remap_page_events(card, &page_ids);
        card.active_page = active
            .or_else(|| card.pages.first().map(|page| page.id.clone()))
            .unwrap_or_default();
    }
}

pub fn remap_page_events(card: &mut Card, ids: &BTreeMap<String, String>) {
    for page in &mut card.pages {
        for node in &mut page.studio.nodes {
            for event in &mut node.events {
                if event.action == studio::Action::SwitchPage {
                    if let Some(target) = ids.get(&event.target) {
                        event.target = target.clone();
                    }
                }
            }
        }
    }
}

fn new_id(prefix: &str) -> String {
    let sequence = ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "{prefix}-{}-{sequence}-{}",
        jstime::now_millis(),
        paths::random_base36(8)
    )
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("桌面卡片配置文件名无效"))?;
    let temporary = path.with_file_name(format!(
        ".{file_name}.tmp-{}-{sequence}",
        std::process::id()
    ));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("创建桌面卡片临时文件失败: {}", temporary.display()))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Err(error) = fs::rename(&temporary, path) {
            if cfg!(windows) && path.exists() {
                fs::remove_file(path)?;
                fs::rename(&temporary, path)?;
            } else {
                return Err(error.into());
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn appearance_defaults_and_customization_roundtrip() {
        let old: super::Appearance =
            serde_json::from_str(r#"{"opacity":85,"fontSize":16,"fontColor":null}"#).unwrap();
        assert_eq!(old.font_size, 16);
        assert_eq!(old.background_color, Some(super::GLASS_BACKGROUND));
        let explicit: super::Appearance =
            serde_json::from_str(r#"{"backgroundColor":null}"#).unwrap();
        assert_eq!(explicit.background_color, None);
        assert!(!old.show_border);
        let mut config = super::DesktopConfig::default();
        let mut card = super::Card::new("test", super::Module::Inbox);
        card.appearance.background_color = Some(0x152c42);
        card.appearance.spacing = 2;
        card.appearance.show_details = true;
        config.cards.push(card);
        for size in 8..=72 {
            config.cards[0].appearance.font_size = size;
            config.validate().unwrap();
        }
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(
            serde_json::from_str::<super::DesktopConfig>(&json).unwrap(),
            config
        );
        config.cards[0].appearance.background_color = Some(0x1000000);
        assert!(config.validate().is_err());
        config.cards[0].appearance.background_color = None;
        config.cards[0].appearance.spacing = 3;
        assert!(config.validate().is_err());
    }

    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    fn workspace(tag: &str) -> PathBuf {
        let n = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("mochi-desktop-cards-{tag}-{n}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn config_round_trips_and_missing_is_empty() {
        let root = workspace("roundtrip");
        let mut config = DesktopConfig::new();
        let mut card = Card::new("今日", Module::Schedule);
        card.pages.push(Page::new(Module::Inbox));
        card.active_page = card.pages[1].id.clone();
        config.cards.push(card);
        config.save(&root).unwrap();
        let loaded = DesktopConfig::load(&root).unwrap();
        assert_eq!(config, loaded);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_file_is_preserved_as_an_error() {
        let root = workspace("corrupt");
        let path = DesktopConfig::path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{broken").unwrap();
        assert!(DesktopConfig::load(&root).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "{broken");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn import_disables_cards_and_regenerates_ids_on_merge() {
        let mut source = DesktopConfig::new();
        source.cards.push(Card::new("源卡片", Module::Home));
        let text = source.export_json().unwrap();
        let imported = DesktopConfig::import_json(&text).unwrap();
        assert!(!imported.cards[0].enabled);
        let old_id = source.cards[0].id.clone();
        let mut target = DesktopConfig::new();
        let ids = target.merge_import(&text).unwrap();
        assert_eq!(ids.len(), 1);
        assert_ne!(ids[0], old_id);
        assert!(!target.cards[0].enabled);
    }

    #[test]
    fn import_rejects_path_traversal_and_unsupported_versions() {
        let bad_path = serde_json::json!({
            "version": 1,
            "cards": [{
                "id": "card-a", "title": "a", "enabled": true, "locked": false,
                "x": 0, "y": 0, "width": 320, "height": 220,
                "activePage": "page-a",
                "pages": [{"id":"page-a","title":"a","module":"home","options":["today"],"source":"../outside","limit":8}]
            }]
        });
        assert!(DesktopConfig::import_json(&bad_path.to_string()).is_err());
        let bad_version = serde_json::json!({"version": 99, "cards": []});
        assert!(DesktopConfig::import_json(&bad_version.to_string()).is_err());
    }

    #[test]
    fn action_round_trip_keeps_idempotent_task_state() {
        let action = Action::ToggleTask {
            id: "task-1".into(),
            checked: true,
        };
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(
            json,
            r#"{"type":"toggleTask","value":{"id":"task-1","checked":true}}"#
        );
        let decoded: Action = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, action);
    }
}

#[cfg(test)]
mod interactive_tests {
    use super::*;
    #[test]
    fn desktop_import_preserves_cross_page_events_with_new_ids() {
        let mut cfg = DesktopConfig::default();
        let mut card = Card::new("联动", Module::Custom);
        let other = Page::new(Module::Clock);
        let mut n = studio::Node::new(studio::Kind::Button);
        n.events = vec![studio::Binding {
            action: studio::Action::SwitchPage,
            target: other.id.clone(),
            ..Default::default()
        }];
        card.pages[0].studio.nodes.push(n);
        card.pages.push(other);
        cfg.cards.push(card);
        let mut imported = DesktopConfig::default();
        imported
            .merge_import(&serde_json::to_string(&cfg).unwrap())
            .unwrap();
        let c = &imported.cards[0];
        assert_ne!(c.pages[1].id, cfg.cards[0].pages[1].id);
        assert_eq!(c.pages[0].studio.nodes[0].events[0].target, c.pages[1].id);
        assert!(!c.enabled);
    }
    #[test]
    fn desktop_document_templates_include_nested_files_and_data_components() {
        let root = std::env::temp_dir().join(new_id("mochi-desktop-docs"));
        std::fs::create_dir_all(root.join("知识库/个人/项目/资料")).unwrap();
        for ext in ["mcb", "mcanvas", "exam"] {
            std::fs::write(
                root.join(format!("知识库/个人/项目/资料/示例.{ext}")),
                "document placeholder",
            )
            .unwrap();
        }
        let mut cfg = DesktopConfig::default();
        for module in [Module::Base, Module::Canvas, Module::Exam] {
            cfg.cards.push(Card::new(module.label(), module));
        }
        let mut custom = Card::new("列表", Module::Custom);
        let mut node = studio::Node::new(studio::Kind::Data);
        node.target = "exam".into();
        custom.pages[0].studio.nodes.push(node.clone());
        cfg.cards.push(custom);
        let snap = SnapshotBuilder::new(&root).build(&cfg).unwrap();
        for c in cfg.cards.iter().take(3) {
            let page = &c.pages[0];
            let rows = &snap.pages[&page_key(&c.id, &page.id)].rows;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].title, "示例");
            assert!(
                matches!(&rows[0].action,Some(Action::OpenFile(path)) if path.contains("项目/资料"))
            );
        }
        let c = &cfg.cards[3];
        assert_eq!(
            snap.pages[&format!("{}:node:{}", page_key(&c.id, &c.pages[0].id), node.id)]
                .rows
                .len(),
            1
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn desktop_shortcut_auto_layout_scrolls_without_stacking() {
        let mut s = studio::Studio::default();
        s.add_shortcuts(
            &(0..40)
                .map(|i| format!("C:/Desktop/{i}.lnk"))
                .collect::<Vec<_>>(),
        );
        s.arrange_shortcuts();
        assert_eq!(s.height, 250);
        let mut positions = std::collections::HashSet::new();
        assert!(s.nodes.iter().all(|n| positions.insert((n.x, n.y))));
        s.validate().unwrap();
    }
}
