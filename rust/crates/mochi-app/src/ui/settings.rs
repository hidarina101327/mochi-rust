//! 控件由 app_settings 描述表生成；新增设置应修改注册表。

use mochi_core::app_settings::{self, SettingDescriptor, SettingValue, SettingValueType};

use super::background;
use super::draw::{Align, DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::overlay_scrollbar::{Axis as ScrollAxis, Bar as Scrollbar};
use super::text::{self, Emphasis};
use super::theme::{self, Palette};
use super::widgets::{FieldLook, TextField};

/// 原生设置分页。
pub const TABS: &[(&str, &str, Icon)] = &[
    ("general", "通用", Icon::SETTINGS),
    ("web-clipper", "网页剪藏", Icon::BOOK_OPEN),
    ("notifications", "通知中心", Icon::BELL),
    ("shortcuts", "快捷键", Icon::KEYBOARD),
    ("version-history", "版本历史", Icon::HISTORY),
    ("customization", "自定义设置", Icon::PALETTE),
    ("ai", "AI 配置", Icon::SPARKLES),
    ("plugins", "第三方库", Icon::BLOCKS),
];

/// `customizationSections.ts`。
pub const CUSTOMIZATION_SECTIONS: &[(&str, &str)] = &[
    ("appearance", "外观"),
    ("interface", "界面字体与控件"),
    ("theme-colors", "明暗主题配色"),
    ("navigation", "导航栏"),
    ("area-backgrounds", "区域背景"),
    ("sidebar", "侧边栏"),
    ("tabs", "标签栏"),
    ("editor-layout", "编辑器布局"),
    ("typography", "排版"),
    ("headings", "标题样式"),
    ("code", "代码"),
    ("tables", "表格"),
    ("lists", "列表"),
    ("blockquote", "引用块"),
    ("spacing", "间距"),
    ("outline", "大纲"),
    ("assistant", "AI 对话外观"),
    ("dashboard", "首页与仪表盘"),
    ("schedule", "日程待办"),
    ("canvas", "画布"),
];

/// `KeyboardShortcutsSettings.tsx` 的两张表：可改的全局快捷键 + 固定的编辑器快捷键。
pub const GLOBAL_SHORTCUTS: &[(&str, &str, &str)] = &[
    ("Ctrl+P", "快速打开文件", "导航"),
    ("Ctrl+Shift+P", "命令面板", "导航"),
    ("Ctrl+Shift+F", "全局搜索", "导航"),
    ("Alt+ArrowLeft", "当前标签后退", "导航"),
    ("Alt+ArrowRight", "当前标签前进", "导航"),
    ("Ctrl+Alt+S", "打开日程", "导航"),
    ("Ctrl+Alt+D", "打开仪表盘", "导航"),
    ("Ctrl+Shift+L", "切换深色模式", "视图"),
    ("Ctrl+S", "保存文件", "文件操作"),
    ("Ctrl+O", "打开工作区", "文件操作"),
    ("Ctrl+N", "新建文件", "文件操作"),
    ("Ctrl+W", "关闭标签页", "文件操作"),
    ("Ctrl+Tab", "下一个标签页", "导航"),
    ("Ctrl+Shift+Tab", "上一个标签页", "导航"),
    ("Ctrl+J", "切换 AI 面板", "视图"),
    ("Ctrl+\\", "切换侧边栏", "视图"),
    ("Ctrl+Shift+C", "应用上次字体颜色", "编辑器"),
    ("Alt+Shift+C", "应用上次背景颜色", "编辑器"),
    ("Ctrl+Shift+E", "切换源码模式", "编辑器"),
    ("Ctrl+Shift+Space", "快速捕获", "全局"),
];
pub const EDITOR_SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+B", "加粗"),
    ("Ctrl+I", "斜体"),
    ("Ctrl+U", "下划线"),
    ("Ctrl+Shift+X", "删除线"),
    ("Ctrl+E", "行内代码"),
    ("Ctrl+Alt+1", "H1 标题"),
    ("Ctrl+Alt+2", "H2 标题"),
    ("Ctrl+Alt+3", "H3 标题"),
    ("Ctrl+Alt+4", "H4 标题"),
    ("Ctrl+Alt+5", "H5 标题"),
    ("Ctrl+Alt+6", "H6 标题"),
    ("Ctrl+Shift+8", "无序列表"),
    ("Ctrl+Shift+9", "有序列表"),
    ("Ctrl+Shift+7", "任务列表"),
    ("Ctrl+Shift+B", "代码块"),
    ("Ctrl+Shift+>", "引用块"),
    ("Ctrl+Z", "撤销"),
    ("Ctrl+Shift+Z", "重做"),
    ("Ctrl+Space", "触发 AI 内联补全"),
    ("Tab", "接受内联补全"),
    ("Ctrl+L", "接受补全一行"),
    ("Ctrl+K", "接受补全一个字符"),
    ("Esc", "取消内联补全"),
];

pub fn tab_label(id: &str) -> &'static str {
    TABS.iter()
        .find(|(t, _, _)| *t == id)
        .map(|(_, l, _)| *l)
        .unwrap_or("设置")
}

/// 旧版本的描述表把区域配色和背景图片都标成了 `appearance`。保留描述表
/// 的原始值给设置快照使用，在原生设置页按实际产品分区归类，避免「区域背景」
/// 只剩一个空页面。
pub fn customization_section_for(d: &SettingDescriptor) -> &str {
    if d.category == "background"
        || matches!(
            d.key.as_str(),
            "appearance.navigationBackground"
                | "appearance.sidebarBackground"
                | "appearance.mainBackground"
                | "appearance.aiPanelBackground"
                | "appearance.areaBackgroundOpacity"
        )
    {
        "area-backgrounds"
    } else {
        d.ui_section.as_deref().unwrap_or("appearance")
    }
}

pub fn is_customization_section(id: &str) -> bool {
    CUSTOMIZATION_SECTIONS
        .iter()
        .any(|(section, _)| *section == id)
}

/// 自定义设置始终使用同一份连续内容。调用方可以用分区 ID 找到锚点，
/// 但不会因为点击侧栏而换成另一张内容页。
pub fn customization_groups() -> Vec<(
    &'static str,
    Vec<(&'static str, Vec<&'static SettingDescriptor>)>,
)> {
    CUSTOMIZATION_SECTIONS
        .iter()
        .map(|(section, _)| (*section, descriptors_for("customization", Some(section))))
        .collect()
}

/// 某分页（及分区）下要显示的描述符，按类别分组、保持描述表顺序。
pub fn descriptors_for(
    tab: &str,
    section: Option<&str>,
) -> Vec<(&'static str, Vec<&'static SettingDescriptor>)> {
    let mut groups: Vec<(&'static str, Vec<&'static SettingDescriptor>)> = Vec::new();
    for d in app_settings::descriptors() {
        // 由专用界面和浏览器授权管理，不接受自由填写。
        if matches!(
            d.key.as_str(),
            super::navigation_preferences::ORDER_KEY
                | super::navigation_preferences::HIDDEN_KEY
                | "webClipper.extensionIds"
        ) {
            continue;
        }
        if d.ui_tab != tab {
            continue;
        }
        if tab == "customization" {
            let want = section.unwrap_or("appearance");
            if customization_section_for(d) != want {
                continue;
            }
        }
        match groups.iter_mut().find(|(c, _)| *c == d.category.as_str()) {
            Some((_, v)) => v.push(d),
            None => groups.push((d.category.as_str(), vec![d])),
        }
    }
    groups
}

/// 筛选当前显示设置页对应的描述注册表。
/// 标签、说明、键名、分类和枚举标签都可搜索，
/// 这样用户即使只记得某项设置的作用，也能找到它。
pub fn matching_descriptors_for(
    tab: &str,
    section: Option<&str>,
    query: &str,
) -> Vec<(&'static str, Vec<&'static SettingDescriptor>)> {
    let query = query.trim();
    descriptors_for(tab, section)
        .into_iter()
        .filter_map(|(category, items)| {
            let items = items
                .into_iter()
                .filter(|descriptor| descriptor_matches(descriptor, query))
                .collect::<Vec<_>>();
            (!items.is_empty()).then_some((category, items))
        })
        .collect()
}

/// 全局搜索结果按分类分组，且保持设置注册表中的顺序。
pub fn matching_descriptors(query: &str) -> Vec<(&'static str, Vec<&'static SettingDescriptor>)> {
    let mut groups: Vec<(&'static str, Vec<&'static SettingDescriptor>)> = Vec::new();
    for descriptor in app_settings::descriptors() {
        // 这两个持久化值由导航专用卡片一起编辑。
        if matches!(
            descriptor.key.as_str(),
            super::navigation_preferences::ORDER_KEY | super::navigation_preferences::HIDDEN_KEY
        ) || !descriptor_matches(descriptor, query)
        {
            continue;
        }
        match groups
            .iter_mut()
            .find(|(category, _)| *category == descriptor.category.as_str())
        {
            Some((_, descriptors)) => descriptors.push(descriptor),
            None => groups.push((descriptor.category.as_str(), vec![descriptor])),
        }
    }
    groups
}

fn descriptor_matches(descriptor: &SettingDescriptor, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut searchable = format!(
        "{} {} {} {} {}",
        descriptor.label,
        descriptor.description.as_deref().unwrap_or(""),
        descriptor.key,
        descriptor.category,
        app_settings::category_label(&descriptor.category).unwrap_or(&descriptor.category),
    );
    if descriptor.ui_tab == "customization" {
        searchable.push(' ');
        searchable.push_str(
            CUSTOMIZATION_SECTIONS
                .iter()
                .find(|(id, _)| *id == customization_section_for(descriptor))
                .map(|(_, label)| *label)
                .unwrap_or(""),
        );
    }
    for option in descriptor.options.iter().flatten() {
        searchable.push(' ');
        searchable.push_str(option);
        searchable.push(' ');
        searchable.push_str(&enum_option_label(descriptor, option));
    }
    searchable.to_lowercase().contains(&query.to_lowercase())
}

// ---------- 左侧导航 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavHit {
    Tab(usize),
    Section(usize),
    Scrollbar,
}

#[derive(Debug, Clone, Default)]
pub struct NavLayout {
    pub entries: Vec<(Rect, NavHit)>,
    /// 固定标题以下的可滚动区域。绘制与命中测试共用它，避免点到被裁掉的行。
    pub scroll_area: Rect,
    pub content_height: f32,
    pub scroll: f32,
    pub scrollbar: Option<Scrollbar>,
}

impl NavLayout {
    pub fn hit(&self, x: f32, y: f32) -> Option<NavHit> {
        if !self.scroll_area.contains(x, y) {
            return None;
        }
        self.entries
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h)
            .or_else(|| {
                self.scrollbar
                    .is_some_and(|bar| bar.hotzone.contains(x, y))
                    .then_some(NavHit::Scrollbar)
            })
    }
    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.scroll_area.height()).max(0.0)
    }
    #[cfg(test)]
    pub fn rect_of(&self, h: NavHit) -> Option<Rect> {
        self.entries.iter().find(|(_, x)| *x == h).map(|(r, _)| *r)
    }
}

pub fn nav_layout(area: Rect, active_tab: &str, scroll: f32) -> NavLayout {
    let mut out = NavLayout::default();
    if area.is_empty() {
        return out;
    }
    // 标题和分隔线固定；滚动视口从原本第一行的位置开始，保证默认布局不变。
    let top = (area.top + 65.0).min(area.bottom);
    out.scroll_area = Rect::new(area.left, top, area.right, area.bottom);
    let tab_height = 32.0_f32.max(TextStyle::Label.line_height());
    let section_height = 28.0_f32.max(TextStyle::Caption.line_height());
    let mut y = top;
    let x0 = area.left + 8.0;
    let x1 = area.right - 8.0;
    for (i, (id, _, _)) in TABS.iter().enumerate() {
        out.entries
            .push((Rect::new(x0, y, x1, y + tab_height), NavHit::Tab(i))); // py-1.5 + 20
        y += tab_height + 2.0; // space-y-0.5
        if *id == "customization" && active_tab == "customization" {
            y += 2.0; // mt-1（叠上 space-y）
            for (si, _) in CUSTOMIZATION_SECTIONS.iter().enumerate() {
                out.entries.push((
                    Rect::new(x0, y, x1, y + section_height),
                    NavHit::Section(si),
                )); // py-1.5 + 16
                y += section_height + 2.0;
            }
        }
    }
    out.content_height = (y - top).max(0.0);
    out.scroll = scroll.clamp(0.0, out.max_scroll());
    for (rect, _) in &mut out.entries {
        rect.top -= out.scroll;
        rect.bottom -= out.scroll;
    }

    out.scrollbar = Scrollbar::new(
        out.scroll_area,
        ScrollAxis::Vertical,
        out.max_scroll(),
        out.scroll,
        false,
    );
    out
}

pub fn paint_nav(
    list: &mut DrawList,
    area: Rect,
    lay: &NavLayout,
    active_tab: &str,
    active_section: &str,
    pointer: Option<(f32, f32)>,
    p: &Palette,
) {
    if area.is_empty() {
        return;
    }
    list.push_clip(area);
    list.rect(area, p.surface);
    list.border_right(area, p.border);
    let head_height = 20.0_f32.max(TextStyle::Large.line_height());
    let head = Rect::new(
        area.left + 12.0,
        area.top + 12.0,
        area.right - 12.0,
        area.top + 12.0 + head_height,
    );
    list.icon_centered(
        Rect::new(head.left, head.top, head.left + 16.0, head.bottom),
        Icon::SETTINGS,
        16.0,
        p.muted,
    );
    list.text(
        Rect::new(head.left + 24.0, head.top, head.right, head.bottom),
        "设置",
        TextStyle::Label,
        p.muted,
    );
    list.hline(area.left, area.right, area.top + 56.0, p.border);

    list.push_clip(lay.scroll_area);
    for (r, hit) in &lay.entries {
        match hit {
            NavHit::Tab(i) => {
                let (id, label, icon) = TABS[*i];
                let active = id == active_tab;
                if active {
                    list.rounded_rect(*r, 6.0, p.surface_muted);
                }
                let color = if active { p.foreground } else { p.muted };
                list.icon_centered(
                    Rect::new(r.left + 8.0, r.top, r.left + 24.0, r.bottom),
                    icon,
                    16.0,
                    color,
                );
                list.text_run(
                    Rect::new(r.left + 32.0, r.top, r.right - 24.0, r.bottom),
                    label,
                    TextStyle::Label,
                    color,
                    Align::Leading,
                    if active {
                        Emphasis::Bold
                    } else {
                        Emphasis::None
                    },
                );
                if id == "customization" {
                    let chevron = if active {
                        Icon::CHEVRON_DOWN
                    } else {
                        Icon::CHEVRON_RIGHT
                    };
                    list.icon_centered(
                        Rect::new(r.right - 22.0, r.top, r.right - 8.0, r.bottom),
                        chevron,
                        14.0,
                        color,
                    );
                }
            }
            NavHit::Section(si) => {
                let (id, label) = CUSTOMIZATION_SECTIONS[*si];
                let active = id == active_section;
                if active {
                    list.rounded_rect(*r, 4.0, p.surface_muted);
                }
                let color = if active { p.foreground } else { p.muted };
                list.text_run(
                    Rect::new(r.left + 40.0, r.top, r.right - 8.0, r.bottom),
                    label,
                    TextStyle::Caption,
                    color,
                    Align::Leading,
                    if active {
                        Emphasis::Bold
                    } else {
                        Emphasis::None
                    },
                );
            }
            NavHit::Scrollbar => {}
        }
    }
    list.pop_clip();
    if let Some(bar) = lay.scrollbar.filter(|bar| {
        super::settings_values::boolean("scrollbar.alwaysVisible", false)
            || pointer.is_some_and(|(x, y)| bar.hotzone.contains(x, y))
    }) {
        list.push_clip(bar.hotzone);
        list.rounded_rect_alpha(bar.track, 3.0, p.foreground, 0.06);
        list.rounded_rect_alpha(
            bar.thumb,
            3.0,
            super::settings_values::color("scrollbar.color", p.foreground),
            super::settings_values::number("scrollbar.opacity", 38.0) / 100.0,
        );
        list.pop_clip();
    }
    list.pop_clip();
}

// ---------- 右侧内容 ----------

#[derive(Debug, Clone, Default)]
pub struct ContentLayout {
    pub navigation: super::navigation_preferences::Layout,
    pub shortcuts: Vec<(Rect, usize)>,
    /// (控件矩形, 描述符在 `descriptors()` 里的下标)
    pub controls: Vec<(Rect, usize)>,
    pub body: Rect,
    pub content_height: f32,
    /// 自定义设置各分区相对于正文起点的滚动锚点。
    pub section_anchors: Vec<(&'static str, f32)>,
    /// 连续正文右侧的滚动条。零矩形表示内容不需要滚动。
    pub scrollbar_track: Rect,
    pub scrollbar_thumb: Rect,
    /// 背景图片选择/清除按钮，随「区域背景」分区一起滚动。
    pub background_buttons: Vec<(Rect, &'static str)>,
    /// 设置页顶部的搜索框。搜索文字保存在 `App::prefs` 中。
    pub search_rect: Rect,
    /// 通用设置页的原生 Rust 更新检查按钮。
    pub update_check_rect: Rect,
}

impl ContentLayout {
    /// 重置按钮与绘制共用矩形，滚出正文的按钮不可点击。
    pub fn reset_hit(&self, x: f32, y: f32) -> Option<usize> {
        if !self.body.contains(x, y) {
            return None;
        }
        self.controls
            .iter()
            .find(|(rect, _)| reset_rect(*rect).contains(x, y))
            .map(|(_, index)| *index)
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        if !self.body.contains(x, y) {
            return None;
        }
        self.controls
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, i)| *i)
    }
    pub fn control_rect(&self, index: usize) -> Option<Rect> {
        self.controls
            .iter()
            .find(|(_, i)| *i == index)
            .map(|(r, _)| *r)
    }
    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.body.height()).max(0.0)
    }

    pub fn section_scroll(&self, section: &str) -> Option<f32> {
        self.section_anchors
            .iter()
            .find(|(id, _)| *id == section)
            .map(|(_, offset)| offset.clamp(0.0, self.max_scroll()))
    }

    pub fn section_at_scroll(&self, scroll: f32) -> Option<&'static str> {
        self.section_anchors
            .iter()
            .rev()
            .find(|(_, offset)| *offset <= scroll.max(0.0) + 1.0)
            .map(|(id, _)| *id)
    }
}

const HEADER_H: f32 = 16.0 + 28.0 + 16.0 + 1.0;
const ROW_H: f32 = 16.0 + 20.0 + 4.0 + 16.0 + 16.0; // p-4 + 标签 20 + mt-1 + 描述 16 + p-4
const ROW_H_NO_DESC: f32 = 16.0 + 20.0 + 16.0;
const SECTION_GAP: f32 = 32.0; // space-y-8
const CUSTOM_SECTION_GAP: f32 = 24.0;
const EMPTY_SECTION_H: f32 = 44.0;
const SCROLLBAR_MIN_THUMB: f32 = 28.0;
const UPDATE_CARD_H: f32 = 104.0;

fn reset_rect(control: Rect) -> Rect {
    Rect::new(
        control.left - 48.0,
        control.top,
        control.left - 8.0,
        control.bottom,
    )
}

/// 供共享使用的预留区域；绘制和指针事件分派都使用这个矩形。
pub fn close_rect(area: Rect) -> Rect {
    Rect::from_size(area.right - 44.0, area.top + 14.0, 32.0, 32.0)
}

fn header_height(area: Rect, searchable: bool) -> f32 {
    let extra_title_height = (TextStyle::Large.line_height() - 28.0).max(0.0);
    if searchable && area.width() < 560.0 {
        108.0 + extra_title_height
    } else {
        HEADER_H + extra_title_height
    }
}

fn large_title_height() -> f32 {
    TextStyle::Large.line_height().max(28.0)
}

fn section_head_height() -> f32 {
    large_title_height() + 4.0 + 20.0 + 16.0
}

fn custom_section_head_height() -> f32 {
    large_title_height() + 4.0 + 20.0
}

fn descriptor_index(d: &SettingDescriptor) -> usize {
    app_settings::descriptors()
        .iter()
        .position(|x| std::ptr::eq(x, d))
        .unwrap_or(0)
}

fn row_height(d: &SettingDescriptor) -> f32 {
    let base = if d.description.is_some() {
        ROW_H
    } else {
        ROW_H_NO_DESC
    };
    base + (setting_label_height() - 20.0)
        + if d.description.is_some() {
            setting_description_height() - 16.0
        } else {
            0.0
        }
}

fn setting_label_height() -> f32 {
    (TextStyle::Label.font_size() * (20.0 / 14.0)).max(20.0)
}
fn setting_description_height() -> f32 {
    (TextStyle::Caption.font_size() * (16.0 / 11.5)).max(16.0)
}

fn control_width(d: &SettingDescriptor) -> f32 {
    match d.value_type {
        SettingValueType::Boolean => 44.0,
        SettingValueType::Enum => {
            let widest = d
                .options
                .iter()
                .flatten()
                .map(|o| text::measure(&enum_option_label(d, o), TextStyle::Label))
                .fold(0.0, f32::max);
            widest + 24.0 + 20.0
        }
        SettingValueType::Number => {
            80.0 + d
                .unit
                .as_ref()
                .map(|u| text::measure(u, TextStyle::Label) + 8.0)
                .unwrap_or(0.0)
        }
        SettingValueType::String | SettingValueType::Color => 160.0,
    }
}

pub fn restore_scroll(saved: Option<&str>, max_scroll: f32) -> f32 {
    let saved = saved
        .and_then(|raw| raw.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(0.0);
    saved.clamp(0.0, max_scroll.max(0.0))
}

fn scrollbar_for(body: Rect, content_height: f32, scroll: f32) -> (Rect, Rect) {
    if body.is_empty() || content_height <= body.height() + 0.5 {
        return (Rect::ZERO, Rect::ZERO);
    }
    let track = Rect::new(
        body.right - 12.0,
        body.top + 8.0,
        body.right - 6.0,
        body.bottom - 8.0,
    );
    if track.is_empty() {
        return (Rect::ZERO, Rect::ZERO);
    }
    let min_thumb = SCROLLBAR_MIN_THUMB.min(track.height());
    let thumb_height = (track.height() * body.height() / content_height)
        .max(min_thumb)
        .min(track.height());
    let range = (track.height() - thumb_height).max(0.0);
    let max_scroll = (content_height - body.height()).max(0.0);
    let progress = if max_scroll > 0.0 {
        (scroll / max_scroll).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let top = track.top + range * progress;
    (
        track,
        Rect::new(track.left, top, track.right, top + thumb_height),
    )
}

fn groups_height(groups: &[(&'static str, Vec<&'static SettingDescriptor>)]) -> f32 {
    if groups.is_empty() {
        return EMPTY_SECTION_H;
    }
    groups
        .iter()
        .enumerate()
        .map(|(index, (_, items))| {
            let gap = if index == 0 { 0.0 } else { SECTION_GAP };
            gap + section_head_height()
                + 1.0
                + items
                    .iter()
                    .map(|descriptor| row_height(descriptor) + 1.0)
                    .sum::<f32>()
        })
        .sum()
}

fn layout_group_controls(
    x1: f32,
    mut y: f32,
    groups: &[(&'static str, Vec<&'static SettingDescriptor>)],
    controls: &mut Vec<(Rect, usize)>,
) {
    for (gi, (_, items)) in groups.iter().enumerate() {
        if gi > 0 {
            y += SECTION_GAP;
        }
        y += section_head_height();
        y += 1.0; // 卡片上边框
        for d in items {
            let h = row_height(d);
            let w = control_width(d);
            let control_h = match d.value_type {
                SettingValueType::Boolean => 24.0,
                _ => 36.0,
            };
            let cy = y + h / 2.0;
            controls.push((
                Rect::new(
                    x1 - 16.0 - w,
                    cy - control_h / 2.0,
                    x1 - 16.0,
                    cy + control_h / 2.0,
                ),
                descriptor_index(d),
            ));
            y += h + 1.0; // divide-y
        }
    }
}

pub fn content_layout(area: Rect, tab: &str, section: Option<&str>, scroll: f32) -> ContentLayout {
    content_layout_filtered(area, tab, section, scroll, "")
}

pub fn content_layout_filtered(
    area: Rect,
    tab: &str,
    section: Option<&str>,
    scroll: f32,
    query: &str,
) -> ContentLayout {
    let mut out = ContentLayout::default();
    if area.is_empty() {
        return out;
    }
    let searchable = tab != "shortcuts" && tab != "plugins";
    let body = Rect::new(
        area.left,
        area.top + header_height(area, searchable),
        area.right,
        area.bottom,
    );
    out.body = body;
    if searchable {
        out.search_rect = if area.width() < 560.0 {
            let top = area.top + 60.0 + (TextStyle::Large.line_height() - 28.0).max(0.0);
            Rect::new(area.left + 24.0, top, area.right - 24.0, top + 36.0)
        } else {
            let right = close_rect(area).left - 12.0;
            Rect::new(right - 280.0, area.top + 12.0, right, area.top + 48.0)
        };
    }
    let y0 = body.top + 24.0;
    let x1 = body.right - 24.0;
    if tab == "shortcuts" || tab == "plugins" {
        out.content_height = shortcuts_height(tab);
        if tab == "shortcuts" {
            for i in 0..GLOBAL_SHORTCUTS.len() {
                let top = y0 - scroll + section_head_height() + 1.0 + i as f32 * 45.0;
                out.shortcuts
                    .push((Rect::new(x1 - 260.0, top, x1, top + 44.0), i));
            }
        }
        (out.scrollbar_track, out.scrollbar_thumb) =
            scrollbar_for(body, out.content_height, scroll);
        return out;
    }

    if !query.trim().is_empty() {
        let groups = matching_descriptors(query);
        let mut y = y0 - scroll;
        layout_group_controls(x1, y, &groups, &mut out.controls);
        y += groups_height(&groups) + 24.0;
        out.content_height = y + scroll - body.top;
        (out.scrollbar_track, out.scrollbar_thumb) =
            scrollbar_for(body, out.content_height, scroll);
        return out;
    }

    if tab == "customization" {
        let section_groups = customization_groups();
        let mut cursor = 0.0;
        let mut shown_sections = 0usize;
        for (section_id, _) in section_groups {
            let groups = matching_descriptors_for("customization", Some(section_id), query);
            if !query.trim().is_empty() && groups.is_empty() {
                continue;
            }
            if shown_sections > 0 {
                cursor += CUSTOM_SECTION_GAP;
            }
            shown_sections += 1;
            out.section_anchors.push((section_id, cursor));
            let section_top = y0 + cursor - scroll;
            if section_id == "area-backgrounds" {
                out.background_buttons = background::buttons_at(body, section_top).to_vec();
            }
            cursor += custom_section_head_height();
            if section_id == "navigation" && query.trim().is_empty() {
                out.navigation = super::navigation_preferences::layout(
                    body.left + 24.0,
                    x1,
                    y0 + cursor - scroll,
                    &super::navigation_preferences::Preferences::read(),
                );
                cursor += super::navigation_preferences::height();
            }
            if groups.is_empty() {
                cursor += EMPTY_SECTION_H;
            } else {
                layout_group_controls(x1, y0 + cursor - scroll, &groups, &mut out.controls);
                cursor += groups_height(&groups);
            }
            cursor += 24.0;
        }
        out.content_height = 24.0 + cursor;
        (out.scrollbar_track, out.scrollbar_thumb) =
            scrollbar_for(body, out.content_height, scroll);
        return out;
    }

    let mut y = y0 - scroll;
    let groups = matching_descriptors_for(tab, section, query);
    if tab == "general" {
        out.update_check_rect = Rect::new(x1 - 156.0, y + 48.0, x1 - 16.0, y + 84.0);
        y += UPDATE_CARD_H + SECTION_GAP;
    }
    layout_group_controls(x1, y, &groups, &mut out.controls);
    y += groups_height(&groups) + 24.0;
    out.content_height = y + scroll - body.top;
    (out.scrollbar_track, out.scrollbar_thumb) = scrollbar_for(body, out.content_height, scroll);
    out
}

fn shortcuts_height(tab: &str) -> f32 {
    if tab == "plugins" {
        return 120.0;
    }
    // 两个 section：标题 + 每行 44
    24.0 + section_head_height()
        + GLOBAL_SHORTCUTS.len() as f32 * 45.0
        + SECTION_GAP
        + section_head_height()
        + EDITOR_SHORTCUTS.len() as f32 * 45.0
        + 24.0
}

/// 一行设置的当前值怎么显示。
pub fn enum_option_label(d: &SettingDescriptor, value: &str) -> String {
    let label = match (d.key.as_str(), value) {
        ("appearance.themeMode", "light") => "亮色",
        ("appearance.themeMode", "dark") => "深色",
        ("appearance.themeMode", "auto") => "跟随系统",
        ("sidebar.treeGuideStyle" | "outline.guideStyle", "solid") => "实线",
        ("sidebar.treeGuideStyle" | "outline.guideStyle", "dashed") => "虚线",
        ("sidebar.sortOrder", "manual") => "自定义（拖拽顺序）",
        ("sidebar.sortOrder", "name") => "名称升序",
        ("sidebar.sortOrder", "name-desc") => "名称降序",
        ("tabs.openFileBehavior", "replace") => "复用当前标签页",
        ("tabs.openFileBehavior", "new-tab") => "新建标签页",
        ("tabs.newTabPosition", "after-active") => "当前标签之后",
        ("tabs.newTabPosition", "end") => "末尾",
        ("tabs.showCloseButtons", "hover") => "悬停时显示",
        ("tabs.showCloseButtons", "always") => "始终显示",
        ("code.style", "adaptive") => "跟随主题",
        ("code.style", "dark") => "深色",
        ("code.style", "paper") => "纸张",
        ("code.style", "terminal") => "终端",
        ("editorLayout.alignment", "left") => "居左",
        ("editorLayout.alignment", "center") => "居中",
        ("editorLayout.alignment", "right") => "居右",
        ("code.overflow", "wrap") => "自动换行",
        ("code.overflow", "scroll") => "横向滚动条",
        ("outline.placement", "left-sidebar") => "左侧栏",
        ("outline.placement", "editor-right") => "文件区内",
        ("outline.placement", "ai-sidebar") => "右侧边栏",
        ("ai.editApplyMode", "approve") => "先批准后应用",
        ("ai.editApplyMode", "auto") => "自动直接应用",
        ("tables.baseDefaultMode", "edit") => "编辑模式",
        ("tables.baseDefaultMode", "view") => "查看模式",
        _ => value,
    };
    label.into()
}

/// 一行设置的当前值怎么显示。
fn value_label(d: &SettingDescriptor, v: &SettingValue) -> String {
    match v {
        SettingValue::Bool(b) => b.to_string(),
        SettingValue::Number(n) => {
            let s = if n.fract() == 0.0 {
                format!("{}", *n as i64)
            } else {
                format!("{n}")
            };
            let _ = d;
            s
        }
        SettingValue::Text(s) if d.value_type == SettingValueType::Enum => enum_option_label(d, s),
        SettingValue::Text(s) => s.clone(),
    }
}

/// 正在编辑的输入框（数字/文本/颜色）。
pub struct EditingField {
    pub index: usize,
    pub field: TextField,
}

pub struct ContentModel<'a> {
    pub tab: &'a str,
    pub section: Option<&'a str>,
    pub query: &'a str,
    /// 读某个描述符的当前值。
    pub read: &'a dyn Fn(&SettingDescriptor) -> SettingValue,
    pub editing: Option<&'a EditingField>,
}

pub fn paint_content(
    list: &mut DrawList,
    area: Rect,
    lay: &ContentLayout,
    model: &ContentModel,
    scroll: f32,
    p: &Palette,
) {
    if area.is_empty() {
        return;
    }
    list.push_clip(area);
    list.rect(area, p.surface);
    let head = Rect::new(area.left, area.top, area.right, lay.body.top);
    list.text(
        Rect::new(
            head.left + 24.0,
            head.top + 16.0,
            if lay.search_rect.is_empty() || lay.search_rect.top >= area.top + 60.0 {
                close_rect(area).left - 12.0
            } else {
                lay.search_rect.left - 16.0
            },
            head.top + 16.0 + large_title_height(),
        ),
        if model.query.trim().is_empty() {
            tab_label(model.tab)
        } else {
            "搜索结果"
        },
        TextStyle::Large,
        p.foreground,
    );
    list.border_bottom(head, p.border);

    let body = lay.body;
    list.push_clip(body);
    let x0 = body.left + 24.0;
    let x1 = body.right - 24.0;
    let mut y = body.top + 24.0 - scroll;

    if !model.query.trim().is_empty() {
        let groups = matching_descriptors(model.query);
        if groups.is_empty() {
            list.text(
                Rect::new(x0, y + 12.0, x1, y + 36.0),
                "没有匹配的设置项",
                TextStyle::Label,
                p.muted,
            );
        } else {
            paint_descriptor_groups(list, x0, x1, y, &groups, lay, model, p);
        }
    } else {
        match model.tab {
            "shortcuts" => {
                let keys = GLOBAL_SHORTCUTS
                    .iter()
                    .map(|(key, description, category)| {
                        (super::shortcuts::binding(key), *description, *category)
                    })
                    .collect::<Vec<_>>();
                y = paint_shortcut_section(
                    list,
                    x0,
                    x1,
                    y,
                    "全局快捷键",
                    "点击组合键修改；编辑时留空可恢复默认。",
                    keys.iter().map(|(k, d, c)| (k.as_str(), *d, Some(*c))),
                    p,
                );
                y += SECTION_GAP;
                paint_shortcut_section(
                    list,
                    x0,
                    x1,
                    y,
                    "编辑器快捷键",
                    "编辑器内置的格式化快捷键，不可修改。",
                    EDITOR_SHORTCUTS.iter().map(|(k, d)| (*k, *d, None)),
                    p,
                );
            }
            "plugins" => {
                list.text(
                    Rect::new(x0, y, x1, y + large_title_height()),
                    "第三方库",
                    TextStyle::Large,
                    p.foreground,
                );
                list.text(
                    Rect::new(
                        x0,
                        y + large_title_height() + 4.0,
                        x1,
                        y + large_title_height() + 24.0,
                    ),
                    "原生版不加载插件（插件运行在 Web 引擎里，与本版的目标相悖）。",
                    TextStyle::Label,
                    p.muted,
                );
            }
            _ => {
                if model.tab == "customization" {
                    let mut shown_sections = 0usize;
                    for (section_id, section_label) in CUSTOMIZATION_SECTIONS {
                        let groups = matching_descriptors_for(
                            "customization",
                            Some(section_id),
                            model.query,
                        );
                        if !model.query.trim().is_empty() && groups.is_empty() {
                            continue;
                        }
                        if shown_sections > 0 {
                            y += CUSTOM_SECTION_GAP;
                        }
                        shown_sections += 1;
                        let buttons: &[(Rect, &'static str)] = if *section_id == "area-backgrounds"
                        {
                            &lay.background_buttons
                        } else {
                            &[]
                        };
                        let heading_right = buttons
                            .first()
                            .map(|(rect, _)| rect.left - 12.0)
                            .unwrap_or(x1);
                        list.text(
                            Rect::new(x0, y, heading_right, y + large_title_height()),
                            *section_label,
                            TextStyle::Large,
                            p.foreground,
                        );
                        let section_description = if *section_id == "area-backgrounds" {
                            "区域颜色与自定义背景图片"
                        } else {
                            "自定义设置分区"
                        };
                        list.text(
                            Rect::new(
                                x0,
                                y + large_title_height() + 4.0,
                                heading_right,
                                y + large_title_height() + 24.0,
                            ),
                            section_description,
                            TextStyle::Label,
                            p.muted,
                        );
                        if *section_id == "area-backgrounds" {
                            paint_background_buttons(list, buttons, p);
                        }
                        y += custom_section_head_height();

                        if *section_id == "navigation" && model.query.trim().is_empty() {
                            super::navigation_preferences::paint(list, &lay.navigation, p);
                            y += super::navigation_preferences::height();
                        }
                        if groups.is_empty() {
                            list.text(
                                Rect::new(x0 + 16.0, y, x1 - 16.0, y + 32.0),
                                "此分区暂无可调整项",
                                TextStyle::Label,
                                p.muted,
                            );
                            y += EMPTY_SECTION_H;
                        } else {
                            y = paint_descriptor_groups(list, x0, x1, y, &groups, lay, model, p);
                        }
                        y += 24.0;
                    }
                } else {
                    let groups = matching_descriptors_for(model.tab, model.section, model.query);
                    if groups.is_empty() && !model.query.trim().is_empty() {
                        list.text(
                            Rect::new(x0, y + 12.0, x1, y + 36.0),
                            "没有匹配的设置项",
                            TextStyle::Label,
                            p.muted,
                        );
                    } else {
                        if model.tab == "general" {
                            paint_update_card(list, x0, x1, y, lay.update_check_rect, p);
                            y += UPDATE_CARD_H + SECTION_GAP;
                        }
                        paint_descriptor_groups(list, x0, x1, y, &groups, lay, model, p);
                    }
                }
            }
        }
    }
    if !lay.scrollbar_track.is_empty() {
        list.rounded_rect(
            lay.scrollbar_track,
            3.0,
            theme::mix(p.border, p.surface, 0.45),
        );
        list.rounded_rect(lay.scrollbar_thumb, 3.0, p.muted);
    }
    list.pop_clip();
    list.pop_clip();
}

fn paint_update_card(list: &mut DrawList, x0: f32, x1: f32, y: f32, button: Rect, p: &Palette) {
    let card = Rect::new(x0, y, x1, y + UPDATE_CARD_H);
    list.rounded_rect(card, 8.0, theme::mix(p.surface_muted, p.surface, 0.45));
    list.text(
        Rect::new(x0 + 16.0, y + 14.0, button.left - 16.0, y + 38.0),
        "应用更新",
        TextStyle::Label,
        p.foreground,
    );
    list.text(
        Rect::new(x0 + 16.0, y + 42.0, button.left - 16.0, y + 62.0),
        format!(
            "Rust 原生版当前 v{} · 启动时检查并安全下载更新",
            env!("CARGO_PKG_VERSION")
        ),
        TextStyle::Caption,
        p.muted,
    );
    super::workspace_ui::button(list, button, "检查更新", None, false, false, p);
}

fn paint_background_buttons(list: &mut DrawList, buttons: &[(Rect, &'static str)], p: &Palette) {
    for (rect, label) in buttons {
        list.rounded_rect(*rect, 6.0, p.surface);
        list.rounded_border(*rect, 6.0, p.border);
        list.text(
            Rect::new(rect.left + 8.0, rect.top, rect.right - 8.0, rect.bottom),
            *label,
            TextStyle::Label,
            p.foreground,
        );
    }
}

fn paint_descriptor_groups(
    list: &mut DrawList,
    x0: f32,
    x1: f32,
    mut y: f32,
    groups: &[(&'static str, Vec<&'static SettingDescriptor>)],
    lay: &ContentLayout,
    model: &ContentModel,
    p: &Palette,
) -> f32 {
    for (gi, (category, items)) in groups.iter().enumerate() {
        if gi > 0 {
            y += SECTION_GAP;
        }
        let label = app_settings::category_label(category).unwrap_or(category);
        list.text(
            Rect::new(x0, y, x1, y + large_title_height()),
            label,
            TextStyle::Large,
            p.foreground,
        );
        list.text(
            Rect::new(
                x0,
                y + large_title_height() + 4.0,
                x1,
                y + large_title_height() + 24.0,
            ),
            format!("{} 项设置", items.len()),
            TextStyle::Label,
            p.muted,
        );
        y += section_head_height();
        let card_h: f32 = items.iter().map(|d| row_height(d) + 1.0).sum::<f32>() + 1.0;
        let card = Rect::new(x0, y, x1, y + card_h);
        list.hline(card.left, card.right, card.top, p.border);
        list.hline(card.left, card.right, card.bottom, p.border);
        y += 1.0;
        for (ri, d) in items.iter().enumerate() {
            let h = row_height(d);
            if ri > 0 {
                list.hline(card.left + 1.0, card.right - 1.0, y - 1.0, p.border);
            }
            let index = descriptor_index(d);
            let control = lay.control_rect(index).unwrap_or(Rect::ZERO);
            let text_right = if control.is_empty() {
                x1 - 16.0
            } else {
                reset_rect(control).left - 12.0
            };
            let label = if d.key == "editorLayout.liveLineSource" {
                "当前块显示 Markdown 源码"
            } else {
                &d.label
            };
            let available = (text_right - x0 - 16.0).max(0.0);
            list.text_run(
                Rect::new(
                    x0 + 16.0,
                    y + 16.0,
                    text_right,
                    y + 16.0 + setting_label_height(),
                ),
                text::ellipsize(label, TextStyle::Label, available),
                TextStyle::Label,
                p.foreground,
                Align::Leading,
                Emphasis::Bold,
            );
            if let Some(desc) = &d.description {
                let desc = if d.key == "editorLayout.liveLineSource" {
                    "关闭时直接编辑富文本；开启后当前块显示 Markdown 源码。"
                } else {
                    desc
                };
                list.text(
                    Rect::new(
                        x0 + 16.0,
                        y + 20.0 + setting_label_height(),
                        text_right,
                        y + 20.0 + setting_label_height() + setting_description_height(),
                    ),
                    text::ellipsize(desc, TextStyle::Caption, available),
                    TextStyle::Caption,
                    p.muted,
                );
            }
            let value = (model.read)(d);
            list.text_aligned(
                reset_rect(control),
                "重置",
                TextStyle::Caption,
                if value == d.default_value {
                    p.muted
                } else {
                    p.accent
                },
                Align::Center,
            );
            paint_control(
                list,
                control,
                d,
                &value,
                model.editing.filter(|e| e.index == index),
                p,
            );
            y += h + 1.0;
        }
    }
    y
}

fn paint_shortcut_section<'a>(
    list: &mut DrawList,
    x0: f32,
    x1: f32,
    mut y: f32,
    title: &str,
    desc: &str,
    rows: impl Iterator<Item = (&'a str, &'a str, Option<&'a str>)>,
    p: &Palette,
) -> f32 {
    list.text(
        Rect::new(x0, y, x1, y + large_title_height()),
        title,
        TextStyle::Large,
        p.foreground,
    );
    list.text(
        Rect::new(
            x0,
            y + large_title_height() + 4.0,
            x1,
            y + large_title_height() + 24.0,
        ),
        desc,
        TextStyle::Label,
        p.muted,
    );
    y += section_head_height();
    let rows: Vec<_> = rows.collect();
    let card = Rect::new(x0, y, x1, y + rows.len() as f32 * 45.0 + 1.0);
    list.rounded_rect(card, 12.0, p.background);
    list.rounded_border(card, 12.0, p.border);
    y += 1.0;
    for (i, (key, d, cat)) in rows.iter().enumerate() {
        if i > 0 {
            list.hline(card.left + 1.0, card.right - 1.0, y - 1.0, p.border);
        }
        let w = text::measure(key, TextStyle::Mono) + 16.0;
        let kbd = Rect::new(x1 - 16.0 - w, y + 10.0, x1 - 16.0, y + 34.0);
        let cat_right = kbd.left - 12.0;
        let text_right = if cat.is_some() {
            cat_right - 92.0
        } else {
            cat_right
        };
        list.text(
            Rect::new(x0 + 16.0, y, text_right, y + 44.0),
            text::ellipsize(d, TextStyle::Label, (text_right - x0 - 16.0).max(0.0)),
            TextStyle::Label,
            p.foreground,
        );
        if let Some(c) = cat {
            list.text_aligned(
                Rect::new(cat_right - 80.0, y, cat_right, y + 44.0),
                *c,
                TextStyle::Caption,
                p.muted,
                Align::Trailing,
            );
        }
        // kbd：等宽小字、圆角边框
        list.rounded_rect(kbd, 4.0, p.surface);
        list.rounded_border(kbd, 4.0, p.border);
        list.text_aligned(kbd, *key, TextStyle::Mono, p.foreground, Align::Center);
        y += 45.0;
    }
    y
}

pub fn sync_editing_scroll(layout: &ContentLayout, editing: Option<&mut EditingField>) {
    let Some(edit) = editing else { return };
    let Some(rect) = layout.control_rect(edit.index) else {
        return;
    };
    let unit = app_settings::descriptors()
        .get(edit.index)
        .and_then(|d| d.unit.as_ref())
        .map(|u| text::measure(u, TextStyle::Label) + 8.0)
        .unwrap_or(0.0);
    edit.field
        .sync_singleline_scroll((rect.width() - unit - 24.0).max(1.0));
}

fn paint_control(
    list: &mut DrawList,
    r: Rect,
    d: &SettingDescriptor,
    value: &SettingValue,
    editing: Option<&EditingField>,
    p: &Palette,
) {
    if r.is_empty() {
        return;
    }
    match d.value_type {
        SettingValueType::Boolean => {
            let on = matches!(value, SettingValue::Bool(true));
            list.rounded_rect(r, r.height() / 2.0, if on { p.accent } else { p.border });
            let knob_x = if on {
                r.right - 4.0 - 16.0
            } else {
                r.left + 4.0
            };
            list.rounded_rect(
                Rect::new(knob_x, r.top + 4.0, knob_x + 16.0, r.bottom - 4.0),
                8.0,
                if on { p.accent_foreground } else { p.surface },
            );
        }
        SettingValueType::Enum => {
            list.rounded_rect(r, 8.0, p.surface);
            list.rounded_border(r, 8.0, p.border);
            list.text(
                Rect::new(r.left + 12.0, r.top, r.right - 24.0, r.bottom),
                value_label(d, value),
                TextStyle::Label,
                p.foreground,
            );
            list.icon_centered(
                Rect::new(r.right - 22.0, r.top, r.right - 8.0, r.bottom),
                Icon::CHEVRON_DOWN,
                14.0,
                p.muted,
            );
        }
        SettingValueType::Number => {
            let unit_w = d
                .unit
                .as_ref()
                .map(|u| text::measure(u, TextStyle::Label) + 8.0)
                .unwrap_or(0.0);
            let field = Rect::new(r.left, r.top, r.right - unit_w, r.bottom);
            match editing {
                Some(e) => {
                    let mut f = e.field.clone();
                    f.paint(list, field, true, p, FieldLook::dialog(p));
                }
                None => {
                    list.rounded_rect(field, 4.0, p.surface);
                    list.rounded_border(field, 4.0, p.border);
                    list.text_aligned(
                        Rect::new(field.left + 8.0, field.top, field.right - 8.0, field.bottom),
                        value_label(d, value),
                        TextStyle::Label,
                        p.foreground,
                        Align::Trailing,
                    );
                }
            }
            if let Some(u) = &d.unit {
                list.text(
                    Rect::new(field.right + 8.0, r.top, r.right, r.bottom),
                    u.clone(),
                    TextStyle::Label,
                    p.muted,
                );
            }
        }
        SettingValueType::String | SettingValueType::Color => match editing {
            Some(e) => {
                let mut f = e.field.clone();
                f.paint(list, r, true, p, FieldLook::dialog(p));
            }
            None => {
                list.rounded_rect(r, 4.0, p.surface);
                list.rounded_border(r, 4.0, p.border);
                let mut text_left = r.left + 8.0;
                if d.value_type == SettingValueType::Color {
                    // 自动色用边框色占位；自定义值支持短十六进制和 RGB。
                    let swatch =
                        Rect::new(r.left + 8.0, r.top + 10.0, r.left + 24.0, r.bottom - 10.0);
                    let color = match value {
                        SettingValue::Text(s) => super::styles::color(s).unwrap_or(p.border),
                        _ => p.border,
                    };
                    list.rounded_rect(swatch, 4.0, color);
                    list.rounded_border(swatch, 4.0, p.border);
                    text_left += 24.0;
                }
                list.text(
                    Rect::new(text_left, r.top, r.right - 8.0, r.bottom),
                    text::ellipsize(
                        &value_label(d, value),
                        TextStyle::Label,
                        r.right - 8.0 - text_left,
                    ),
                    TextStyle::Label,
                    p.foreground,
                );
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::DrawCmd;

    const NAV: Rect = Rect {
        left: 220.0,
        top: 32.0,
        right: 480.0,
        bottom: 800.0,
    };
    const MAIN: Rect = Rect {
        left: 480.0,
        top: 69.0,
        right: 1200.0,
        bottom: 776.0,
    };

    #[test]
    fn every_descriptor_lands_on_exactly_one_tab_that_the_navigation_can_show() {
        // 描述表里 uiTab 的取值必须都在导航分页里，否则那些设置永远看不见
        let tabs: Vec<&str> = TABS.iter().map(|(id, _, _)| *id).collect();
        for d in app_settings::descriptors() {
            assert!(
                tabs.contains(&d.ui_tab.as_str()),
                "{} 的 uiTab={} 不在导航里",
                d.key,
                d.ui_tab
            );
        }
        // 自定义设置的每个 uiSection 也得在分区表里
        let sections: Vec<&str> = CUSTOMIZATION_SECTIONS.iter().map(|(id, _)| *id).collect();
        for d in app_settings::descriptors()
            .iter()
            .filter(|d| d.ui_tab == "customization")
        {
            let s = d.ui_section.as_deref().unwrap_or("");
            assert!(
                sections.contains(&s),
                "{} 的 uiSection={s} 不在分区表里",
                d.key
            );
        }
    }

    #[test]
    fn the_general_tab_groups_by_category_in_table_order() {
        let groups = descriptors_for("general", None);
        assert!(!groups.is_empty());
        let total: usize = groups.iter().map(|(_, v)| v.len()).sum();
        assert_eq!(
            total,
            app_settings::descriptors()
                .iter()
                .filter(|d| d.ui_tab == "general")
                .count()
        );
        // 同一类别只出现一次
        let mut seen = std::collections::HashSet::new();
        for (c, _) in &groups {
            assert!(seen.insert(*c), "{c} 重复分组");
        }
    }

    #[test]
    fn search_filters_settings_by_key_and_enum_label() {
        let by_key =
            matching_descriptors_for("customization", Some("editor-layout"), "liveLineSource");
        assert_eq!(
            by_key
                .iter()
                .flat_map(|(_, descriptors)| descriptors.iter())
                .map(|descriptor| descriptor.key.as_str())
                .collect::<Vec<_>>(),
            vec!["editorLayout.liveLineSource"]
        );

        let by_enum_label =
            matching_descriptors_for("customization", Some("appearance"), "跟随系统");
        assert!(by_enum_label
            .iter()
            .flat_map(|(_, descriptors)| descriptors.iter())
            .any(|descriptor| descriptor.key == "appearance.themeMode"));

        let global = content_layout_filtered(MAIN, "general", None, 0.0, "themeMode");
        let theme_index =
            descriptor_index(app_settings::descriptor("appearance.themeMode").unwrap());
        assert!(global
            .controls
            .iter()
            .any(|(_, index)| *index == theme_index));

        let no_results = content_layout_filtered(
            MAIN,
            "customization",
            Some("appearance"),
            0.0,
            "this-setting-does-not-exist",
        );
        assert!(no_results.controls.is_empty());
        assert!(!no_results.search_rect.is_empty());
    }

    #[test]
    fn customization_filters_by_section() {
        let appearance = descriptors_for("customization", Some("appearance"));
        assert!(appearance
            .iter()
            .flat_map(|(_, v)| v)
            .all(|d| d.ui_section.as_deref() == Some("appearance")));
        let backgrounds = descriptors_for("customization", Some("area-backgrounds"));
        let background_keys: Vec<_> = backgrounds
            .iter()
            .flat_map(|(_, values)| values)
            .map(|d| d.key.as_str())
            .collect();
        assert!(background_keys.contains(&"appearance.navigationBackground"));
        assert!(background_keys.contains(&"background.enabled"));
        assert!(background_keys.contains(&"background.imageOpacity"));
        assert!(background_keys.contains(&"background.positionY"));
        let sidebar = descriptors_for("customization", Some("sidebar"));
        assert!(sidebar
            .iter()
            .flat_map(|(_, v)| v)
            .any(|d| d.key == "sidebar.fileTreeItemSize"));
    }

    #[test]
    fn customization_is_one_continuous_layout_with_anchors_and_a_scrollbar() {
        let groups = customization_groups();
        assert_eq!(groups.len(), CUSTOMIZATION_SECTIONS.len());
        assert!(groups
            .iter()
            .find(|(section, _)| *section == "area-backgrounds")
            .is_some_and(|(_, groups)| !groups.is_empty()));

        let layout = content_layout(MAIN, "customization", Some("appearance"), 0.0);
        let expected_controls: usize = groups
            .iter()
            .flat_map(|(_, section_groups)| section_groups.iter())
            .map(|(_, values)| values.len())
            .sum();
        assert_eq!(layout.controls.len(), expected_controls);
        assert_eq!(layout.section_anchors.len(), CUSTOMIZATION_SECTIONS.len());
        assert!(layout
            .section_anchors
            .windows(2)
            .all(|pair| pair[0].1 < pair[1].1));
        assert!(layout.section_scroll("sidebar").unwrap() > 0.0);
        assert!(!layout.scrollbar_track.is_empty());
        assert!(!layout.scrollbar_thumb.is_empty());
        assert_eq!(layout.background_buttons.len(), 2);
    }

    #[test]
    fn saved_settings_scroll_is_restored_and_clamped_to_new_content() {
        assert_eq!(restore_scroll(Some("120"), 80.0), 80.0);
        assert_eq!(restore_scroll(Some("-4"), 80.0), 0.0);
        assert_eq!(restore_scroll(Some("not-a-number"), 80.0), 0.0);
        assert_eq!(restore_scroll(Some("120"), 0.0), 0.0);
        assert_eq!(restore_scroll(Some("NaN"), 80.0), 0.0);
    }

    #[test]
    fn the_navigation_lists_all_tabs_and_expands_sections_only_under_customization() {
        let lay = nav_layout(NAV, "general", 0.0);
        assert_eq!(
            lay.entries
                .iter()
                .filter(|(_, h)| matches!(h, NavHit::Tab(_)))
                .count(),
            TABS.len()
        );
        assert!(lay
            .entries
            .iter()
            .all(|(_, h)| !matches!(h, NavHit::Section(_))));
        let lay = nav_layout(NAV, "customization", 0.0);
        assert_eq!(
            lay.entries
                .iter()
                .filter(|(_, h)| matches!(h, NavHit::Section(_)))
                .count(),
            CUSTOMIZATION_SECTIONS.len()
        );
        // 分区紧跟在「自定义设置」之后、「AI 配置」之前
        let custom_index = TABS
            .iter()
            .position(|(id, _, _)| *id == "customization")
            .unwrap();
        let ai_index = TABS.iter().position(|(id, _, _)| *id == "ai").unwrap();
        let custom = lay.rect_of(NavHit::Tab(custom_index)).unwrap();
        let first = lay.rect_of(NavHit::Section(0)).unwrap();
        let ai = lay.rect_of(NavHit::Tab(ai_index)).unwrap();
        assert!(first.top > custom.bottom && first.bottom < ai.top);
        assert_eq!(
            lay.hit(300.0, custom.top + 5.0),
            Some(NavHit::Tab(custom_index))
        );
    }

    #[test]
    fn expanded_customization_navigation_scrolls_and_clips_hit_targets() {
        let top = nav_layout(NAV, "customization", 0.0);
        assert_eq!(top.scroll, 0.0);
        assert!(top.max_scroll() > 0.0);
        assert!(top.scrollbar.is_some());

        let last_section = top
            .rect_of(NavHit::Section(CUSTOMIZATION_SECTIONS.len() - 1))
            .unwrap();
        assert!(last_section.bottom > top.scroll_area.bottom);
        assert_eq!(
            top.hit(last_section.left + 4.0, last_section.top + 4.0),
            None,
            "被视口裁掉的分区不能命中"
        );

        let bottom = nav_layout(NAV, "customization", top.max_scroll());
        assert_eq!(bottom.scroll, bottom.max_scroll());
        assert!(bottom.scrollbar.unwrap().thumb.top > top.scrollbar.unwrap().thumb.top);
        let last_section = bottom
            .rect_of(NavHit::Section(CUSTOMIZATION_SECTIONS.len() - 1))
            .unwrap();
        assert!(bottom.scroll_area.contains(
            (last_section.left + last_section.right) / 2.0,
            (last_section.top + last_section.bottom) / 2.0
        ));
        assert_eq!(
            bottom.hit(
                (last_section.left + last_section.right) / 2.0,
                (last_section.top + last_section.bottom) / 2.0
            ),
            Some(NavHit::Section(CUSTOMIZATION_SECTIONS.len() - 1))
        );
        let plugin = bottom.rect_of(NavHit::Tab(TABS.len() - 1)).unwrap();
        assert!(bottom.scroll_area.contains(
            (plugin.left + plugin.right) / 2.0,
            (plugin.top + plugin.bottom) / 2.0
        ));
        assert_eq!(
            bottom.hit(
                bottom.scrollbar.unwrap().thumb.left + 1.0,
                bottom.scrollbar.unwrap().thumb.top + 1.0
            ),
            Some(NavHit::Scrollbar)
        );
    }

    #[test]
    fn every_setting_row_gets_a_control_rect_inside_the_card() {
        let lay = content_layout(MAIN, "general", None, 0.0);
        let n: usize = descriptors_for("general", None)
            .iter()
            .map(|(_, v)| v.len())
            .sum();
        assert_eq!(lay.controls.len(), n);
        for (r, _) in &lay.controls {
            assert!(r.right <= MAIN.right - 24.0 - 16.0 + 0.01);
            assert!(r.width() > 0.0);
        }
        assert!(lay.content_height > 0.0);
    }

    #[test]
    fn a_boolean_paints_a_switch_and_an_enum_paints_a_select() {
        let read = |d: &SettingDescriptor| d.default_value.clone();
        let model = ContentModel {
            tab: "general",
            section: None,
            query: "",
            read: &read,
            editing: None,
        };
        let lay = content_layout(MAIN, "general", None, 0.0);
        let mut list = DrawList::new();
        let p = *theme::tokens().palette(false);
        paint_content(&mut list, MAIN, &lay, &model, 0.0, &p);
        // 开关关着 = border 色药丸 + 白圆点
        assert!(list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::RoundedRect { color, .. } if *color == 0xFFFFFF)));
        // 下拉有向下箭头
        assert!(list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Icon { icon, .. } if *icon == Icon::CHEVRON_DOWN)));
        // 标题在头部
        assert!(list.cmds().iter().any(
            |c| matches!(c, DrawCmd::Text { text, style: TextStyle::Large, .. } if text == "通用")
        ));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn the_shortcuts_tab_lists_every_shortcut_as_a_kbd() {
        let read = |d: &SettingDescriptor| d.default_value.clone();
        let model = ContentModel {
            tab: "shortcuts",
            section: None,
            query: "",
            read: &read,
            editing: None,
        };
        let tall = Rect::new(480.0, 69.0, 1200.0, 3000.0);
        let lay = content_layout(tall, "shortcuts", None, 0.0);
        let mut list = DrawList::new();
        paint_content(
            &mut list,
            tall,
            &lay,
            &model,
            0.0,
            theme::tokens().palette(false),
        );
        let texts: Vec<String> = list
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        for (k, _, _) in GLOBAL_SHORTCUTS {
            assert!(texts.contains(&(*k).to_owned()), "缺 {k}");
        }
        assert!(texts.contains(&"Ctrl+Shift+Z".to_owned()));
    }

    #[test]
    fn scrolling_moves_the_controls_up() {
        let a = content_layout(MAIN, "customization", Some("sidebar"), 0.0);
        let b = content_layout(MAIN, "customization", Some("sidebar"), 40.0);
        assert_eq!(a.controls[0].0.top - b.controls[0].0.top, 40.0);
        assert_eq!(a.content_height, b.content_height);
    }
}
