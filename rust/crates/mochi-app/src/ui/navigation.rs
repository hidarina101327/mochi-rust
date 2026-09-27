//! 绘制和命中共用 NavLayout。

use std::collections::HashSet;

use super::draw::{Align, DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::theme::{self, Palette};
use super::widgets::{FieldLook, TextField};

/// `ai-prompts` 类型不在导航轨里列出——它是 AI 提示词配置，
/// 走的是「Agent配置」入口。TSX 里到处写着 `libraryType.id !== 'ai-prompts'`。
pub const HIDDEN_TYPE_ID: &str = "ai-prompts";

/// 默认展开的库类型（TSX：`useState({ 'knowledge-base': true })`）。
pub const DEFAULT_EXPANDED_TYPE: &str = "knowledge-base";

/// 九个内置入口，默认顺序与共享导航设置一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NavItem {
    Home,
    Inbox,
    Knowledge,
    Favorites,
    Schedule,
    MochiAi,
    QuickNote,
    Recent,
    Templates,
    AgentConfig,
    Automations,
}

impl NavItem {
    pub fn id(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Inbox => "inbox",
            Self::Knowledge => "knowledge",
            Self::Favorites => "favorites",
            Self::Schedule => "schedule",
            Self::MochiAi => "mochi-ai",
            Self::QuickNote => "notes",
            Self::Recent => "recent",
            Self::Templates => "templates",
            Self::AgentConfig => "agent-config",
            Self::Automations => "automations",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.id() == id.trim())
    }

    pub const ALL: [NavItem; 9] = [
        NavItem::Home,
        NavItem::Inbox,
        NavItem::Favorites,
        NavItem::Schedule,
        NavItem::MochiAi,
        NavItem::QuickNote,
        NavItem::Recent,
        NavItem::Templates,
        NavItem::Automations,
    ];

    pub fn label(self) -> &'static str {
        match self {
            NavItem::Home => "首页",
            NavItem::Inbox => "收件箱",
            NavItem::Knowledge => "知识库",
            NavItem::Favorites => "收藏",
            NavItem::Schedule => "日程待办",
            NavItem::MochiAi => "墨池AI",
            NavItem::QuickNote => "小记",
            NavItem::Recent => "最近",
            NavItem::Templates => "模板中心",
            NavItem::AgentConfig => "Agent配置",
            NavItem::Automations => "自动化",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            NavItem::Home => Icon::HOME,
            NavItem::Inbox => Icon::INBOX,
            NavItem::Knowledge => Icon::FOLDER,
            NavItem::Favorites => Icon::STAR,
            NavItem::Schedule => Icon::CALENDAR_DAYS,
            NavItem::MochiAi => Icon::BOT,
            NavItem::QuickNote => Icon::BOOK_OPEN,
            NavItem::Recent => Icon::CLOCK,
            NavItem::Templates => Icon::LAYOUT_GRID,
            NavItem::AgentConfig => Icon::SPARKLES,
            NavItem::Automations => Icon::GIT_BRANCH,
        }
    }
}

/// 点到了导航轨的什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavHit {
    Collapse,
    /// 搜索按钮。
    Search,
    Item(NavItem),
    /// 库类型标题（下标指向 [`NavModel::types`]）。
    TypeHeader(usize),
    /// 库类型标题右侧的「+」。
    TypeAdd(usize),
    TypeIcon(usize),
    /// 库实例（下标指向 [`NavModel::libraries`] 的**原始**下标）。
    Library(usize),
    /// 正在输入的新建库输入框。
    NewLibraryField,
    Settings,
    PluginEntry(usize),
}

/// 导航轨要画什么，全部由外部投影进来——它不持有 `Shell`。
pub struct NavModel<'a> {
    /// (类型 id, 类型名)。
    pub types: &'a [(String, String)],
    /// (所属类型 id, 库名)。下标是 `WorkspaceState::libraries` 的原始下标。
    pub libraries: &'a [(String, String)],
    pub expanded_types: &'a HashSet<String>,
    /// 正在哪个类型下新建库，以及输入框。
    pub creating: Option<(&'a str, &'a TextField, &'a str)>,
    pub inbox_count: usize,
    pub collapsed: bool,
    /// 高亮的固定入口。`None` 时没有入口高亮（例如打开了一个设置标签）。
    pub active_item: Option<NavItem>,
    pub plugin_entries: &'a [(String, String)],
    pub active_plugin: Option<usize>,
    pub selected_library: Option<usize>,
}

/// 一次布局的产物：每个可点元素的矩形。
#[derive(Debug, Clone, Default)]
pub struct NavLayout {
    pub entries: Vec<(Rect, NavHit)>,
    /// 可滚动区的矩形与内容总高。
    pub scroll_area: Rect,
    pub content_height: f32,
}

impl NavLayout {
    pub fn hit(&self, x: f32, y: f32) -> Option<NavHit> {
        // 后加的元素优先：「+」画在标题行之上
        self.entries
            .iter()
            .rev()
            .find(|(r, hit)| {
                r.contains(x, y)
                    && (matches!(hit, NavHit::Collapse | NavHit::Search | NavHit::Settings)
                        || self.scroll_area.contains(x, y))
            })
            .map(|(_, h)| *h)
    }

    pub fn rect_of(&self, hit: NavHit) -> Option<Rect> {
        self.entries
            .iter()
            .find(|(_, h)| *h == hit)
            .map(|(r, _)| *r)
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.scroll_area.height()).max(0.0)
    }
}

// Tailwind 尺寸（px）
const PAD_X: f32 = 16.0;
const PAD_X_COLLAPSED: f32 = 8.0;
const LOGO: f32 = 28.0;
// GlobalNavigation 的文字是 13px，沿用正文 1.8 的行高；py-1.5 再增加 12px。
// 展开态的行高由文字决定，而不是由 20px 图标决定。
const ITEM_HEIGHT: f32 = 13.0 * 1.8 + 12.0;
const ITEM_HEIGHT_COLLAPSED: f32 = 40.0;
const ITEM_GAP: f32 = 1.0;
const ICON: f32 = 20.0;
const RADIUS: f32 = 6.0;
const TYPE_HEADER_HEIGHT: f32 = 36.0;
const LIBRARY_ROW_HEIGHT: f32 = ITEM_HEIGHT;
const EMPTY_ROW_HEIGHT: f32 = 11.0 * 1.8 + 8.0;
const NEW_LIBRARY_ROW_HEIGHT: f32 = 32.0;

fn setting_number(key: &str, fallback: f32, min: f32, max: f32) -> f32 {
    let value = super::settings_values::number(key, fallback);
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

fn item_scale() -> f32 {
    setting_number("navigation.itemScale", 100.0, 50.0, 300.0) / 100.0
}

fn library_item_scale() -> f32 {
    setting_number("navigation.libraryItemScale", 100.0, 50.0, 300.0) / 100.0
}

fn item_height() -> f32 {
    (TextStyle::Body.font_size() * 1.8 + 12.0) * item_scale()
}

fn collapsed_item_height() -> f32 {
    ITEM_HEIGHT_COLLAPSED * item_scale()
}

fn library_item_height() -> f32 {
    (TextStyle::Body.font_size() * 1.8 + 12.0) * library_item_scale()
}

fn item_gap() -> f32 {
    ITEM_GAP * item_scale()
}

fn library_item_gap() -> f32 {
    setting_number("navigation.libraryItemSpacing", ITEM_GAP, 0.0, 48.0)
}

fn library_group_gap() -> f32 {
    setting_number("navigation.libraryGroupSpacing", 20.0, 0.0, 64.0)
}

fn item_icon_size() -> f32 {
    ICON * item_scale()
}

fn library_icon_size() -> f32 {
    14.0 * library_item_scale()
}

fn item_radius() -> f32 {
    RADIUS * item_scale()
}

fn scaled_text(
    list: &mut DrawList,
    rect: Rect,
    text: impl Into<String>,
    style: TextStyle,
    color: u32,
    align: Align,
    scale: f32,
) {
    let start = list.cmds().len();
    list.text_aligned(rect, text, style, color, align);
    if (scale - 1.0).abs() > f32::EPSILON {
        list.scale_text_since(start, scale);
    }
}

/// 算一次布局。`scroll` 是可滚动区已滚过的像素。
pub fn layout(model: &NavModel, area: Rect, scroll: f32) -> NavLayout {
    let mut out = NavLayout::default();
    if area.is_empty() {
        return out;
    }
    let collapsed = model.collapsed;
    let pad_x = if collapsed { PAD_X_COLLAPSED } else { PAD_X };

    // ── 头部 ──
    let mut y = area.top + 16.0;
    if collapsed {
        // flex-col gap-2：logo 在上，折叠按钮在下
        let cx = (area.left + area.right) / 2.0;
        y += LOGO + 8.0;
        out.entries
            .push((Rect::from_size(cx - 12.0, y, 24.0, 24.0), NavHit::Collapse));
        y += 24.0 + 12.0;
        out.entries.push((
            Rect::new(area.left + pad_x, y, area.right - pad_x, y + 36.0),
            NavHit::Search,
        ));
        y += 36.0;
    } else {
        out.entries.push((
            Rect::from_size(
                area.right - pad_x - 24.0,
                y + (LOGO - 24.0) / 2.0,
                24.0,
                24.0,
            ),
            NavHit::Collapse,
        ));
        y += LOGO + 12.0;
        out.entries.push((
            Rect::new(area.left + pad_x, y, area.right - pad_x, y + ITEM_HEIGHT),
            NavHit::Search,
        ));
        y += ITEM_HEIGHT;
    }
    y += 8.0; // pb-2
    let header_bottom = y;

    // ── 底部（先算，可滚动区夹在中间）──
    let row_h = if collapsed {
        collapsed_item_height()
    } else {
        item_height()
    };
    let settings_block = row_h + 16.0;
    let settings_top = area.bottom - settings_block;
    out.entries.push((
        Rect::new(
            area.left + 8.0,
            settings_top + 8.0,
            area.right - 8.0,
            settings_top + 8.0 + row_h,
        ),
        NavHit::Settings,
    ));
    let scroll_bottom = settings_top;
    out.scroll_area = Rect::new(
        area.left,
        header_bottom,
        area.right,
        scroll_bottom.max(header_bottom),
    );

    // ── 可滚动区 ──
    let x0 = area.left + 8.0;
    let x1 = area.right - 8.0;
    let mut cy = header_bottom + 4.0 - scroll; // py-1
    let visible_items = super::navigation_preferences::Preferences::read().visible();
    for (i, item) in visible_items.iter().enumerate() {
        if i > 0 {
            cy += item_gap();
        }
        out.entries
            .push((Rect::new(x0, cy, x1, cy + row_h), NavHit::Item(*item)));
        cy += row_h;
    }
    for (index, _) in model.plugin_entries.iter().enumerate() {
        cy += item_gap();
        out.entries.push((
            Rect::new(x0, cy, x1, cy + row_h),
            NavHit::PluginEntry(index),
        ));
        cy += row_h;
    }

    let shown_types: Vec<(usize, &(String, String))> = model
        .types
        .iter()
        .enumerate()
        .filter(|(_, (id, _))| id != HIDDEN_TYPE_ID)
        .collect();

    if !collapsed {
        if !visible_items.is_empty() || !model.plugin_entries.is_empty() {
            cy += library_group_gap(); // 仅当导航组非空时，才在资料库之间留出间距。
        }
        for (ti, (type_id, _)) in &shown_types {
            let header = Rect::new(x0 + 10.0, cy, x1 - 10.0, cy + TYPE_HEADER_HEIGHT);
            out.entries.push((header, NavHit::TypeHeader(*ti)));
            out.entries.push((
                Rect::new(header.right - 16.0, header.top, header.right, header.bottom),
                NavHit::TypeAdd(*ti),
            ));
            cy += TYPE_HEADER_HEIGHT + 4.0; // mb-1

            if model.expanded_types.contains(type_id.as_str()) {
                let mut any = false;
                for (li, (kind, _)) in model.libraries.iter().enumerate() {
                    if kind == type_id {
                        if any {
                            cy += library_item_gap();
                        }
                        let height = library_item_height();
                        out.entries
                            .push((Rect::new(x0, cy, x1, cy + height), NavHit::Library(li)));
                        cy += height;
                        any = true;
                    }
                }
                let creating_here = matches!(model.creating, Some((t, _, _)) if t == type_id);
                if !any && !creating_here {
                    cy += EMPTY_ROW_HEIGHT;
                }
                if creating_here {
                    if any {
                        cy += library_item_gap();
                    }
                    out.entries.push((
                        Rect::new(x0, cy, x1, cy + NEW_LIBRARY_ROW_HEIGHT),
                        NavHit::NewLibraryField,
                    ));
                    cy += NEW_LIBRARY_ROW_HEIGHT;
                    // 错误提示行
                    if matches!(model.creating, Some((_, _, err)) if !err.is_empty()) {
                        cy += 18.0;
                    }
                }
            }
            cy += 12.0; // mb-3
        }
    } else if !shown_types.is_empty() {
        if !visible_items.is_empty() || !model.plugin_entries.is_empty() {
            cy += 12.0 + 1.0 + 12.0; // mt-3 border-t pt-3
        }
        for (i, (ti, _)) in shown_types.iter().enumerate() {
            if i > 0 {
                cy += item_gap();
            }
            let height = collapsed_item_height();
            out.entries
                .push((Rect::new(x0, cy, x1, cy + height), NavHit::TypeIcon(*ti)));
            cy += height;
        }
    }
    cy += 4.0; // py-1
    out.content_height = cy + scroll - header_bottom;

    // 滚出可滚动区的条目不参与命中
    let sa = out.scroll_area;
    out.entries.retain(|(r, hit)| match hit {
        NavHit::Collapse | NavHit::Search | NavHit::Settings => true,
        _ => r.bottom > sa.top && r.top < sa.bottom,
    });
    out
}

/// 画导航轨。
pub fn paint(
    list: &mut DrawList,
    area: Rect,
    model: &NavModel,
    lay: &NavLayout,
    scroll: f32,
    p: &Palette,
) {
    if area.is_empty() {
        return;
    }
    let collapsed = model.collapsed;
    let scale = item_scale();
    let library_scale = library_item_scale();
    let icon_size = item_icon_size();
    let radius = item_radius();
    let bg = p.area_navigation_default;
    list.push_clip(area);

    // ── 头部 ──
    let pad_x = if collapsed { PAD_X_COLLAPSED } else { PAD_X };
    let top = area.top + 16.0;
    if collapsed {
        let cx = (area.left + area.right) / 2.0;
        list.logo(Rect::from_size(cx - LOGO / 2.0, top, LOGO, LOGO));
    } else {
        list.logo(Rect::from_size(area.left + pad_x, top, LOGO, LOGO));
        list.text(
            Rect::new(
                area.left + pad_x + LOGO + 8.0,
                top,
                area.right - pad_x - 24.0,
                top + LOGO,
            ),
            "墨池",
            TextStyle::Title,
            p.foreground,
        );
    }
    if let Some(r) = lay.rect_of(NavHit::Collapse) {
        let icon = if collapsed {
            Icon::CHEVRON_RIGHT
        } else {
            Icon::CHEVRON_LEFT
        };
        list.icon_centered(r, icon, 16.0, p.muted);
    }
    if let Some(r) = lay.rect_of(NavHit::Search) {
        if collapsed {
            list.icon_centered(r, Icon::SEARCH, ICON, p.muted);
        } else {
            list.icon_centered(
                Rect::new(r.left + 10.0, r.top, r.left + 26.0, r.bottom),
                Icon::SEARCH,
                16.0,
                p.muted,
            );
            list.text(
                Rect::new(r.left + 34.0, r.top, r.right - 10.0, r.bottom),
                "搜索",
                TextStyle::Body,
                p.muted,
            );
        }
    }

    // ── 可滚动区 ──
    let sa = lay.scroll_area;
    list.push_clip(sa);
    for (r, hit) in &lay.entries {
        match hit {
            NavHit::Item(item) => {
                let active = model.active_item == Some(*item);
                let color = if active { p.accent } else { p.muted };
                if active {
                    list.rounded_rect(*r, radius, theme::mix(p.accent, bg, 0.10));
                }
                let badge = if *item == NavItem::Inbox {
                    model.inbox_count
                } else {
                    0
                };
                if collapsed {
                    list.icon_centered(*r, item.icon(), icon_size, color);
                    if badge > 0 {
                        // 折叠时没地方放数字，用一个圆点保住「有东西待处理」的信号
                        let cx = (r.left + r.right) / 2.0 + icon_size / 2.0;
                        let cy = (r.top + r.bottom) / 2.0 - icon_size / 2.0;
                        list.rounded_rect(
                            Rect::new(
                                cx - 4.0 * scale,
                                cy - 4.0 * scale,
                                cx + 4.0 * scale,
                                cy + 4.0 * scale,
                            ),
                            4.0 * scale,
                            bg,
                        );
                        list.rounded_rect(
                            Rect::new(
                                cx - 2.0 * scale,
                                cy - 2.0 * scale,
                                cx + 2.0 * scale,
                                cy + 2.0 * scale,
                            ),
                            2.0 * scale,
                            p.accent,
                        );
                    }
                } else {
                    // gap-2.5 px-2.5：图标 20 在左内边距 10 之后
                    list.icon_centered(
                        Rect::new(
                            r.left + 10.0 * scale,
                            r.top,
                            r.left + 30.0 * scale,
                            r.bottom,
                        ),
                        item.icon(),
                        icon_size,
                        color,
                    );
                    let mut text_right = r.right - 10.0 * scale;
                    if badge > 0 {
                        let label = if badge > 99 {
                            "99+".to_owned()
                        } else {
                            badge.to_string()
                        };
                        let w = (super::text::measure(&label, TextStyle::Caption) + 12.0) * scale;
                        let br = Rect::new(
                            r.right - 10.0 * scale - w,
                            r.top + 7.0 * scale,
                            r.right - 10.0 * scale,
                            r.bottom - 7.0 * scale,
                        );
                        list.rounded_rect(br, br.height() / 2.0, theme::mix(p.accent, bg, 0.12));
                        scaled_text(
                            list,
                            br,
                            label,
                            TextStyle::Caption,
                            p.accent,
                            Align::Center,
                            scale,
                        );
                        text_right = br.left - 6.0 * scale;
                    }
                    scaled_text(
                        list,
                        Rect::new(r.left + 40.0 * scale, r.top, text_right, r.bottom),
                        item.label(),
                        TextStyle::Body,
                        color,
                        Align::Leading,
                        scale,
                    );
                }
            }
            NavHit::PluginEntry(index) => {
                let active = model.active_plugin == Some(*index);
                let color = if active { p.accent } else { p.muted };
                if active {
                    list.rounded_rect(*r, radius, theme::mix(p.accent, bg, 0.10));
                }
                if collapsed {
                    list.icon_centered(*r, Icon::PUZZLE, icon_size, color);
                } else if let Some((_, title)) = model.plugin_entries.get(*index) {
                    list.icon_centered(
                        Rect::new(
                            r.left + 10.0 * scale,
                            r.top,
                            r.left + 30.0 * scale,
                            r.bottom,
                        ),
                        Icon::PUZZLE,
                        icon_size,
                        color,
                    );
                    scaled_text(
                        list,
                        Rect::new(
                            r.left + 40.0 * scale,
                            r.top,
                            r.right - 10.0 * scale,
                            r.bottom,
                        ),
                        title,
                        TextStyle::Body,
                        color,
                        Align::Leading,
                        scale,
                    );
                }
            }
            NavHit::TypeHeader(ti) => {
                let (type_id, name) = &model.types[*ti];
                let expanded = model.expanded_types.contains(type_id.as_str());
                let chevron = if expanded {
                    Icon::CHEVRON_DOWN
                } else {
                    Icon::CHEVRON_RIGHT
                };
                list.icon_centered(
                    Rect::new(r.left, r.top, r.left + 12.0, r.bottom),
                    chevron,
                    12.0,
                    p.muted,
                );
                // uppercase tracking-wider：中文没有大小写，英文类型名转大写
                list.text(
                    Rect::new(r.left + 16.0, r.top, r.right - 20.0, r.bottom),
                    name.to_uppercase(),
                    TextStyle::Caption,
                    p.muted,
                );
            }
            NavHit::TypeAdd(_) => {
                list.icon_centered(*r, Icon::PLUS, 12.0, p.muted);
            }
            NavHit::TypeIcon(ti) => {
                let (type_id, _) = &model.types[*ti];
                let has_selected = model
                    .selected_library
                    .and_then(|i| model.libraries.get(i))
                    .map(|(kind, _)| kind == type_id)
                    .unwrap_or(false);
                let color = if has_selected { p.accent } else { p.muted };
                if has_selected {
                    list.rounded_rect(*r, radius, theme::mix(p.accent, bg, 0.10));
                }
                list.icon_centered(*r, Icon::FOLDER, icon_size, color);
            }
            NavHit::Library(li) => {
                let (_, name) = &model.libraries[*li];
                let selected = model.selected_library == Some(*li);
                if selected {
                    list.rounded_rect(*r, radius, p.background);
                }
                let color = if selected { p.foreground } else { p.muted };
                // pl-6 gap-2：文件夹图标 14px，opacity-40
                list.icon_centered(
                    Rect::new(
                        r.left + 24.0 * library_scale,
                        r.top,
                        r.left + 38.0 * library_scale,
                        r.bottom,
                    ),
                    Icon::FOLDER,
                    library_icon_size(),
                    theme::mix(p.muted, bg, 0.4),
                );
                scaled_text(
                    list,
                    Rect::new(
                        r.left + 46.0 * library_scale,
                        r.top,
                        r.right - 10.0 * library_scale,
                        r.bottom,
                    ),
                    super::text::ellipsize(
                        name,
                        TextStyle::Body,
                        (r.width() / library_scale - 56.0).max(10.0),
                    ),
                    TextStyle::Body,
                    color,
                    Align::Leading,
                    library_scale,
                );
            }
            NavHit::NewLibraryField => {
                if let Some((_, field, err)) = model.creating {
                    list.icon_centered(
                        Rect::new(
                            r.left + 24.0 * library_scale,
                            r.top,
                            r.left + 38.0 * library_scale,
                            r.bottom,
                        ),
                        Icon::FOLDER,
                        library_icon_size(),
                        theme::mix(p.muted, bg, 0.4),
                    );
                    let field_rect = Rect::new(
                        r.left + 46.0 * library_scale,
                        r.top + 4.0,
                        r.right - 10.0 * library_scale,
                        r.bottom - 4.0,
                    );
                    // 输入框不可变借用地画：克隆一份（几十字节），避免让模型持有 &mut
                    let mut f = field.clone();
                    f.paint(list, field_rect, true, p, FieldLook::inline(p));
                    if !err.is_empty() {
                        list.text(
                            Rect::new(r.left + 44.0, r.bottom, r.right, r.bottom + 18.0),
                            err,
                            TextStyle::Caption,
                            p.danger,
                        );
                    }
                }
            }
            _ => {}
        }
    }
    // 「暂无」——展开但空的类型
    if !collapsed {
        let mut prev_header: Option<Rect> = None;
        for (r, hit) in &lay.entries {
            if let NavHit::TypeHeader(ti) = hit {
                let (type_id, _) = &model.types[*ti];
                let expanded = model.expanded_types.contains(type_id.as_str());
                let has_lib = model.libraries.iter().any(|(k, _)| k == type_id);
                let creating_here = matches!(model.creating, Some((t, _, _)) if t == type_id);
                if expanded && !has_lib && !creating_here {
                    let top = r.bottom + 4.0;
                    list.text(
                        Rect::new(r.left + 14.0, top, r.right, top + EMPTY_ROW_HEIGHT),
                        "暂无",
                        TextStyle::Caption,
                        p.muted,
                    );
                }
                prev_header = Some(*r);
            }
        }
        let _ = prev_header;
    } else if lay
        .entries
        .iter()
        .any(|(_, h)| matches!(h, NavHit::Item(_) | NavHit::PluginEntry(_)))
    {
        if let Some((first, _)) = lay
            .entries
            .iter()
            .find(|(_, h)| matches!(h, NavHit::TypeIcon(_)))
        {
            list.hline(
                area.left + 8.0,
                area.right - 8.0,
                first.top - 13.0,
                p.border,
            );
        }
    }
    list.pop_clip();

    // ── 底部 ──
    if let Some(r) = lay.rect_of(NavHit::Settings) {
        if collapsed {
            list.icon_centered(r, Icon::SETTINGS, icon_size, p.muted);
        } else {
            list.icon_centered(
                Rect::new(
                    r.left + 10.0 * scale,
                    r.top,
                    r.left + 30.0 * scale,
                    r.bottom,
                ),
                Icon::SETTINGS,
                icon_size,
                p.muted,
            );
            scaled_text(
                list,
                Rect::new(
                    r.left + 40.0 * scale,
                    r.top,
                    r.right - 10.0 * scale,
                    r.bottom,
                ),
                "设置",
                TextStyle::Body,
                p.muted,
                Align::Leading,
                scale,
            );
        }
    }
    let _ = scroll;
    list.pop_clip();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::DrawCmd;

    const AREA: Rect = Rect {
        left: 0.0,
        top: 32.0,
        right: 220.0,
        bottom: 800.0,
    };

    fn types() -> Vec<(String, String)> {
        vec![
            ("knowledge-base".into(), "知识库".into()),
            ("ai-prompts".into(), "AI 提示词".into()),
            ("problem-set".into(), "刷题库".into()),
        ]
    }

    fn libraries() -> Vec<(String, String)> {
        vec![
            ("knowledge-base".into(), "计算机通识".into()),
            ("knowledge-base".into(), "面试经历".into()),
            ("ai-prompts".into(), "提示词".into()),
            ("problem-set".into(), "LeetCode".into()),
        ]
    }

    fn expanded() -> HashSet<String> {
        [DEFAULT_EXPANDED_TYPE.to_owned()].into_iter().collect()
    }

    fn model<'a>(
        types: &'a [(String, String)],
        libs: &'a [(String, String)],
        exp: &'a HashSet<String>,
        collapsed: bool,
    ) -> NavModel<'a> {
        NavModel {
            types,
            libraries: libs,
            expanded_types: exp,
            creating: None,
            inbox_count: 0,
            collapsed,
            active_item: Some(NavItem::Home),
            plugin_entries: &[],
            active_plugin: None,
            selected_library: Some(0),
        }
    }

    fn texts(list: &DrawList) -> Vec<String> {
        list.cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn icons(list: &DrawList) -> Vec<Icon> {
        list.cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Icon { icon, .. } => Some(*icon),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn hiding_all_entries_removes_library_group_spacing() {
        use mochi_core::{
            app_settings::{self, AppSettings, SettingValue},
            settings::SettingsService,
        };
        let settings = AppSettings::new(std::sync::Arc::new(SettingsService::new(None)));
        settings.write(
            app_settings::descriptor(super::super::navigation_preferences::HIDDEN_KEY).unwrap(),
            &SettingValue::Text(NavItem::ALL.map(NavItem::id).join(",")),
        );
        super::super::settings_values::load(&settings);
        let (t, l, e) = (types(), libraries(), expanded());
        for collapsed in [false, true] {
            let lay = layout(&model(&t, &l, &e, collapsed), AREA, 0.0);
            let hit = if collapsed {
                NavHit::TypeIcon(0)
            } else {
                NavHit::TypeHeader(0)
            };
            assert_eq!(lay.rect_of(hit).unwrap().top, lay.scroll_area.top + 4.0);
        }
        super::super::settings_values::reset();
    }

    #[test]
    fn seven_primary_entries_remain_after_workspace_tools_move_to_titlebar() {
        let (t, l, e) = (types(), libraries(), expanded());
        let lay = layout(&model(&t, &l, &e, false), AREA, 0.0);
        let items: Vec<NavItem> = lay
            .entries
            .iter()
            .filter_map(|(_, h)| match h {
                NavHit::Item(i) => Some(*i),
                _ => None,
            })
            .collect();
        assert_eq!(
            items,
            vec![
                NavItem::Home,
                NavItem::Inbox,
                NavItem::Favorites,
                NavItem::Schedule,
                NavItem::MochiAi,
                NavItem::QuickNote,
                NavItem::Recent
            ]
        );
        assert!(lay.rect_of(NavHit::Item(NavItem::Templates)).is_none());
        assert!(lay.rect_of(NavHit::Item(NavItem::Automations)).is_none());
    }

    #[test]
    fn entries_use_inherited_line_height_with_a_1px_gap_below_the_header() {
        let (t, l, e) = (types(), libraries(), expanded());
        let lay = layout(&model(&t, &l, &e, false), AREA, 0.0);
        let home = lay.rect_of(NavHit::Item(NavItem::Home)).unwrap();
        let inbox = lay.rect_of(NavHit::Item(NavItem::Inbox)).unwrap();
        let header = 16.0 + LOGO + 12.0 + ITEM_HEIGHT + 8.0;
        assert!((home.top - (AREA.top + header + 4.0)).abs() < 0.01);
        assert!((home.height() - ITEM_HEIGHT).abs() < 0.01);
        assert_eq!(inbox.top - home.bottom, 1.0);
        // px-2
        assert_eq!(home.left, AREA.left + 8.0);
        assert_eq!(home.right, AREA.right - 8.0);
    }

    #[test]
    fn only_the_default_expanded_type_lists_its_libraries() {
        let (t, l, e) = (types(), libraries(), expanded());
        let lay = layout(&model(&t, &l, &e, false), AREA, 0.0);
        let libs: Vec<usize> = lay
            .entries
            .iter()
            .filter_map(|(_, h)| match h {
                NavHit::Library(i) => Some(*i),
                _ => None,
            })
            .collect();
        // 知识库展开：0、1；刷题库折叠：LeetCode(3) 不出现；ai-prompts 永不出现
        assert_eq!(libs, vec![0, 1]);
        let headers: Vec<usize> = lay
            .entries
            .iter()
            .filter_map(|(_, h)| match h {
                NavHit::TypeHeader(i) => Some(*i),
                _ => None,
            })
            .collect();
        assert_eq!(headers, vec![0, 2], "ai-prompts 类型不该列出");
    }

    #[test]
    fn a_library_row_keeps_the_original_index() {
        let (t, l) = (types(), libraries());
        let e: HashSet<String> = ["problem-set".to_owned()].into_iter().collect();
        let lay = layout(&model(&t, &l, &e, false), AREA, 0.0);
        let libs: Vec<usize> = lay
            .entries
            .iter()
            .filter_map(|(_, h)| match h {
                NavHit::Library(i) => Some(*i),
                _ => None,
            })
            .collect();
        assert_eq!(libs, vec![3], "过滤掉 ai-prompts 后下标不能跟着挪");
    }

    #[test]
    fn library_rows_use_label_line_height_and_type_headers_have_a_plus_on_the_right() {
        let (t, l, e) = (types(), libraries(), expanded());
        let lay = layout(&model(&t, &l, &e, false), AREA, 0.0);
        let row = lay.rect_of(NavHit::Library(0)).unwrap();
        assert!((row.height() - LIBRARY_ROW_HEIGHT).abs() < 0.01);
        let header = lay.rect_of(NavHit::TypeHeader(0)).unwrap();
        let plus = lay.rect_of(NavHit::TypeAdd(0)).unwrap();
        assert_eq!(plus.right, header.right);
        // 点在「+」上命中的是 TypeAdd 而不是 TypeHeader
        assert_eq!(
            lay.hit(plus.left + 4.0, plus.top + 4.0),
            Some(NavHit::TypeAdd(0))
        );
        assert_eq!(
            lay.hit(header.left + 4.0, header.top + 4.0),
            Some(NavHit::TypeHeader(0))
        );
    }

    #[test]
    fn collapsed_shows_only_icons_and_type_icons() {
        let (t, l, e) = (types(), libraries(), expanded());
        let m = model(&t, &l, &e, true);
        let lay = layout(&m, Rect::new(0.0, 32.0, 60.0, 800.0), 0.0);
        assert!(lay
            .entries
            .iter()
            .all(|(_, h)| !matches!(h, NavHit::Library(_) | NavHit::TypeHeader(_))));
        let type_icons = lay
            .entries
            .iter()
            .filter(|(_, h)| matches!(h, NavHit::TypeIcon(_)))
            .count();
        assert_eq!(type_icons, 2);
        let home = lay.rect_of(NavHit::Item(NavItem::Home)).unwrap();
        assert_eq!(home.height(), 40.0, "折叠态 p-2.5 + 20 图标");

        let mut list = DrawList::new();
        paint(
            &mut list,
            Rect::new(0.0, 32.0, 60.0, 800.0),
            &m,
            &lay,
            0.0,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        assert!(
            !t.iter().any(|s| s == "墨池" || s == "搜索" || s == "首页"),
            "折叠时不画文字: {t:?}"
        );
        assert!(icons(&list).contains(&Icon::HOME));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn the_active_entry_is_tinted_and_the_selected_library_gets_the_background_color() {
        let (t, l, e) = (types(), libraries(), expanded());
        let m = model(&t, &l, &e, false);
        let lay = layout(&m, AREA, 0.0);
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint(&mut list, AREA, &m, &lay, 0.0, &p);

        let tint = theme::mix(p.accent, p.area_navigation_default, 0.10);
        let tinted: Vec<Rect> = list
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::RoundedRect { rect, color, .. } if *color == tint => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(tinted.len(), 1);
        assert_eq!(tinted[0], lay.rect_of(NavHit::Item(NavItem::Home)).unwrap());

        let selected: Vec<Rect> = list
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::RoundedRect { rect, color, .. } if *color == p.background => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(selected, vec![lay.rect_of(NavHit::Library(0)).unwrap()]);
        assert!(list.finish().is_ok());
    }

    #[test]
    fn every_entry_paints_its_lucide_icon_and_label() {
        let (t, l, e) = (types(), libraries(), expanded());
        let m = model(&t, &l, &e, false);
        let lay = layout(&m, AREA, 0.0);
        let mut list = DrawList::new();
        paint(
            &mut list,
            AREA,
            &m,
            &lay,
            0.0,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        let ic = icons(&list);
        for item in [
            NavItem::Home,
            NavItem::Inbox,
            NavItem::Favorites,
            NavItem::Schedule,
            NavItem::MochiAi,
            NavItem::QuickNote,
            NavItem::Recent,
        ] {
            assert!(t.contains(&item.label().to_owned()), "缺 {}", item.label());
            assert!(ic.contains(&item.icon()), "缺图标 {:?}", item.icon());
        }
        assert!(!t.contains(&"模板中心".to_owned()));
        assert!(!t.contains(&"自动化".to_owned()));
        assert!(t.contains(&"设置".to_owned()));
        assert!(!t.contains(&"第三方库".to_owned()));
        assert_eq!(
            lay.scroll_area.bottom,
            lay.rect_of(NavHit::Settings).unwrap().top - 8.0
        );
        assert!(t.contains(&"搜索".to_owned()));
        assert!(
            ic.contains(&Icon::SETTINGS) && ic.contains(&Icon::SEARCH) && ic.contains(&Icon::PLUS)
        );
    }

    #[test]
    fn the_inbox_badge_shows_the_count_and_caps_at_99_plus() {
        let (t, l, e) = (types(), libraries(), expanded());
        let mut m = model(&t, &l, &e, false);
        m.inbox_count = 7;
        let lay = layout(&m, AREA, 0.0);
        let mut list = DrawList::new();
        paint(
            &mut list,
            AREA,
            &m,
            &lay,
            0.0,
            theme::tokens().palette(false),
        );
        assert!(texts(&list).contains(&"7".to_owned()));

        m.inbox_count = 150;
        let mut list = DrawList::new();
        paint(
            &mut list,
            AREA,
            &m,
            &lay,
            0.0,
            theme::tokens().palette(false),
        );
        assert!(texts(&list).contains(&"99+".to_owned()));
    }

    #[test]
    fn an_expanded_type_without_libraries_says_none() {
        let t = vec![("empty".to_owned(), "空类型".to_owned())];
        let e: HashSet<String> = ["empty".to_owned()].into_iter().collect();
        let m = model(&t, &[], &e, false);
        let lay = layout(&m, AREA, 0.0);
        let mut list = DrawList::new();
        paint(
            &mut list,
            AREA,
            &m,
            &lay,
            0.0,
            theme::tokens().palette(false),
        );
        assert!(texts(&list).contains(&"暂无".to_owned()));
    }

    #[test]
    fn the_new_library_field_replaces_the_none_row_and_shows_errors_below() {
        let t = vec![("empty".to_owned(), "空类型".to_owned())];
        let e: HashSet<String> = ["empty".to_owned()].into_iter().collect();
        let field = TextField::new("库名称");
        let mut m = model(&t, &[], &e, false);
        m.creating = Some(("empty", &field, "名称不能为空"));
        let lay = layout(&m, AREA, 0.0);
        assert!(lay.rect_of(NavHit::NewLibraryField).is_some());
        let mut list = DrawList::new();
        paint(
            &mut list,
            AREA,
            &m,
            &lay,
            0.0,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        assert!(!t.contains(&"暂无".to_owned()));
        assert!(t.contains(&"库名称".to_owned()), "占位文字");
        assert!(t.contains(&"名称不能为空".to_owned()));
    }

    #[test]
    fn the_settings_button_is_pinned_to_the_bottom_regardless_of_scroll() {
        let (t, l, e) = (types(), libraries(), expanded());
        let m = model(&t, &l, &e, false);
        let a = layout(&m, AREA, 0.0).rect_of(NavHit::Settings).unwrap();
        let b = layout(&m, AREA, 500.0).rect_of(NavHit::Settings).unwrap();
        assert_eq!(a, b);
        // px-2 py-2：距底 8
        assert_eq!(a.bottom, AREA.bottom - 8.0);
        assert!((a.height() - ITEM_HEIGHT).abs() < 0.01);
    }

    #[test]
    fn scrolling_moves_the_list_but_not_the_header() {
        let (t, l, e) = (types(), libraries(), expanded());
        let m = model(&t, &l, &e, false);
        let a = layout(&m, AREA, 0.0);
        let b = layout(&m, AREA, 10.0);
        assert_eq!(a.rect_of(NavHit::Search), b.rect_of(NavHit::Search));
        let ha = a.rect_of(NavHit::Item(NavItem::Home)).unwrap();
        let hb = b.rect_of(NavHit::Item(NavItem::Home)).unwrap();
        assert_eq!(ha.top - hb.top, 10.0);
    }

    #[test]
    fn rows_scrolled_out_of_the_list_area_cannot_be_hit() {
        // 把窗口压得很矮，让库行滚到设置按钮底下
        let (t, l, e) = (types(), libraries(), expanded());
        let m = model(&t, &l, &e, false);
        let short = Rect::new(0.0, 32.0, 220.0, 300.0);
        let lay = layout(&m, short, 0.0);
        assert!(
            lay.rect_of(NavHit::Library(1)).is_none(),
            "被设置块盖住的行不该能点"
        );
        assert!(lay.max_scroll() > 0.0);
    }

    #[test]
    fn clipped_portion_of_a_row_cannot_capture_header_or_footer_clicks() {
        let (t, l, e) = (types(), libraries(), expanded());
        let lay = layout(&model(&t, &l, &e, false), AREA, 10.0);
        let row = lay.rect_of(NavHit::Item(NavItem::Home)).unwrap();
        assert!(row.top < lay.scroll_area.top);
        assert_eq!(lay.hit(row.left + 20.0, lay.scroll_area.top - 2.0), None);
        assert_eq!(
            lay.hit(row.left + 20.0, lay.scroll_area.top + 2.0),
            Some(NavHit::Item(NavItem::Home))
        );
    }

    #[test]
    fn a_zero_sized_rail_produces_nothing() {
        let (t, l, e) = (types(), libraries(), expanded());
        let m = model(&t, &l, &e, false);
        let lay = layout(&m, Rect::ZERO, 0.0);
        assert!(lay.entries.is_empty());
        let mut list = DrawList::new();
        paint(
            &mut list,
            Rect::ZERO,
            &m,
            &lay,
            0.0,
            theme::tokens().palette(false),
        );
        assert!(list.is_empty());
    }
}
