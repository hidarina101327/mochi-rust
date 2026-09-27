//! 状态栏属于编辑器列，不横跨整个窗口。绘制和命中共用布局树。

use super::draw::{DrawList, TextStyle};
use super::layout::{Edges, NodeId, NodeKey, Position, Rect, Size, Style, Tree};
use super::theme::{self, Palette};

/// 可见分隔条按样式变量定义的宽度绘制，不可见的点击区域则向两侧加宽。
/// 外侧分隔条要与相邻面板有一点点重叠，这样指针恰好落在边界上时
/// 也能触发拖拽调宽。
const RESIZE_HIT_WIDTH: f32 = 8.0;

fn layout_setting(key: &str, fallback: f32, min: f32, max: f32) -> f32 {
    let value = super::settings_values::number(key, fallback);
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

/// The same configured shell dimensions feed both layout and native caption geometry.
/// Keep this as a fresh copy so changing a setting affects the next frame immediately.
pub fn configured_layout() -> theme::Layout {
    let mut layout = theme::tokens().layout;
    layout.title_bar_height =
        layout_setting("chrome.titleBarHeight", layout.title_bar_height, 24.0, 64.0);
    layout.title_bar_controls_width = layout_setting(
        "chrome.titleBarControlsWidth",
        layout.title_bar_controls_width,
        90.0,
        240.0,
    );
    layout.status_bar_height = layout_setting(
        "chrome.statusBarHeight",
        layout.status_bar_height,
        12.0,
        48.0,
    );
    layout.tab_bar_height = layout_setting("tabs.height", layout.tab_bar_height, 28.0, 60.0);
    layout.tab_bar_compact_height = layout_setting(
        "tabs.compactHeight",
        layout.tab_bar_compact_height,
        24.0,
        56.0,
    );
    layout.right_sidebar_toolbar_height = layout_setting(
        "chrome.rightSidebarToolbarHeight",
        layout.right_sidebar_toolbar_height,
        32.0,
        72.0,
    );
    layout.resize_handle_width = layout_setting(
        "chrome.resizeHandleWidth",
        layout.resize_handle_width,
        1.0,
        12.0,
    );
    // 高度设置是首选值；放大文字后仍需留出完整的一行，原生标题栏命中也共用此尺寸。
    layout.title_bar_height = layout.title_bar_height.max(TextStyle::Title.line_height());
    layout.status_bar_height = layout
        .status_bar_height
        .max(TextStyle::Caption.line_height());
    layout.tab_bar_height = layout.tab_bar_height.max(TextStyle::Body.line_height());
    layout.tab_bar_compact_height = layout
        .tab_bar_compact_height
        .max(TextStyle::Body.line_height());
    layout
}

fn title_bar_leading_inset() -> f32 {
    layout_setting("chrome.titleBarLeadingInset", 16.0, 0.0, 80.0)
}

fn title_bar_auxiliary_inset() -> f32 {
    layout_setting("chrome.titleBarAuxiliaryInset", 48.0, 0.0, 120.0)
}

fn status_bar_padding_x() -> f32 {
    layout_setting("chrome.statusBarPaddingX", 16.0, 0.0, 64.0)
}

fn toolbar_horizontal_inset() -> f32 {
    layout_setting("chrome.rightToolbarHorizontalInset", 8.0, 0.0, 32.0)
        .max(RESIZE_HIT_WIDTH.max(configured_layout().resize_handle_width))
}

fn toolbar_button_gap() -> f32 {
    layout_setting("chrome.rightToolbarButtonGap", 4.0, 0.0, 20.0)
}

fn toolbar_button_size(toolbar_height: f32) -> f32 {
    layout_setting("chrome.rightToolbarButtonSize", 32.0, 16.0, 48.0).min(toolbar_height.max(0.0))
}

/// 右侧栏当前显示哪个面板。对应 `MainLayout.tsx` 的 `rightSidebarView`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightPanel {
    Outline,
    Assistant,
    Pomodoro,
    VersionHistory,
    DocumentMounts,
    Annotations,
    Comments,
    AgentInbox,
    /// 受控的第三方插件右侧栏容器；具体插件由 App 保存，避免把动态 ID
    /// 混进布局层的 Copy 状态。
    Plugin,
}

impl RightPanel {
    /// 面板文字标签；工具条图标由装配代码单独绘制。
    pub fn label(self) -> &'static str {
        match self {
            RightPanel::Outline => "大纲",
            RightPanel::Assistant => "AI 助手",
            RightPanel::Pomodoro => "番茄钟",
            RightPanel::VersionHistory => "版本历史",
            RightPanel::DocumentMounts => "文档挂载",
            RightPanel::Annotations => "PDF 标注",
            RightPanel::Comments => "评论",
            RightPanel::AgentInbox => "待批准操作",
            RightPanel::Plugin => "插件",
        }
    }
}

/// 主区当前是哪个视图。对应 `MainLayout.tsx` 的 `activeWorkspaceView`。
///
/// 这不只是"画什么内容"的区别：首页、最近、速记、收件箱这几个**独立视图**
/// 会把标签栏、状态栏和左侧栏一起藏掉，主区整块交给该视图。
/// 照着 TSX 的 `isStandaloneWorkspaceView` 与 `showWorkspaceLeftPanel` 抄的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceView {
    /// 编辑器视图（TSX 的 `tabs`）：标签栏 + 正文 + 状态栏，左侧栏在。
    Editor,
    /// 首页仪表盘：独占主区。
    Home,
    Recent,
    Templates,
    Marketplace,
    DesktopCards,
    /// 第三方插件的独立工作区；具体插件 ID 由 App 状态持有。
    Plugin,
    /// Agent 定义管理：独立工作区页面，保留其专属的左侧配置导航。
    AgentConfig,
    Automations,
    /// 日程：左侧栏换成日历侧栏（`ScheduleSidebar`）。
    Schedule,
    /// 墨池 AI 工作区：左侧栏换成会话列表（`MochiAISidebar`）。
    MochiAi,
    QuickNote,
    Inbox,
}

impl WorkspaceView {
    /// 独立视图整块接管主区（没有标签栏和状态栏）——TSX 的 `isStandaloneWorkspaceView`。
    pub fn is_auxiliary_page(self) -> bool {
        matches!(
            self,
            Self::Automations | Self::Marketplace | Self::Templates | Self::DesktopCards
        )
    }

    pub fn is_standalone(self) -> bool {
        self != WorkspaceView::Editor
    }

    /// 这几个视图连左侧栏也不显示——`showWorkspaceLeftPanel` 的否定条件。
    /// 注意日程、墨池 AI、Agent 配置 **不在**其中：它们把左侧栏换成自己的内容。
    pub fn hides_left_panel(self) -> bool {
        matches!(
            self,
            WorkspaceView::Home
                | WorkspaceView::Recent
                | WorkspaceView::Templates
                | WorkspaceView::Marketplace
                | WorkspaceView::DesktopCards
                | WorkspaceView::QuickNote
                | WorkspaceView::Inbox
                | WorkspaceView::Automations
        )
    }
}

/// Shell 和设置的状态快照，不反向持有服务。
#[derive(Debug, Clone)]
pub struct ChromeState {
    pub dark: bool,
    pub view: WorkspaceView,
    /// 标题栏显示的工作区名；`None` 时只显示"墨池"。
    pub workspace_name: Option<String>,
    pub navigation_width: f32,
    pub navigation_collapsed: bool,
    /// 对应 `showWorkspaceLeftPanel`：首页/最近/速记/收件箱视图下左面板整个不显示。
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
    pub ai_panel_open: bool,
    pub ai_panel_width: f32,
    /// 大纲被放到右侧栏时，即使 AI 面板关着右侧栏也要在。
    pub outline_in_ai_sidebar: bool,
    pub right_panel: RightPanel,
    /// PDF 标注按钮只在当前文件是 PDF 时出现。
    pub active_file_is_pdf: bool,
    pub compact_tab_bar: bool,
    pub status_text: String,
    pub custom_status_bar: bool,
}

impl Default for ChromeState {
    fn default() -> Self {
        let l = configured_layout();
        ChromeState {
            dark: false,
            view: WorkspaceView::Home,
            workspace_name: None,
            navigation_width: l.navigation_width,
            navigation_collapsed: false,
            sidebar_visible: true,
            sidebar_width: l.sidebar_width,
            ai_panel_open: false,
            ai_panel_width: l.ai_panel_width,
            outline_in_ai_sidebar: false,
            right_panel: RightPanel::Outline,
            active_file_is_pdf: false,
            compact_tab_bar: false,
            custom_status_bar: false,
            status_text: String::new(),
        }
    }
}

impl ChromeState {
    /// `showRightSidebar = aiPanelOpen || outlineInAISidebar`
    pub fn right_sidebar_visible(&self) -> bool {
        if self.view == WorkspaceView::MochiAi && self.right_panel == RightPanel::Assistant {
            return false;
        }
        self.ai_panel_open || self.outline_in_ai_sidebar
    }

    fn navigation_effective_width(&self) -> f32 {
        let l = configured_layout();
        if self.navigation_collapsed {
            l.navigation_collapsed_width
        } else {
            theme::tokens().clamps.navigation(self.navigation_width)
        }
    }

    fn tab_bar_height(&self) -> f32 {
        let l = configured_layout();
        if self.compact_tab_bar {
            l.tab_bar_compact_height
        } else {
            l.tab_bar_height
        }
    }
}

/// 搭好的外壳：一棵解算完的控件树 + 各区域的句柄。
pub struct Chrome {
    pub tree: Tree,
    pub title_bar: NodeId,
    pub ai_toggle: NodeId,
    pub theme_toggle: NodeId,
    pub back: NodeId,
    pub notifications_toggle: NodeId,
    pub desktop_toggle: NodeId,
    pub marketplace_toggle: NodeId,
    pub templates_toggle: NodeId,
    pub automations_toggle: NodeId,
    pub navigation: NodeId,
    /// 导航栏拖动手柄的布局节点；命中区域与可见宽度分别计算。
    #[allow(dead_code)]
    pub navigation_resize: NodeId,
    pub sidebar: NodeId,
    pub sidebar_resize: NodeId,
    pub editor_column: NodeId,
    pub tab_bar: NodeId,
    pub editor: NodeId,
    pub status_bar: NodeId,
    pub right_sidebar: NodeId,
    #[allow(dead_code)]
    pub right_sidebar_resize: NodeId,
    pub right_sidebar_toolbar: NodeId,
    /// 右侧栏内容区；其可见性参与工具条按钮集合的计算。
    #[allow(dead_code)]
    pub right_sidebar_body: NodeId,
    /// 三条外侧分隔条的命中测试矩形，加宽且不可见。
    /// 对应的树节点仍保留相同大小的可见矩形。
    navigation_resize_hit: Rect,
    sidebar_resize_hit: Rect,
    right_sidebar_resize_hit: Rect,
}

impl Chrome {
    /// 按状态搭树并解算到 `viewport`。
    pub fn build(state: &ChromeState, viewport: Rect) -> Chrome {
        let l = configured_layout();
        let handle_w = l.resize_handle_width;

        let mut tree = Tree::new(Style::column());

        // ── 标题栏 ──
        let title_bar = tree.add(
            NodeId::ROOT,
            NodeKey::TitleBar,
            Style::row()
                .h(Size::Fixed(l.title_bar_height))
                .padding(Edges::xy(title_bar_leading_inset(), 0.0)),
        );
        // 让开右侧 138px 的 Windows 窗口控制按钮区
        let ai_toggle = tree.add(
            title_bar,
            NodeKey::TitleBarAiToggle,
            Style::row()
                .w(Size::Fixed(l.title_bar_height))
                .h(Size::Fixed(l.title_bar_height))
                .overlay(Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(l.title_bar_controls_width),
                    bottom: None,
                }),
        );

        let notifications_toggle = tree.add(
            title_bar,
            NodeKey::TitleBarNotifications,
            Style::row()
                .w(Size::Fixed(l.title_bar_height))
                .h(Size::Fixed(l.title_bar_height))
                .overlay(Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(l.title_bar_controls_width + l.title_bar_height),
                    bottom: None,
                }),
        );

        let desktop_toggle = tree.add(
            title_bar,
            NodeKey::TitleBarDesktop,
            Style::row()
                .w(Size::Fixed(l.title_bar_height))
                .h(Size::Fixed(l.title_bar_height))
                .overlay(Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(l.title_bar_controls_width + l.title_bar_height * 2.0),
                    bottom: None,
                }),
        );

        let marketplace_toggle = tree.add(
            title_bar,
            NodeKey::TitleBarMarketplace,
            Style::row()
                .w(Size::Fixed(l.title_bar_height))
                .h(Size::Fixed(l.title_bar_height))
                .overlay(Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(l.title_bar_controls_width + l.title_bar_height * 3.0),
                    bottom: None,
                }),
        );

        let templates_toggle = tree.add(
            title_bar,
            NodeKey::TitleBarTemplates,
            Style::row()
                .w(Size::Fixed(l.title_bar_height))
                .h(Size::Fixed(l.title_bar_height))
                .overlay(Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(l.title_bar_controls_width + l.title_bar_height * 4.0),
                    bottom: None,
                }),
        );
        let automations_toggle = tree.add(
            title_bar,
            NodeKey::TitleBarAutomations,
            Style::row()
                .w(Size::Fixed(l.title_bar_height))
                .h(Size::Fixed(l.title_bar_height))
                .overlay(Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(l.title_bar_controls_width + l.title_bar_height * 5.0),
                    bottom: None,
                }),
        );

        let theme_toggle = tree.add(
            title_bar,
            NodeKey::TitleBarThemeToggle,
            Style::row()
                .w(Size::Fixed(l.title_bar_height))
                .h(Size::Fixed(l.title_bar_height))
                .overlay(Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(l.title_bar_controls_width),
                    bottom: None,
                }),
        );
        let back = tree.add(
            title_bar,
            NodeKey::TitleBarBack,
            Style::row()
                .w(Size::Fixed(l.title_bar_height))
                .h(Size::Fixed(l.title_bar_height))
                .overlay(Position::Overlay {
                    left: Some(8.0),
                    top: Some(0.0),
                    right: None,
                    bottom: None,
                }),
        );
        tree.set_hidden(back, !state.view.is_auxiliary_page());
        let mut right = l.title_bar_controls_width + l.title_bar_height;
        for (node, key) in [
            (ai_toggle, "navigation.titleBar.assistant"),
            (notifications_toggle, "navigation.titleBar.notifications"),
            (desktop_toggle, "navigation.titleBar.desktop"),
            (marketplace_toggle, "navigation.titleBar.marketplace"),
            (templates_toggle, "navigation.titleBar.templates"),
            (automations_toggle, "navigation.titleBar.automations"),
        ] {
            let visible = super::settings_values::boolean(key, true);
            tree.set_hidden(node, !visible);
            if visible {
                tree.style_mut(node).position = Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(right),
                    bottom: None,
                };
                right += l.title_bar_height;
            }
        }

        // ── 主体 ──
        let body = tree.add(NodeId::ROOT, NodeKey::None, Style::row());

        let navigation = tree.add(
            body,
            NodeKey::Navigation,
            Style::column().w(Size::Fixed(state.navigation_effective_width())),
        );
        let navigation_resize = tree.add(
            navigation,
            NodeKey::NavigationResize,
            Style::column()
                .w(Size::Fixed(handle_w))
                .overlay(Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(0.0),
                    bottom: Some(0.0),
                }),
        );
        // 折叠态没有拖动手柄（TSX 里是 `{!navigationCollapsed && <div/>}`）
        tree.set_hidden(navigation_resize, state.navigation_collapsed);

        let sidebar = tree.add(
            body,
            NodeKey::Sidebar,
            Style::column().w(Size::Fixed(state.sidebar_width)),
        );
        // Sidebar.tsx 自己的拖动手柄：`absolute right-0 w-1 bg-border`——
        // 它是**可见的** 4px 灰条，不像另外两条手柄那样透明
        let sidebar_resize = tree.add(
            sidebar,
            NodeKey::SidebarResize,
            Style::column()
                .w(Size::Fixed(handle_w))
                .overlay(Position::Overlay {
                    left: None,
                    top: Some(0.0),
                    right: Some(0.0),
                    bottom: Some(0.0),
                }),
        );
        // 首页这类独立视图不显示左侧栏；日程/墨池 AI 的左侧栏在，只是内容不同
        tree.set_hidden(
            sidebar,
            !state.sidebar_visible || state.view.hides_left_panel(),
        );

        // 编辑器列：唯一的 Grow。min_w 给 0——TSX 写的就是 `min-w-0`，
        // 窗口拖窄时该被挤扁的是它，不是侧栏。
        let editor_column = tree.add(
            body,
            NodeKey::EditorColumn,
            Style::column().w(Size::Grow(1.0)).min_w(0.0),
        );
        let tab_bar = tree.add(
            editor_column,
            NodeKey::TabBar,
            Style::row().h(Size::Fixed(state.tab_bar_height())),
        );
        let editor = tree.add(
            editor_column,
            NodeKey::Editor,
            Style::column().h(Size::Grow(1.0)),
        );
        // 独立视图整块接管主区：没有标签栏也没有状态栏
        tree.set_hidden(tab_bar, state.view.is_standalone());
        let status_bar = tree.add(
            editor_column,
            NodeKey::StatusBar,
            Style::row()
                .h(Size::Fixed(l.status_bar_height))
                .padding(Edges::xy(status_bar_padding_x(), 0.0)),
        );

        let right_sidebar = tree.add(
            body,
            NodeKey::RightSidebar,
            Style::column().w(Size::Fixed(
                theme::tokens()
                    .clamps
                    .ai_panel(state.ai_panel_width, viewport.width()),
            )),
        );
        tree.set_hidden(status_bar, state.view.is_standalone());
        tree.set_hidden(right_sidebar, !state.right_sidebar_visible());
        let right_sidebar_resize = tree.add(
            right_sidebar,
            NodeKey::RightSidebarResize,
            Style::column()
                .w(Size::Fixed(handle_w))
                .overlay(Position::Overlay {
                    left: Some(0.0),
                    top: Some(0.0),
                    right: None,
                    bottom: Some(0.0),
                }),
        );
        let right_sidebar_toolbar = tree.add(
            right_sidebar,
            NodeKey::RightSidebarToolbar,
            Style::row()
                .h(Size::Fixed(l.right_sidebar_toolbar_height))
                .padding(Edges::xy(toolbar_horizontal_inset(), 0.0))
                .gap(toolbar_button_gap()),
        );
        let right_sidebar_body = tree.add(
            right_sidebar,
            NodeKey::RightSidebarBody,
            Style::column().h(Size::Grow(1.0)),
        );

        tree.layout(viewport);

        // 可见的 4 DIP 分隔条与布局几何保持不变。导航栏和左侧栏手柄向各自
        // 所属面板内部加宽；右侧手柄则跨过编辑器/面板边界加宽——从编辑器
        // 一侧拖拽调宽时，指针自然会落在边界上。
        let resize_hit_width = RESIZE_HIT_WIDTH.max(handle_w);
        let navigation_resize_hit = if tree.rect(navigation_resize).is_empty() {
            Rect::ZERO
        } else {
            let r = tree.rect(navigation);
            Rect::new(
                (r.right - resize_hit_width).max(r.left),
                r.top,
                r.right,
                r.bottom,
            )
        };
        let sidebar_resize_hit = if tree.rect(sidebar_resize).is_empty() {
            Rect::ZERO
        } else {
            let r = tree.rect(sidebar);
            Rect::new(
                (r.right - resize_hit_width).max(r.left),
                r.top,
                r.right,
                r.bottom,
            )
        };
        let right_sidebar_resize_hit = if tree.rect(right_sidebar_resize).is_empty() {
            Rect::ZERO
        } else {
            let r = tree.rect(right_sidebar);
            Rect::new(
                r.left - resize_hit_width,
                r.top,
                (r.left + resize_hit_width).min(r.right),
                r.bottom,
            )
        };

        Chrome {
            tree,
            title_bar,
            ai_toggle,
            theme_toggle,
            back,
            notifications_toggle,
            desktop_toggle,
            marketplace_toggle,
            templates_toggle,
            automations_toggle,
            navigation,
            navigation_resize,
            sidebar,
            sidebar_resize,
            editor_column,
            tab_bar,
            editor,
            status_bar,
            right_sidebar,
            right_sidebar_resize,
            right_sidebar_toolbar,
            right_sidebar_body,
            navigation_resize_hit,
            sidebar_resize_hit,
            right_sidebar_resize_hit,
        }
    }

    /// 画外壳本身——底色、边框、标题栏文字、状态栏文字、右侧栏工具条。
    /// 各面板的内容由各自的模块往同一个 `DrawList` 里追加。
    pub fn paint(&self, state: &ChromeState, list: &mut DrawList) {
        let p = theme::configured_palette(state.dark);
        let rect = |id: NodeId| self.tree.rect(id);

        // 根底色。窗口被拖大的一瞬间，没被任何面板覆盖的区域会露出来，
        // 不铺底色就会看到上一帧的残留。
        list.rect(self.tree.rect(NodeId::ROOT), p.background);

        // ── 标题栏 ──
        let title = rect(self.title_bar);
        list.rect(title, p.surface);
        list.border_bottom(title, p.border);
        // TSX 的标题栏只有「墨池」两个字（text-sm font-medium），工作区名不在这里显示
        list.text(
            Rect::new(
                title.left
                    + if state.view.is_auxiliary_page() {
                        title_bar_auxiliary_inset()
                    } else {
                        title_bar_leading_inset()
                    },
                title.top,
                title.right,
                title.bottom,
            ),
            "墨池",
            TextStyle::Title,
            p.foreground,
        );

        list.icon_centered(
            rect(self.theme_toggle),
            if state.dark {
                super::icons::Icon::SUN
            } else {
                super::icons::Icon::MOON
            },
            18.0,
            p.muted,
        );
        if state.view.is_auxiliary_page() {
            list.icon_centered(
                rect(self.back),
                super::icons::Icon::ARROW_LEFT,
                18.0,
                p.foreground,
            );
        }

        // AI 开关：开着时是 accent 的 10% 底 + accent 前景，关着时透明 + muted 前景
        let toggle = rect(self.ai_toggle);
        let assistant_active = state.ai_panel_open && state.right_panel == RightPanel::Assistant;
        if assistant_active {
            list.rect(toggle, theme::mix(p.accent, p.surface, 0.10));
        }
        list.icon_centered(
            toggle,
            super::icons::Icon::BOT,
            18.0,
            if assistant_active { p.accent } else { p.muted },
        );
        list.icon_centered(
            rect(self.notifications_toggle),
            super::icons::Icon::BELL,
            18.0,
            p.muted,
        );

        for (node, icon, view) in [
            (
                self.automations_toggle,
                super::icons::Icon::GIT_BRANCH,
                WorkspaceView::Automations,
            ),
            (
                self.templates_toggle,
                super::icons::Icon::LAYOUT_GRID,
                WorkspaceView::Templates,
            ),
            (
                self.marketplace_toggle,
                super::icons::Icon::BLOCKS,
                WorkspaceView::Marketplace,
            ),
            (
                self.desktop_toggle,
                super::icons::Icon::MONITOR,
                WorkspaceView::DesktopCards,
            ),
        ] {
            let r = rect(node);
            let selected = state.view == view;
            if selected {
                list.rounded_rect(r.inset(Edges::xy(4.0, 4.0)), 6.0, p.surface_muted);
            }
            list.icon_centered(r, icon, 18.0, if selected { p.foreground } else { p.muted });
        }

        // ── 导航轨 ──
        let nav = rect(self.navigation);
        list.rect(nav, p.area_navigation_default);
        list.border_right(nav, p.border);

        // ── 侧栏 ──
        if !self.tree.is_hidden(self.sidebar) {
            let side = rect(self.sidebar);
            list.rect(side, p.area_sidebar_default);
            list.border_right(side, p.border);
            // 可见的 4px 拖动手柄（bg-border）
            list.rect(rect(self.sidebar_resize), p.border);
        }

        // ── 编辑器列 ──
        let col = rect(self.editor_column);
        list.rect(col, p.area_main_default);

        if !self.tree.is_hidden(self.tab_bar) {
            // 标签栏底色是 foreground 的 3% 叠在主区底色上（index.css 的 .mochi-tab-bar 规则）
            let tabs = rect(self.tab_bar);
            list.rect(tabs, theme::mix(p.foreground, p.area_main_default, 0.03));
            list.border_bottom(tabs, p.border);
        }

        if !self.tree.is_hidden(self.status_bar) {
            // 状态栏：border-t（不是 border-b）——它在列的最底下
            let status = rect(self.status_bar);
            list.hline(status.left, status.right, status.top, p.border);
            if !state.custom_status_bar {
                list.text(
                    status.inset(Edges::xy(status_bar_padding_x(), 0.0)),
                    state.status_text.clone(),
                    TextStyle::Caption,
                    p.muted,
                );
            }
        }

        // ── 右侧栏 ──
        if !self.tree.is_hidden(self.right_sidebar) {
            let panel = rect(self.right_sidebar);
            list.rect(panel, p.area_assistant_default);
            list.border_left(panel, p.border);

            let toolbar = rect(self.right_sidebar_toolbar);
            list.rect(toolbar, p.area_assistant_default);
            list.border_bottom(toolbar, p.border);
            self.paint_toolbar(state, &p, list);
        }
    }

    /// 右侧栏工具条上的按钮。可见集合随状态变化——
    /// 大纲按钮只在大纲被放进右侧栏时出现，PDF 标注只在当前文件是 PDF 时出现。
    fn paint_toolbar(&self, state: &ChromeState, p: &Palette, list: &mut DrawList) {
        let toolbar = self.tree.rect(self.right_sidebar_toolbar);
        let content = toolbar.inset(Edges::xy(toolbar_horizontal_inset(), 0.0));
        let button = toolbar_button_size(toolbar.height());
        let gap = toolbar_button_gap();
        let top = toolbar.top + (toolbar.height() - button) * 0.5;

        let mut x = content.left;
        for panel in self.toolbar_buttons(state) {
            let r = Rect::from_size(x, top, button, button);
            if r.right > content.right {
                break; // 装不下就不画，别画到工具条外面去
            }
            let active = panel == state.right_panel;
            if active {
                list.rect(r, theme::mix(p.accent, p.area_assistant_default, 0.10));
            }
            let icon = match panel {
                RightPanel::Outline => super::icons::Icon::LIST,
                RightPanel::Comments => super::icons::Icon::MESSAGE_SQUARE,
                RightPanel::Assistant => super::icons::Icon::SPARKLES,
                RightPanel::Pomodoro => super::icons::Icon::TIMER,
                RightPanel::DocumentMounts => super::icons::Icon::FILE_TEXT,
                RightPanel::Annotations => super::icons::Icon::PENCIL_LINE,
                RightPanel::VersionHistory => super::icons::Icon::HISTORY,
                RightPanel::AgentInbox => super::icons::Icon::INBOX,
                RightPanel::Plugin => super::icons::Icon::PUZZLE,
            };
            list.icon_centered(
                r,
                icon,
                16.0_f32.min(button * 0.55),
                if active { p.accent } else { p.muted },
            );
            x += button + gap;
        }
    }

    /// 工具条按钮的可见集合与顺序，照抄 `MainLayout.tsx` 的 JSX 顺序。
    pub fn toolbar_buttons(&self, state: &ChromeState) -> Vec<RightPanel> {
        let mut out = Vec::new();
        if state.outline_in_ai_sidebar {
            out.push(RightPanel::Outline);
        }
        out.push(RightPanel::Comments);
        out.push(RightPanel::Assistant);
        out.push(RightPanel::Pomodoro);
        out.push(RightPanel::DocumentMounts);
        if state.active_file_is_pdf {
            out.push(RightPanel::Annotations);
        }
        if super::settings_values::boolean("git.showVersionHistory", true) {
            out.push(RightPanel::VersionHistory);
        }
        out.push(RightPanel::AgentInbox);
        out.push(RightPanel::Plugin);
        out
    }

    /// Resolve a toolbar click using the exact rectangles used by `paint_toolbar`.
    pub fn toolbar_button_hit(&self, state: &ChromeState, x: f32, y: f32) -> Option<RightPanel> {
        let toolbar = self.tree.rect(self.right_sidebar_toolbar);
        let content = toolbar.inset(Edges::xy(toolbar_horizontal_inset(), 0.0));
        let button = toolbar_button_size(toolbar.height());
        let gap = toolbar_button_gap();
        let top = toolbar.top + (toolbar.height() - button) * 0.5;
        let mut left = content.left;
        for panel in self.toolbar_buttons(state) {
            let rect = Rect::from_size(left, top, button, button);
            if rect.right > content.right {
                break;
            }
            if rect.contains(x, y) {
                return Some(panel);
            }
            left += button + gap;
        }
        None
    }

    /// 命中测试的转发。
    pub fn hit(&self, x: f32, y: f32) -> Option<NodeKey> {
        if self.navigation_resize_hit.contains(x, y) {
            return Some(NodeKey::NavigationResize);
        }
        if self.sidebar_resize_hit.contains(x, y) {
            return Some(NodeKey::SidebarResize);
        }
        if self.right_sidebar_resize_hit.contains(x, y) {
            return Some(NodeKey::RightSidebarResize);
        }
        self.tree.hit(x, y).map(|h| h.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::DrawCmd;

    const VIEW: Rect = Rect {
        left: 0.0,
        top: 0.0,
        right: 1200.0,
        bottom: 800.0,
    };

    /// 大多数几何用例验的是编辑器视图（三栏俱全）。
    fn editor_state() -> ChromeState {
        ChromeState {
            view: WorkspaceView::Editor,
            ..Default::default()
        }
    }

    fn painted(state: &ChromeState) -> (Chrome, DrawList) {
        let chrome = Chrome::build(state, VIEW);
        let mut list = DrawList::new();
        chrome.paint(state, &mut list);
        assert!(list.finish().is_ok(), "裁剪栈没配平");
        (chrome, list)
    }

    /// 找出覆盖指定点的最后一条矩形指令的颜色——「这个像素最终是什么颜色」。
    fn color_at(list: &DrawList, x: f32, y: f32) -> Option<u32> {
        list.cmds().iter().rev().find_map(|c| match c {
            DrawCmd::Rect { rect, color } if rect.contains(x, y) => Some(*color),
            _ => None,
        })
    }

    #[test]
    fn hidden_title_bar_entries_release_space_and_can_be_restored() {
        use mochi_core::{
            app_settings::{self, AppSettings, SettingValue},
            settings::SettingsService,
        };
        let settings = AppSettings::new(std::sync::Arc::new(SettingsService::new(None)));
        let descriptor = app_settings::descriptor("navigation.titleBar.assistant").unwrap();
        let original = Chrome::build(&editor_state(), VIEW);
        let ai = original.tree.rect(original.ai_toggle);
        settings.write(descriptor, &SettingValue::Bool(false));
        super::super::settings_values::load(&settings);
        let hidden = Chrome::build(&editor_state(), VIEW);
        assert!(hidden.tree.rect(hidden.ai_toggle).is_empty());
        assert_eq!(hidden.tree.rect(hidden.notifications_toggle), ai);
        assert_eq!(
            hidden.hit(ai.left + 2.0, ai.top + 2.0),
            Some(NodeKey::TitleBarNotifications)
        );
        settings.write(descriptor, &SettingValue::Bool(true));
        super::super::settings_values::load(&settings);
        let restored = Chrome::build(&editor_state(), VIEW);
        assert_eq!(restored.tree.rect(restored.ai_toggle), ai);
        super::super::settings_values::reset();
    }

    #[test]
    fn the_default_layout_matches_the_electron_geometry() {
        let state = editor_state();
        let chrome = Chrome::build(&state, VIEW);

        // 标题栏 32 通栏
        assert_eq!(
            chrome.tree.rect(chrome.title_bar),
            Rect::new(0.0, 0.0, 1200.0, 32.0)
        );
        // 导航 220 + 侧栏 260 → 编辑器列从 480 起
        assert_eq!(
            chrome.tree.rect(chrome.navigation),
            Rect::new(0.0, 32.0, 220.0, 800.0)
        );
        assert_eq!(
            chrome.tree.rect(chrome.sidebar),
            Rect::new(220.0, 32.0, 480.0, 800.0)
        );
        // 右侧栏默认关着，编辑器列吃满剩余
        assert_eq!(
            chrome.tree.rect(chrome.editor_column),
            Rect::new(480.0, 32.0, 1200.0, 800.0)
        );
        assert!(chrome.tree.is_hidden(chrome.right_sidebar));
    }

    #[test]
    fn the_status_bar_spans_only_the_editor_column() {
        // 旧版 main.rs 把状态栏画成通栏，是一处 1:1 走样。这条钉住正确形状。
        let state = editor_state();
        let chrome = Chrome::build(&state, VIEW);
        let status = chrome.tree.rect(chrome.status_bar);

        assert_eq!(status.left, 480.0);
        assert_eq!(status.right, 1200.0);
        assert_eq!(status.bottom, 800.0);
        assert_eq!(status.height(), 24.0);
        // 侧栏（220..480）最底下那格属于侧栏，状态栏没有伸到它上面
        assert_eq!(chrome.hit(300.0, 795.0), Some(NodeKey::Sidebar));
        // 导航轨（0..220）同理
        assert_eq!(chrome.hit(100.0, 795.0), Some(NodeKey::Navigation));
    }

    #[test]
    fn opening_the_ai_panel_shrinks_the_editor_column_by_exactly_its_width() {
        let mut state = editor_state();
        let before = Chrome::build(&state, VIEW).tree.rect(NodeId::ROOT);
        let _ = before;
        let closed = Chrome::build(&state, VIEW);
        let closed_width = closed.tree.rect(closed.editor_column).width();

        state.ai_panel_open = true;
        let open = Chrome::build(&state, VIEW);
        assert_eq!(open.tree.rect(open.right_sidebar).width(), 420.0);
        assert_eq!(
            closed_width - open.tree.rect(open.editor_column).width(),
            420.0
        );
    }

    #[test]
    fn the_outline_placement_alone_is_enough_to_show_the_right_sidebar() {
        // showRightSidebar = aiPanelOpen || outlineInAISidebar
        let mut state = editor_state();
        state.ai_panel_open = false;
        state.outline_in_ai_sidebar = true;
        let chrome = Chrome::build(&state, VIEW);
        assert!(!chrome.tree.is_hidden(chrome.right_sidebar));
    }

    #[test]
    fn hiding_the_sidebar_gives_its_width_to_the_editor_column() {
        let mut state = editor_state();
        state.sidebar_visible = false;
        let chrome = Chrome::build(&state, VIEW);

        assert_eq!(chrome.tree.rect(chrome.sidebar), Rect::ZERO);
        // 编辑器列从导航右缘接上，不留空隙
        assert_eq!(chrome.tree.rect(chrome.editor_column).left, 220.0);
        assert_eq!(chrome.hit(300.0, 400.0), Some(NodeKey::Editor));
    }

    #[test]
    fn a_collapsed_navigation_is_60_wide_and_has_no_resize_handle() {
        let mut state = editor_state();
        state.navigation_collapsed = true;
        let chrome = Chrome::build(&state, VIEW);

        assert_eq!(chrome.tree.rect(chrome.navigation).width(), 60.0);
        assert!(chrome.tree.is_hidden(chrome.navigation_resize));
        // 折叠时点导航右缘拿到的是导航本身，不是手柄
        assert_eq!(chrome.hit(58.0, 400.0), Some(NodeKey::Navigation));
    }

    #[test]
    fn the_resize_handles_sit_on_the_inner_edges_of_their_panels() {
        let mut state = editor_state();
        state.ai_panel_open = true;
        let chrome = Chrome::build(&state, VIEW);

        // 导航手柄贴右缘
        assert_eq!(
            chrome.tree.rect(chrome.navigation_resize),
            Rect::new(216.0, 32.0, 220.0, 800.0)
        );
        // AI 面板手柄贴左缘
        assert_eq!(
            chrome.tree.rect(chrome.right_sidebar_resize),
            Rect::new(780.0, 32.0, 784.0, 800.0)
        );

        assert_eq!(chrome.hit(218.0, 400.0), Some(NodeKey::NavigationResize));
        assert_eq!(chrome.hit(782.0, 400.0), Some(NodeKey::RightSidebarResize));
    }

    #[test]
    fn outer_splitters_keep_four_dip_visuals_but_accept_an_eight_dip_grab_area() {
        let mut state = editor_state();
        state.ai_panel_open = true;
        let chrome = Chrome::build(&state, VIEW);

        // 绘制和布局用的矩形仍保持样式变量规定的原始宽度。
        assert_eq!(chrome.tree.rect(chrome.navigation_resize).width(), 4.0);
        assert_eq!(chrome.tree.rect(chrome.sidebar_resize).width(), 4.0);
        assert_eq!(chrome.tree.rect(chrome.right_sidebar_resize).width(), 4.0);

        // 多出来的四个不可见 DIP 加在所属面板那一侧。
        assert_eq!(chrome.hit(212.1, 400.0), Some(NodeKey::NavigationResize));
        assert_eq!(chrome.hit(472.1, 400.0), Some(NodeKey::SidebarResize));
        assert_eq!(chrome.hit(787.9, 400.0), Some(NodeKey::RightSidebarResize));
    }

    #[test]
    fn expanded_right_splitter_hit_covers_both_sides_without_covering_toolbar_buttons() {
        let mut state = editor_state();
        state.ai_panel_open = true;
        let chrome = Chrome::build(&state, VIEW);

        // 右侧栏从 x=780 开始。对称的命中区域向编辑器和面板各延伸八个
        // DIP。第一个工具条按钮仍在 x=788。
        assert_eq!(chrome.hit(772.0, 50.0), Some(NodeKey::RightSidebarResize));
        assert_eq!(chrome.hit(779.9, 50.0), Some(NodeKey::RightSidebarResize));
        assert_eq!(chrome.hit(788.0, 50.0), Some(NodeKey::RightSidebarToolbar));
    }

    #[test]
    fn the_ai_toggle_avoids_the_window_control_buttons() {
        let state = editor_state();
        let chrome = Chrome::build(&state, VIEW);
        let toggle = chrome.tree.rect(chrome.ai_toggle);

        // 主题切换按钮位于 AI 开关和窗口控制按钮之间。
        assert_eq!(
            toggle.right,
            1200.0 - theme::tokens().layout.title_bar_controls_width - 32.0
        );
        assert_eq!(toggle.width(), 32.0);
        // 让开的那段仍归标题栏（真实版里那是最小化/最大化/关闭）
        assert_eq!(chrome.hit(1150.0, 16.0), Some(NodeKey::TitleBar));
        assert_eq!(chrome.hit(1018.0, 16.0), Some(NodeKey::TitleBarAiToggle));
        assert_eq!(chrome.hit(1050.0, 16.0), Some(NodeKey::TitleBarThemeToggle));
        assert_eq!(chrome.tree.rect(chrome.theme_toggle).left, toggle.right);
    }

    #[test]
    fn the_editor_column_is_the_one_that_gets_squeezed_on_a_narrow_window() {
        // min-w-0：窗口拖到比 导航+侧栏+AI面板 还窄时，被挤扁的是编辑器列
        let mut state = editor_state();
        state.ai_panel_open = true;
        let narrow = Rect::new(0.0, 0.0, 700.0, 600.0);
        let chrome = Chrome::build(&state, narrow);

        assert_eq!(chrome.tree.rect(chrome.navigation).width(), 220.0);
        assert_eq!(chrome.tree.rect(chrome.sidebar).width(), 260.0);
        // AI 面板被钳到 max(340, 700*0.45=315) = 340
        assert_eq!(chrome.tree.rect(chrome.right_sidebar).width(), 340.0);
        assert_eq!(chrome.tree.rect(chrome.editor_column).width(), 0.0);
    }

    #[test]
    fn each_area_is_painted_with_its_own_background_token() {
        let mut state = editor_state();
        state.ai_panel_open = true;
        let (_, list) = painted(&state);
        let p = theme::tokens().palette(false);

        assert_eq!(
            color_at(&list, 100.0, 400.0),
            Some(p.area_navigation_default)
        );
        assert_eq!(color_at(&list, 300.0, 400.0), Some(p.area_sidebar_default));
        assert_eq!(color_at(&list, 600.0, 400.0), Some(p.area_main_default));
        assert_eq!(
            color_at(&list, 900.0, 400.0),
            Some(p.area_assistant_default)
        );
        assert_eq!(color_at(&list, 600.0, 16.0), Some(p.surface)); // 标题栏
    }

    #[test]
    fn the_dark_theme_paints_dark_colors() {
        let mut state = editor_state();
        state.dark = true;
        let (_, list) = painted(&state);
        let p = theme::tokens().palette(true);

        assert_eq!(
            color_at(&list, 100.0, 400.0),
            Some(p.area_navigation_default)
        );
        assert_eq!(color_at(&list, 600.0, 16.0), Some(p.surface));
        // 明暗两套确实不同——不是两次都取到了亮色
        assert_ne!(p.surface, theme::tokens().palette(false).surface);
    }

    #[test]
    fn nothing_from_a_hidden_panel_is_painted() {
        // 右侧栏关着时，工具条/边框都不该出现在指令里
        let state = editor_state();
        let (chrome, list) = painted(&state);
        let toolbar = chrome.tree.rect(chrome.right_sidebar_toolbar);
        assert_eq!(toolbar, Rect::ZERO);
        assert!(list
            .cmds()
            .iter()
            .all(|c| !matches!(c, DrawCmd::Rect { rect, .. } if rect.left >= 780.0 && rect.right <= 784.0)));
    }

    #[test]
    fn the_toolbar_shows_the_pdf_button_only_for_pdf_files() {
        let mut state = editor_state();
        state.ai_panel_open = true;
        let chrome = Chrome::build(&state, VIEW);
        assert!(!chrome
            .toolbar_buttons(&state)
            .contains(&RightPanel::Annotations));

        state.active_file_is_pdf = true;
        assert!(chrome
            .toolbar_buttons(&state)
            .contains(&RightPanel::Annotations));
    }

    #[test]
    fn the_outline_button_appears_only_when_the_outline_lives_in_the_sidebar() {
        let mut state = editor_state();
        state.ai_panel_open = true;
        let chrome = Chrome::build(&state, VIEW);
        assert_eq!(
            chrome.toolbar_buttons(&state).first(),
            Some(&RightPanel::Comments)
        );

        state.outline_in_ai_sidebar = true;
        assert_eq!(
            chrome.toolbar_buttons(&state).first(),
            Some(&RightPanel::Outline)
        );
    }

    #[test]
    fn the_active_toolbar_button_gets_an_accent_tinted_background() {
        let mut state = editor_state();
        state.ai_panel_open = true;
        state.right_panel = RightPanel::Assistant;
        let (_, list) = painted(&state);
        let p = theme::tokens().palette(false);
        let tint = theme::mix(p.accent, p.area_assistant_default, 0.10);

        assert!(list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Rect { color, .. } if *color == tint)));
    }

    #[test]
    fn the_ai_toggle_lights_up_only_for_the_assistant_view() {
        // TSX: aiPanelOpen && panelView === 'assistant'
        let p = theme::tokens().palette(false);
        let tint = theme::mix(p.accent, p.surface, 0.10);

        let mut state = editor_state();
        state.ai_panel_open = true;
        state.right_panel = RightPanel::Pomodoro;
        let (_, list) = painted(&state);
        assert_eq!(color_at(&list, 1018.0, 16.0), Some(p.surface));

        state.right_panel = RightPanel::Assistant;
        let (_, list) = painted(&state);
        assert_eq!(color_at(&list, 1018.0, 16.0), Some(tint));
    }

    #[test]
    fn the_tab_bar_is_tinted_three_percent_over_the_main_area() {
        // index.css: color-mix(in srgb, var(--foreground) 3%, var(--area-main-default))
        let state = editor_state();
        let (chrome, list) = painted(&state);
        let p = theme::tokens().palette(false);
        let tabs = chrome.tree.rect(chrome.tab_bar);

        assert_eq!(
            color_at(&list, tabs.left + 10.0, tabs.top + 5.0),
            Some(theme::mix(p.foreground, p.area_main_default, 0.03))
        );
    }

    #[test]
    fn the_compact_tab_bar_shortens_the_row_and_the_editor_grows_to_match() {
        let mut state = editor_state();
        let normal = Chrome::build(&state, VIEW);
        let normal_editor = normal.tree.rect(normal.editor).height();

        state.compact_tab_bar = true;
        let compact = Chrome::build(&state, VIEW);
        assert_eq!(compact.tree.rect(compact.tab_bar).height(), 33.0);
        assert_eq!(
            compact.tree.rect(compact.editor).height() - normal_editor,
            4.0
        );
    }

    #[test]
    fn the_title_bar_says_only_mochi_even_with_a_workspace_open() {
        // 标题栏仅显示产品名称，不拼接工作区名称。
        let mut state = editor_state();
        state.workspace_name = Some("mochi".to_owned());
        let (_, list) = painted(&state);

        assert!(list.cmds().iter().any(
            |c| matches!(c, DrawCmd::Text { text, style: TextStyle::Title, .. } if text == "墨池")
        ));
        assert!(!list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Text { text, .. } if text.contains("mochi"))));
    }

    #[test]
    fn the_status_text_is_drawn_inside_the_status_bar() {
        let mut state = editor_state();
        state.status_text = "771 个文件 · 软件光栅".to_owned();
        let (chrome, list) = painted(&state);
        let bar = chrome.tree.rect(chrome.status_bar);

        let drawn = list
            .cmds()
            .iter()
            .find_map(|c| match c {
                DrawCmd::Text { rect, text, .. } if text.contains("771") => Some(*rect),
                _ => None,
            })
            .expect("状态栏文字没画出来");
        assert!(drawn.top >= bar.top && drawn.bottom <= bar.bottom);
        assert!(drawn.left >= bar.left);
    }

    #[test]
    fn the_home_view_takes_over_the_whole_main_area() {
        // TSX 的 isHomeView 分支整个替换掉 TabBar + EditorArea + StatusBar，
        // 而 showWorkspaceLeftPanel 在首页时为 false——左侧栏也不显示
        let state = ChromeState {
            view: WorkspaceView::Home,
            ..Default::default()
        };
        let chrome = Chrome::build(&state, VIEW);

        assert!(chrome.tree.is_hidden(chrome.sidebar), "首页不该显示左侧栏");
        assert!(chrome.tree.is_hidden(chrome.tab_bar), "首页不该有标签栏");
        assert!(chrome.tree.is_hidden(chrome.status_bar), "首页不该有状态栏");
        // 主区从导航轨右缘一路铺到底
        let editor = chrome.tree.rect(chrome.editor);
        assert_eq!(editor.left, 220.0);
        assert_eq!(editor.top, 32.0);
        assert_eq!(editor.bottom, 800.0);
    }

    #[test]
    fn dedicated_views_with_their_own_navigation_keep_the_left_panel_but_drop_the_tab_bar() {
        // TSX：showWorkspaceLeftPanel 只排除 home/recent/quick-note/inbox，
        // 日程、墨池 AI 与 Agent 配置把左侧栏换成自己的内容；但它们仍是独立视图，没有标签栏
        for view in [
            WorkspaceView::Schedule,
            WorkspaceView::MochiAi,
            WorkspaceView::AgentConfig,
        ] {
            let state = ChromeState {
                view,
                ..Default::default()
            };
            let chrome = Chrome::build(&state, VIEW);
            assert!(
                !chrome.tree.is_hidden(chrome.sidebar),
                "{view:?} 该有左侧栏"
            );
            assert!(
                chrome.tree.is_hidden(chrome.tab_bar),
                "{view:?} 不该有标签栏"
            );
            assert!(chrome.tree.is_hidden(chrome.status_bar));
        }
        for view in [
            WorkspaceView::Recent,
            WorkspaceView::QuickNote,
            WorkspaceView::Inbox,
        ] {
            let state = ChromeState {
                view,
                ..Default::default()
            };
            let chrome = Chrome::build(&state, VIEW);
            assert!(
                chrome.tree.is_hidden(chrome.sidebar),
                "{view:?} 不该有左侧栏"
            );
        }
    }

    #[test]
    fn switching_back_to_the_editor_view_restores_all_three() {
        let chrome = Chrome::build(&editor_state(), VIEW);
        assert!(!chrome.tree.is_hidden(chrome.sidebar));
        assert!(!chrome.tree.is_hidden(chrome.tab_bar));
        assert!(!chrome.tree.is_hidden(chrome.status_bar));
    }

    #[test]
    fn nothing_from_the_hidden_tab_bar_or_status_bar_is_painted_on_home() {
        // 藏起来还照画的话，首页顶上会横着一条空的灰带——实测就是这样
        let state = ChromeState {
            view: WorkspaceView::Home,
            ..Default::default()
        };
        let (_, list) = painted(&state);
        let p = theme::tokens().palette(false);
        let tab_tint = theme::mix(p.foreground, p.area_main_default, 0.03);
        assert!(!list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Rect { color, .. } if *color == tab_tint)));
    }

    #[test]
    fn a_degenerate_viewport_does_not_panic_or_produce_garbage() {
        // 窗口最小化时 WM_SIZE 会给 0x0
        let state = editor_state();
        let chrome = Chrome::build(&state, Rect::new(0.0, 0.0, 0.0, 0.0));
        let mut list = DrawList::new();
        chrome.paint(&state, &mut list);
        assert!(list.finish().is_ok());
        assert_eq!(chrome.hit(0.0, 0.0), None);
    }
}
