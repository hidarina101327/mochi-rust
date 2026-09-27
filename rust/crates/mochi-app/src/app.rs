//! 字符、按键和 IME 输入共用 Focus 路由。
mod capture_hub;

mod agenda_page;
mod agenda_panel;
mod agenda_review;
mod agenda_tick;
#[cfg(debug_assertions)]
mod agenda_verification;
mod agent_configuration;
mod ai_conversations;
mod ai_execution;
mod ai_float;
mod ai_panel_actions;
mod ai_titles;
mod canvas_editor;
mod command_palette;
mod console_agent;
mod context_menus;
mod creation_dialogs;
mod desktop_cards;
mod dialog_dispatch;
mod edit_keys;
mod editor_find;
mod editor_formatting;
mod editor_lifecycle;
mod english_lab;
mod exam_editor;
mod external_drop;
mod file_jobs;
mod frame;
mod global_import;
mod inbox_page;
mod input_focus;
mod keyboard_shortcuts;
mod link_navigation;
mod main_content;
mod mapped_folder_dialog;
pub(crate) mod marketplace;
mod menu_dispatch;
mod navigation_actions;
mod navigation_drag;
mod notifications;
mod pdf_editor;
mod plugin_pages;
mod pointer_click;
mod pointer_cursor;
mod pointer_motion;
mod pointer_wheel;
mod provider_settings;
mod quick_navigation;
mod quick_navigation_edit;
mod right_panel;
mod right_panel_actions;

mod settings_actions;
mod settings_overlay;
mod settings_page;
mod settings_state;
mod sidebar_chrome;
mod sidebar_interactions;
mod state;
mod tab_routing;
mod table_cell_editor;
mod template_pages;
mod text_input;
mod timers;
mod viewer_interactions;
mod window_lifecycle;
mod window_motion;
mod workflows;
mod workspace_lifecycle;
mod workspace_search;

pub use state::CaptionAction;
use state::{
    deleted_versions_key, launch_installer_after_exit, pending_edits_from_transcript, shift_down,
    update_notes_for_dialog, uuid_v4, AgentConfigState, AgentSourceEditor, AiFloat, AiFloatDrag,
    AiState, CreatingLibrary, DialogAction, Drag, DragTarget, EnglishLabHit, EnglishLabPage, Focus,
    MenuAction, NavState, PanelsState, QuickNavHit, ScheduleState, SearchJob, SettingsState,
    SidebarState, SidebarTreeDrag, SidebarTreeDrop, SidebarTreeDropKind, TextSaveActivity,
    ToolbarMenu, ViewsState, WorkspaceIndexEvent, SETTINGS_LAST_SECTION_KEY, SETTINGS_LAST_TAB_KEY,
    SETTINGS_SCROLL_KEY, SIDEBAR_MAX, SIDEBAR_MIN, TREE_DRAG_THRESHOLD,
};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};

use mochi_core::ai::agent_inbox::AgentInboxService;
use mochi_core::ai::document_mounts::AiDocumentMountService;
use mochi_core::ai::models::AiMessage;
use mochi_core::ai::permission::AiPermissionService;
use mochi_core::ai::session::{
    self as ai_session, AiConversation, AiSessionMeta, AiSessionService, AiStoredMessage,
};
use mochi_core::ai::tools::host::ActiveDocument;
use mochi_core::analytics::HomeAnalytics;
use mochi_core::app_settings::{self, AppSettings, SettingValue, SettingValueType};
use mochi_core::capture::CaptureService;
use mochi_core::search::{SearchQueryResult, SearchService};
use mochi_core::settings::SettingsService;
use mochi_core::sidecars;
use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{DestroyWindow, IsZoomed, PostMessageW};

use mochi_core::mochi_url::{self, build_mochi_resource_url, ResourceKind};

use crate::ai_runtime::{self, AppHost, HostSnapshot, RunEvent, RunHandle};
use crate::gfx::{self, Renderer};
use crate::platform;
use crate::shell::{NavigationDirection, Shell, TabKind, TreeDropPosition};
use crate::ui::agenda;
use crate::ui::agent_config;
use crate::ui::ai_workspace;
use crate::ui::assistant;
use crate::ui::base_view;
use crate::ui::canvas_view;
use crate::ui::chrome::{Chrome, ChromeState, RightPanel, WorkspaceView};
use crate::ui::code_blocks;
use crate::ui::command::{self, CommandAction};
use crate::ui::draw::{Align, DrawList, TextStyle};
use crate::ui::exam_view;
use crate::ui::findbar;
use crate::ui::format::{self, Format};
use crate::ui::icons::Icon;
use crate::ui::layout::{NodeKey, Rect};
use crate::ui::link_create;
use crate::ui::mapped_folder;
use crate::ui::navigation::{self, NavHit, NavItem, NavLayout, NavModel};
use crate::ui::panels::{comments, inbox, mounts, pomodoro, version};
use crate::ui::pdf_annotations;
use crate::ui::providers;
use crate::ui::search::{self, RowKind, SearchHit, SearchLayout, SearchState};
use crate::ui::settings::{
    self, ContentLayout as SettingsContentLayout, EditingField, NavLayout as SettingsNavLayout,
};
use crate::ui::sheet_view;
use crate::ui::sidebar::{
    self, EditKind, Editing, SidebarHit, SidebarLayout, SidebarModel, TreeDropIndicator,
};
use crate::ui::table_edit;
use crate::ui::theme::Palette;
use crate::ui::toolbar;
use crate::ui::viewer;
use crate::ui::views;
use crate::ui::widgets::{
    ButtonKind, Dialog, DialogButton, DialogHit, FieldKey, Menu, MenuItem, TextField,
};
use crate::ui::{backlinks, outline, tab_bar, theme};

mod ai_context;
mod ai_controls;
mod ai_follow_up;
mod ai_images;
mod ai_interactions;
mod ai_locator;
mod ai_messages;
#[cfg(debug_assertions)]
mod ai_parity_verification;
mod ai_scroll;
mod ai_search;
mod ai_sessions;
mod base;
mod base_automation;
mod base_export;
#[cfg(debug_assertions)]
mod base_verification;
mod clipboard_edit;
mod commands;
mod document_blocks;
mod document_chunks;
mod document_mode;
mod document_unread;
mod editor_ai;
#[cfg(debug_assertions)]
mod editor_benchmark;
mod editor_blocks;
mod editor_comments;
mod exporting;
mod favorites;
#[cfg(debug_assertions)]
mod feature_verification;
mod file_approvals;
mod home_dashboard;
#[cfg(debug_assertions)]
mod home_favorites_verification;
mod memory;
#[cfg(test)]
mod menu_routing_tests;
mod navigation_preferences;
mod object_links;
mod object_picker_host;
#[cfg(test)]
mod provider_tests;
#[cfg(debug_assertions)]
mod quick_note;
mod remote_images;
mod resource_usage;
mod save_conflicts;
mod scrollbars;
#[cfg(test)]
mod shared_settings_tests;
#[cfg(all(test, debug_assertions))]
mod sidebar_drag;
#[cfg(debug_assertions)]
mod snapshot;
mod split;
#[cfg(debug_assertions)]
mod split_verification;
mod status;
#[cfg(all(test, debug_assertions))]
mod tab_navigation;
mod title;
#[cfg(test)]
mod ux_tests;
#[cfg(debug_assertions)]
mod verification;
mod wiki_suggestions;
use crate::ui::widgets::FieldLook;
use crate::view::{DocPane, HomePane, MainContent, SourcePane, TabFacts};

pub struct App {
    console_jobs: console_agent::SystemState,
    web_clipper: Arc<mochi_core::web_clipper::Receiver>,
    desktop: desktop_cards::State,
    marketplace: marketplace::State,
    workflows: workflows::State,
    notifications: notifications::State,
    automation: base_automation::State,
    base_export: base_export::State,
    settings_overlay: Option<(String, String)>,
    object_picker: Option<object_picker_host::DialogState>,
    object_picker_cache: object_picker_host::Cache,
    desktop_source_cache: object_picker_host::Cache,
    global_import: Option<global_import::Pending>,
    open_document_count: usize,
    tab_scroll: f32,
    last_active_tab: Option<(usize, String)>,
    file_icons: HashMap<PathBuf, String>,
    capture: Option<crate::capture_window::Handle>,
    editor_ai: editor_ai::State,
    split: split::State,
    file_jobs: crate::file_runtime::Jobs,
    remote_images: remote_images::State,
    ai_requests: std::collections::VecDeque<std::time::Instant>,
    renderer: Renderer,
    shell: Shell,
    settings: Arc<SettingsService>,
    settings_revision: u64,
    /// 按描述表读写设置。与 `settings` 共享同一个存储。
    app_settings: AppSettings,
    prefs: SettingsState,
    window_motion: window_motion::State,
    panels: PanelsState,
    sched: ScheduleState,
    agent: AgentConfigState,
    views: ViewsState,
    /// 设置页中当前预览的声明式原生插件。
    plugin_preview: Option<String>,
    /// 当前独立插件工作区的插件 ID。
    plugin_workspace: Option<String>,
    /// 右侧栏当前选中的第三方贡献，按 (插件 ID, 贡献标题) 稳定定位。
    plugin_right_panel: Option<(String, String)>,
    plugin_right_area: Rect,
    /// 快捷导航原生视图上一帧的命中区域。这里不走通用声明式卡片，
    /// 因为它的条目、分组与启动操作都必须真实落盘并可点击。
    quick_nav_hits: Vec<(Rect, QuickNavHit)>,
    /// 新建快捷导航分组时的父级；由当前选中的树节点提供，匹配 Electron 侧栏的“新建子分组”。
    quick_nav_group_parent: Option<String>,
    english_lab_hits: Vec<(Rect, EnglishLabHit)>,
    english_lab_page: EnglishLabPage,
    english_lab_article: Option<i64>,
    english_lab_practice_group: Option<i64>,
    ai: AiState,
    state: ChromeState,
    page_history: Vec<WorkspaceView>,
    /// 复用同一个指令列表，避免每帧重新分配。
    list: DrawList,

    nav: NavState,
    /// 上一帧的导航轨布局。点击要与绘制用同一份，否则「画在这儿、点在那儿」。
    nav_layout: NavLayout,
    links: backlinks::State,
    outline_area: Rect,
    /// 文件区内大纲标题旁的显示/隐藏按钮；隐藏时仍停在文件区右上角以便恢复。
    outline_toggle_rect: Rect,
    outline_scroll: usize,
    scrollbars: scrollbars::State,
    outline_left_active: bool,
    outline_tabs: Option<(Rect, Rect)>,
    links_layout: backlinks::Layout,
    links_job: crate::backlinks_runtime::Job,
    focus: Focus,
    input_feedback: crate::ui::input_feedback::CaretFeedback,
    side: SidebarState,
    /// 打开着的右键菜单。最多一个。
    menu: Option<Menu<MenuAction>>,
    /// 当前正在展开嵌套子菜单的菜单层级和项目索引。
    menu_submenu_target: Option<(usize, usize)>,
    /// `/` 菜单触发符的位置。只有选中命令才删除；Esc/点空白会保留。
    slash_trigger: Option<(PathBuf, usize)>,
    /// 打开着的模态对话框。最多一个；开着时吞掉其它全部输入。
    dialog: Option<Dialog<DialogAction>>,
    /// GitHub Releases 更新检查只在后台线程执行，结果通过 Win32 自定义消息回 UI 线程。
    update_rx:
        Option<Receiver<std::result::Result<Option<mochi_core::updates::UpdateInfo>, String>>>,
    /// 安装包下载与 SHA-256 校验在另一条后台线程完成。
    update_download_rx: Option<Receiver<std::result::Result<PathBuf, String>>>,
    update_check_manual: bool,
    update_available: Option<mochi_core::updates::UpdateInfo>,
    /// 目录树的新建链接弹窗需要“显示名称 + 链接”两个输入框。
    link_create: Option<link_create::State>,
    /// 外部目录映射使用固定高度表单，避免高级设置展开时窗口跳动。
    mapped_folder: Option<mapped_folder::State>,
    pending_ai_delete: Option<mochi_core::ai::message_ops::Deletion>,
    export_form: Option<crate::ui::export_dialog::State>,
    /// 新建文档时选择空白或工作区模板。
    template_picker: Option<crate::ui::template_picker::Picker>,
    /// 全局搜索覆盖层。`None` 表示关着。
    search: Option<SearchState>,
    search_layout: SearchLayout,
    search_job: SearchJob,
    workspace_index_generation: u64,
    workspace_index_rx: Option<Receiver<(u64, WorkspaceIndexEvent)>>,
    /// 命令面板 / 快速打开（同一张卡片，两种模式）。
    command: Option<command::State>,
    command_layout: command::Layout,
    /// 文件查看器上一帧的排布与悬停。
    viewer_layout: viewer::Layout,
    viewer_hover: Option<viewer::Hit>,
    pdf_text_pending: Option<(PathBuf, sidecars::PdfAnnotation)>,
    pdf_jobs: HashMap<PathBuf, (crate::pdf::PdfHandle, Receiver<crate::pdf::PdfEvent>)>,
    /// 搜索历史。覆盖层关掉也要留着（Electron 版持久化在 localStorage）。
    search_history: Vec<String>,

    home: HomePane,
    doc: DocPane,
    table_editing: Option<table_edit::Editing>,
    table_picker: Option<crate::ui::table_picker::Picker>,
    block_pointer: Option<(f32, f32)>,
    chunk_edge_until: Option<std::time::Instant>,
    image_drag: Option<editor_blocks::ImageDrag>,
    image_preview: Option<String>,
    comment_bubbles: Vec<(Rect, String)>,
    comment_ranges: editor_comments::Cache,
    table_drag: Option<(usize, Rect, f32, f32)>,
    table_column_drag: Option<editor_blocks::TableColumnDrag>,
    wiki_suggestion: Option<wiki_suggestions::Suggestion>,
    wiki_dismissed: Option<(PathBuf, usize, String)>,
    source: SourcePane,
    /// 后台线程算完首页快照后从这里取。
    home_rx: Option<Receiver<HomeAnalytics>>,
    home_dirty: bool,

    /// 上一帧解算出的侧栏内容区，滚动夹取要用。
    tree_area: Rect,
    /// 上一帧解算出的编辑器内容区。滚轮与滚动夹取都要它。
    editor_area: Rect,
    /// 上一帧的工具栏矩形（编辑文件时才非空）。
    toolbar_area: Rect,
    /// 文档内查找条（Ctrl+F / Ctrl+H）。
    find: Option<findbar::State>,
    find_layout: findbar::Layout,
    /// 表格查找的匹配（表、记录、字段下标），与查找条计数器对齐。
    base_find_hits: Vec<(usize, usize, usize)>,
    /// 全局 Alt+Left 快捷键使用的文件级导航栈。
    toolbar_hover: Option<toolbar::Hit>,
    toolbar_menu: Option<ToolbarMenu>,
    /// 用户是否点进过编辑器（或在里面敲过字）。`Focus::Main` 是默认焦点，
    /// 不能拿它判断「正在编辑」——刚打开的文件不该露出第一块的源码、也不该闪光标。
    editor_engaged: bool,
    /// 自动保存：有编辑没落盘时挂一个定时器，到点写盘（Electron 版编辑后延时保存）。
    autosave_pending: bool,
    /// 待窗口过程领走的自动保存定时请求（毫秒）。
    autosave_request: Option<u32>,

    /// 首帧是否已经按 DPI 调整过窗口大小。见 `platform::scale_window_to_dpi`。
    initial_sized: bool,
    /// 窗口句柄的裸值。后台线程要往 UI 线程投消息时用它重建 `HWND`。
    /// 在 `WM_CREATE` 里记下；之前是 0。
    hwnd_raw: isize,
    drag: Option<Drag>,
    /// 本次点击命中了哪个窗口按钮，在窗口过程里消费。
    caption_action: Option<CaptionAction>,
    /// 指针是否悬在某个窗口按钮上。悬停高亮是这三个按钮唯一的可发现性来源。
    caption_hover: Option<usize>,
    status_bar: status::State,
    commands: commands::State,
    high_surrogate: Option<(u16, Focus, Option<PathBuf>)>,
    title_editing: Option<title::Editing>,
    background: Option<(String, (u32, u32))>,
    suppress_alt_char: bool,
}

fn group_path_depth(groups: &[mochi_core::quick_navigation::Group], id: &str) -> usize {
    let mut depth = 0;
    let mut current = groups.iter().find(|group| group.id == id);
    while let Some(group) = current {
        let Some(parent) = group.parent_id.as_deref() else {
            break;
        };
        depth += 1;
        current = groups.iter().find(|candidate| candidate.id == parent);
    }
    depth
}

/// Electron 快捷导航允许每个入口保存 6 位背景色。只接受这个受控格式，避免
/// 损坏的旧 data.json 把绘制颜色解释成透明或越界值。
fn quick_nav_color(value: &str) -> Option<u32> {
    let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
    (value.len() == 6)
        .then(|| u32::from_str_radix(value, 16).ok())
        .flatten()
}

fn quick_nav_icon(item: &mochi_core::quick_navigation::Item) -> &'static str {
    if item.custom_icon.is_some() {
        return "◆";
    }
    match item.kind.as_str() {
        "web" => "◎",
        "application" => "▣",
        "folder" => "▰",
        "file" => "◇",
        _ => "◇",
    }
}

fn quick_nav_visible_items(
    model: &mochi_core::quick_navigation::Model,
) -> Vec<mochi_core::quick_navigation::Item> {
    let selected = model.ui.selected.as_str();
    let mut rows: Vec<_> = match selected {
        "favorites" => model
            .items
            .iter()
            .filter(|item| item.favorite)
            .cloned()
            .collect(),
        "recent" => model
            .items
            .iter()
            .filter(|item| item.open_count > 0)
            .cloned()
            .collect(),
        "all" | "settings" => model.items.clone(),
        group_id => {
            let mut ids = vec![group_id.to_owned()];
            let mut cursor = 0;
            while cursor < ids.len() {
                let parent = ids[cursor].clone();
                ids.extend(
                    model
                        .groups
                        .iter()
                        .filter(|group| group.parent_id.as_deref() == Some(&parent))
                        .map(|group| group.id.clone()),
                );
                cursor += 1;
            }
            model
                .items
                .iter()
                .filter(|item| item.group_id.as_ref().is_some_and(|id| ids.contains(id)))
                .cloned()
                .collect()
        }
    };
    match model.settings.default_sort.as_str() {
        "recent" => rows.sort_by(|a, b| b.last_opened_at.cmp(&a.last_opened_at)),
        "usage" => rows.sort_by(|a, b| {
            b.open_count
                .cmp(&a.open_count)
                .then_with(|| a.name.cmp(&b.name))
        }),
        "created" => rows.sort_by(|a, b| a.created_at.cmp(&b.created_at)),
        "name" => rows.sort_by(|a, b| a.name.cmp(&b.name)),
        _ => rows.sort_by_key(|item| item.order),
    }
    rows
}

impl App {
    pub fn new() -> Result<Self> {
        Self::with_settings(Arc::new(SettingsService::new(
            std::env::var_os("MOCHI_SETTINGS_FILE")
                .or_else(|| std::env::var_os("MOCHI_SETTINGS_PATH"))
                .map(PathBuf::from),
        )))
    }

    fn with_settings(settings: Arc<SettingsService>) -> Result<Self> {
        crate::ui::editor_preferences::set(Default::default());
        crate::ui::settings_values::reset();
        crate::ui::shortcuts::load(None);
        let search_history: Vec<String> = settings
            .get("search.history")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Ok(Self {
            console_jobs: Default::default(),
            web_clipper: Arc::new(mochi_core::web_clipper::Receiver::new(Arc::clone(
                &settings,
            ))),
            desktop: desktop_cards::State::default(),
            marketplace: marketplace::State::default(),
            notifications: notifications::State::new(&settings),
            workflows: workflows::State::default(),
            automation: base_automation::State::default(),
            base_export: base_export::State::default(),
            settings_overlay: None,
            object_picker: None,
            object_picker_cache: object_picker_host::Cache::default(),
            desktop_source_cache: object_picker_host::Cache::default(),
            global_import: None,
            open_document_count: 0,
            tab_scroll: 0.0,
            last_active_tab: None,
            file_icons: HashMap::new(),
            capture: None,
            editor_ai: editor_ai::State::default(),
            split: split::State::default(),
            file_jobs: crate::file_runtime::Jobs::default(),
            remote_images: remote_images::State::default(),
            ai_requests: Default::default(),
            renderer: Renderer::new()?,
            shell: Shell::new(),
            app_settings: AppSettings::new(Arc::clone(&settings)),
            prefs: SettingsState::default(),
            window_motion: Default::default(),
            panels: PanelsState::default(),
            sched: ScheduleState::default(),
            agent: AgentConfigState::default(),
            views: ViewsState::default(),
            plugin_preview: None,
            plugin_workspace: None,
            plugin_right_panel: None,
            plugin_right_area: Rect::ZERO,
            quick_nav_hits: Vec::new(),
            quick_nav_group_parent: None,
            english_lab_hits: Vec::new(),
            english_lab_page: EnglishLabPage::Dashboard,
            english_lab_article: None,
            english_lab_practice_group: None,
            ai: AiState::default(),
            settings_revision: settings.revision(),
            settings,
            page_history: Vec::new(),
            state: ChromeState {
                custom_status_bar: true,
                ..ChromeState::default()
            },
            list: DrawList::new(),
            nav: NavState::default(),
            nav_layout: NavLayout::default(),
            links: backlinks::State::default(),
            outline_area: Rect::ZERO,
            outline_toggle_rect: Rect::ZERO,
            outline_scroll: 0,
            scrollbars: scrollbars::State::default(),
            outline_left_active: false,
            outline_tabs: None,
            links_layout: backlinks::Layout::default(),
            links_job: crate::backlinks_runtime::Job::default(),
            focus: Focus::Main,
            input_feedback: crate::ui::input_feedback::CaretFeedback::default(),
            side: SidebarState::default(),
            menu: None,
            menu_submenu_target: None,
            slash_trigger: None,
            dialog: None,
            update_rx: None,
            update_download_rx: None,
            update_check_manual: false,
            update_available: None,
            link_create: None,
            mapped_folder: None,
            pending_ai_delete: None,
            export_form: None,
            template_picker: None,
            search: None,
            search_layout: SearchLayout::default(),
            search_job: SearchJob {
                generation: 0,
                rx: None,
                cancel: None,
                timer_pending: false,
            },
            workspace_index_generation: 0,
            workspace_index_rx: None,
            command: None,
            command_layout: command::Layout::default(),
            viewer_layout: viewer::Layout::default(),
            viewer_hover: None,
            pdf_text_pending: None,
            pdf_jobs: HashMap::new(),
            search_history,
            home: HomePane::default(),
            doc: DocPane::default(),
            table_editing: None,
            table_picker: None,
            block_pointer: None,
            chunk_edge_until: None,
            image_drag: None,
            image_preview: None,
            comment_bubbles: Vec::new(),
            comment_ranges: Default::default(),
            table_drag: None,
            table_column_drag: None,
            wiki_suggestion: None,
            wiki_dismissed: None,
            source: SourcePane::default(),
            home_rx: None,
            home_dirty: true,
            tree_area: Rect::ZERO,
            editor_area: Rect::ZERO,
            toolbar_area: Rect::ZERO,
            find: None,
            find_layout: findbar::Layout::default(),
            base_find_hits: Vec::new(),
            toolbar_hover: None,
            toolbar_menu: None,
            editor_engaged: false,
            autosave_pending: false,
            autosave_request: None,
            initial_sized: false,
            hwnd_raw: 0,
            drag: None,
            caption_action: None,
            caption_hover: None,
            status_bar: Default::default(),
            commands: Default::default(),
            high_surrogate: None,
            title_editing: None,
            background: None,
            suppress_alt_char: false,
        })
    }

    // ---------- 工作区 ----------

    // ---------- 派生状态 ----------

    // ---------- 绘制 ----------

    // ---------- 文件查看器 ----------

    // ---------- PDF 相关 ----------

    // ---------- Agent 配置页 ----------

    // ---------- AI 助手 ----------

    // ---------- 右侧面板的数据 ----------

    // ---------- 收件箱 / 最近 / 小记 ----------

    // ---------- 日程 ----------

    // ---------- 全局搜索 ----------

    // ---------- 全局快捷键 ----------

    // ---------- 点击 ----------

    // ---------- 设置页 ----------

    // ---------- 侧栏 ----------

    // ---------- 键盘与输入法 ----------

    // ---------- 命令面板 / 快速打开 ----------

    // ---------- 查找 / 替换条 ----------

    // ---------- 指针 ----------

    // ---------- 窗口 ----------
}

#[cfg(test)]
mod tests;
