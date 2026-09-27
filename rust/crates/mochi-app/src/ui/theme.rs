//! 设计令牌嵌入自 assets/design-tokens.json，字段对应原版 CSS 变量。

use std::collections::HashMap;
use std::sync::OnceLock;

const RAW: &str = include_str!("../../assets/design-tokens.json");

/// 明暗两套。字段名与 index.css 的 CSS 变量一一对应（连字符换成下划线）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    dark: bool,
    pub background: u32,
    pub foreground: u32,
    pub muted: u32,
    pub surface: u32,
    pub surface_elevated: u32,
    pub surface_muted: u32,
    pub border: u32,
    pub area_navigation_default: u32,
    pub area_sidebar_default: u32,
    pub area_main_default: u32,
    pub area_assistant_default: u32,
    pub accent: u32,
    pub accent_hover: u32,
    pub accent_foreground: u32,
    pub danger: u32,
}

impl Palette {
    pub fn button_foreground(&self) -> u32 {
        if is_dark(self) {
            0x16191d
        } else {
            0xffffff
        }
    }
}
pub fn configured_palette(dark: bool) -> Palette {
    let mut p = with_accent(
        *tokens().palette(dark),
        super::settings_values::color("appearance.accentColor", 0x22c55e),
        dark,
    );
    apply_custom_colors(&mut p);
    let opacity = super::settings_values::number("appearance.areaBackgroundOpacity", 100.0) / 100.0;
    let area = |key: &str, fallback: u32| {
        mix(
            resolve_area_color(super::settings_values::color(key, fallback), dark, fallback),
            p.background,
            opacity,
        )
    };
    p.area_navigation_default = area("appearance.navigationBackground", p.area_navigation_default);
    p.area_sidebar_default = area("appearance.sidebarBackground", p.area_sidebar_default);
    p.area_main_default = area("appearance.mainBackground", p.area_main_default);
    p.area_assistant_default = area("appearance.aiPanelBackground", p.area_assistant_default);
    p
}

fn apply_custom_colors(p: &mut Palette) {
    let mode = if p.dark { "dark" } else { "light" };
    let custom = |name: &str, fallback: u32| {
        super::settings_values::color(&format!("appearance.{mode}.{name}"), fallback)
    };
    p.background = custom("background", p.background);
    p.foreground = custom("foreground", p.foreground);
    p.muted = custom("muted", p.muted);
    p.surface = custom("surface", p.surface);
    p.surface_elevated = custom("surfaceElevated", p.surface_elevated);
    p.surface_muted = custom("surfaceMuted", p.surface_muted);
    p.border = custom("border", p.border);
    p.danger = custom("danger", p.danger);
}

/// 工作流页面保留中性色背景，并共用全局操作控件。
pub fn workflow_palette(base: &Palette) -> Palette {
    let mut p = *base;
    let dark = is_dark(base);
    p.background = if dark { 0x18181b } else { 0xf7f7f8 };
    p.surface_muted = if dark { 0x202024 } else { 0xf1f1f3 };
    p.surface = if dark { 0x242428 } else { 0xffffff };
    p.surface_elevated = p.surface;
    p.border = if dark { 0x38383f } else { 0xe4e4e7 };
    apply_custom_colors(&mut p);
    p
}

/// 与 src/stores/accentColor.ts 保持同步。悬停时，按钮会避开文字
/// 颜色，因此自定义的白色或黑色强调色仍能保证按钮文字清晰可读。
pub fn with_accent(mut p: Palette, color: u32, dark: bool) -> Palette {
    p.dark = dark;
    if color == 0x22c55e {
        p.accent = if dark { 0x34d36b } else { 0x22c55e };
        p.accent_hover = if dark { 0x4ade80 } else { 0x16a34a };
        p.accent_foreground = 0x06150b;
        return p;
    }
    p.accent = if dark && luminance(color) < 0.15 {
        mix(color, 0xffffff, 0.75)
    } else {
        color
    };
    let light_text = luminance(p.accent) < 0.179;
    p.accent_foreground = if light_text { 0xffffff } else { 0x000000 };
    p.accent_hover = mix(p.accent, if light_text { 0x000000 } else { 0xffffff }, 0.88);
    p
}

/// 布局尺寸。宽度是 store 默认值（用户可拖动），高度来自 Tailwind 类。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub navigation_width: f32,
    pub navigation_collapsed_width: f32,
    pub sidebar_width: f32,
    pub ai_panel_width: f32,
    pub split_editor_ratio: f32,
    pub title_bar_height: f32,
    pub status_bar_height: f32,
    /// 标签栏高度是内容撑出来的（text-sm 行高 20 + py-2 各 8 + 1px 边框），
    /// 抽取脚本按这条链推导。**是算出来的不是量出来的**，有真机截图时应复核。
    pub tab_bar_height: f32,
    pub tab_bar_compact_height: f32,
    pub right_sidebar_toolbar_height: f32,
    pub split_pane_header_height: f32,
    pub resize_handle_width: f32,
    /// 正文最大行宽。**0 表示不限宽**（上游默认），不是"没设置"。
    pub editor_max_width: f32,
    pub editor_padding_left: f32,
    pub editor_padding_right: f32,
    pub editor_padding_top: f32,
    /// 无边框窗口右侧让给系统窗口控件（最小化/最大化/关闭）的宽度。
    pub title_bar_controls_width: f32,
}

/// 面板宽度和分栏比例的拖动范围。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clamps {
    pub navigation_min: f32,
    pub navigation_max: f32,
    pub ai_panel_min: f32,
    pub ai_panel_max_fraction: f32,
    pub split_ratio_min: f32,
    pub split_ratio_max: f32,
}

impl Clamps {
    pub fn navigation(&self, width: f32) -> f32 {
        width.clamp(self.navigation_min, self.navigation_max)
    }

    /// AI 面板上界是窗口宽度的一个比例，所以要带着窗口宽度算。
    pub fn ai_panel(&self, width: f32, window_width: f32) -> f32 {
        let max = (window_width * self.ai_panel_max_fraction).max(self.ai_panel_min);
        width.clamp(self.ai_panel_min, max)
    }

    /// 将编辑器分栏比例限制在布局令牌定义的范围内。
    #[allow(dead_code)]
    pub fn split_ratio(&self, ratio: f32) -> f32 {
        ratio.clamp(self.split_ratio_min, self.split_ratio_max)
    }
}

pub struct Tokens {
    pub light: Palette,
    pub dark: Palette,
    pub layout: Layout,
    pub clamps: Clamps,
}

impl Tokens {
    pub fn palette(&self, dark: bool) -> &Palette {
        if dark {
            &self.dark
        } else {
            &self.light
        }
    }
}

/// 通用列表行高（大纲面板、滚轮步长）。文件树有自己的 34px（见 `ui::sidebar`）。
pub const ROW_HEIGHT: f32 = 26.0;
pub const BODY_FONT_SIZE: f32 = 13.0;
pub const TITLE_FONT_SIZE: f32 = 13.0;
pub const CAPTION_FONT_SIZE: f32 = 11.5;
/// 源码编辑面的字号档。用等宽——改的是 Markdown 源码，
/// 缩进和围栏对齐靠等宽才看得清。
pub const EDIT_STYLE: crate::ui::draw::TextStyle = crate::ui::draw::TextStyle::Mono;
/// zh-CN 区域标签配这个字族，DirectWrite 才走中文字形而不是日文汉字变体。
pub const UI_FONT: &str = "Microsoft YaHei UI";

/// 解析后的令牌，进程内只解一次。
pub fn tokens() -> &'static Tokens {
    static CELL: OnceLock<Tokens> = OnceLock::new();
    CELL.get_or_init(|| {
        parse(RAW).expect(
            "design-tokens.json 解析失败（这是编译期嵌入的资源，解不开说明抽取脚本产出坏了）",
        )
    })
}

fn parse(raw: &str) -> Option<Tokens> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    Some(Tokens {
        light: palette(&v["colors"]["light"])?,
        dark: palette(&v["colors"]["dark"])?,
        layout: Layout {
            navigation_width: num(&v["layout"]["navigationWidth"])?,
            navigation_collapsed_width: num(&v["layout"]["navigationCollapsedWidth"])?,
            sidebar_width: num(&v["layout"]["sidebarWidth"])?,
            ai_panel_width: num(&v["layout"]["aiPanelWidth"])?,
            split_editor_ratio: num(&v["layout"]["splitEditorRatio"])?,
            title_bar_height: num(&v["layout"]["titleBarHeight"])?,
            status_bar_height: num(&v["layout"]["statusBarHeight"])?,
            tab_bar_height: num(&v["layout"]["tabBarHeight"])?,
            tab_bar_compact_height: num(&v["layout"]["tabBarCompactHeight"])?,
            right_sidebar_toolbar_height: num(&v["layout"]["rightSidebarToolbarHeight"])?,
            split_pane_header_height: num(&v["layout"]["splitPaneHeaderHeight"])?,
            resize_handle_width: num(&v["layout"]["resizeHandleWidth"])?,
            editor_max_width: num(&v["layout"]["editorMaxWidth"])?,
            editor_padding_left: num(&v["layout"]["editorPaddingLeft"])?,
            editor_padding_right: num(&v["layout"]["editorPaddingRight"])?,
            editor_padding_top: num(&v["layout"]["editorPaddingTop"])?,
            title_bar_controls_width: num(&v["layout"]["titleBarControlsWidth"])?,
        },
        clamps: Clamps {
            navigation_min: num(&v["clamps"]["navigationMin"])?,
            navigation_max: num(&v["clamps"]["navigationMax"])?,
            ai_panel_min: num(&v["clamps"]["aiPanelMin"])?,
            ai_panel_max_fraction: num(&v["clamps"]["aiPanelMaxFraction"])?,
            split_ratio_min: num(&v["clamps"]["splitRatioMin"])?,
            split_ratio_max: num(&v["clamps"]["splitRatioMax"])?,
        },
    })
}

fn num(v: &serde_json::Value) -> Option<f32> {
    v.as_f64().map(|n| n as f32)
}

fn palette(v: &serde_json::Value) -> Option<Palette> {
    let c = |key: &str| hex(v[key].as_str()?);
    Some(Palette {
        dark: luminance(c("background")?) < 0.5,
        background: c("background")?,
        foreground: c("foreground")?,
        muted: c("muted")?,
        surface: c("surface")?,
        surface_elevated: c("surface-elevated")?,
        surface_muted: c("surface-muted")?,
        border: c("border")?,
        area_navigation_default: c("area-navigation-default")?,
        area_sidebar_default: c("area-sidebar-default")?,
        area_main_default: c("area-main-default")?,
        area_assistant_default: c("area-assistant-default")?,
        accent: c("accent")?,
        accent_hover: c("accent-hover")?,
        accent_foreground: c("accent-foreground")?,
        danger: c("danger")?,
    })
}

/// `#rrggbb` / `#rgb` → `0xRRGGBB`。
fn hex(s: &str) -> Option<u32> {
    let s = s.trim().strip_prefix('#')?;
    match s.len() {
        // #abc 展开成 #aabbcc，与 CSS 一致
        3 => {
            let mut out = 0u32;
            for ch in s.chars() {
                let d = ch.to_digit(16)?;
                out = (out << 8) | (d << 4) | d;
            }
            Some(out)
        }
        6 => u32::from_str_radix(s, 16).ok(),
        _ => None,
    }
}

#[allow(dead_code)]
/// 未走令牌的 CSS 暗色分支依赖此判断，例如代码块卡片。
pub fn is_dark(p: &Palette) -> bool {
    p.dark
}

pub fn luminance(color: u32) -> f32 {
    let channel = |shift: u32| {
        let c = ((color >> shift) & 0xFF) as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    channel(16) * 0.2126 + channel(8) * 0.7152 + channel(0) * 0.0722
}

/// 线性插值混色。`t` 是 `color` 所占比例，对应 CSS 的
/// `color-mix(in srgb, color t%, fallback)`。
pub fn mix(color: u32, fallback: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |shift: u32| {
        let a = ((color >> shift) & 0xFF) as f32;
        let b = ((fallback >> shift) & 0xFF) as f32;
        (a * t + b * (1.0 - t)).round().clamp(0.0, 255.0) as u32
    };
    (lerp(16) << 16) | (lerp(8) << 8) | lerp(0)
}

/// 自定义底色与主题反差过大时，混入备用颜色；阈值与 areaColors.ts::resolveAreaColor 保持一致。
#[allow(dead_code)]
pub fn resolve_area_color(color: u32, dark_theme: bool, fallback: u32) -> u32 {
    let l = luminance(color);
    if dark_theme && l > 0.42 {
        mix(color, fallback, 0.12)
    } else if !dark_theme && l < 0.14 {
        mix(color, fallback, 0.10)
    } else {
        color
    }
}

/// 供调试/校验用：把原始 JSON 里的颜色表整份取出来。
#[allow(dead_code)]
pub fn raw_colors(dark: bool) -> HashMap<String, String> {
    let v: serde_json::Value = serde_json::from_str(RAW).unwrap_or(serde_json::Value::Null);
    let key = if dark { "dark" } else { "light" };
    v["colors"][key]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_tokens_parse() {
        let t = tokens();
        assert_eq!(t.light.background, 0xF6F7F8);
        assert_eq!(t.dark.background, 0x121516);
    }

    #[test]
    fn custom_accents_keep_button_labels_readable_in_both_modes() {
        assert_eq!(tokens().light.accent, 0x22c55e);
        for dark in [false, true] {
            for color in [
                0x22c55e, 0x3b82f6, 0xa855f7, 0xffffff, 0x000000, 0xf97316, 0x64748b,
            ] {
                let p = with_accent(*tokens().palette(dark), color, dark);
                for background in [p.accent, p.accent_hover] {
                    let a = luminance(background) + 0.05;
                    let b = luminance(p.accent_foreground) + 0.05;
                    assert!(a.max(b) / a.min(b) >= 4.5);
                }
                let workflow = workflow_palette(&p);
                assert_eq!(workflow.accent, p.accent);
                assert_eq!(workflow.accent_foreground, p.accent_foreground);
            }
        }
    }

    #[test]
    fn the_layout_defaults_match_the_workspace_store() {
        let l = tokens().layout;
        assert_eq!(l.navigation_width, 220.0);
        assert_eq!(l.sidebar_width, 260.0);
        assert_eq!(l.ai_panel_width, 420.0);
        assert_eq!(l.navigation_collapsed_width, 60.0);
        assert_eq!(l.title_bar_height, 32.0);
        assert_eq!(l.right_sidebar_toolbar_height, 40.0);
        assert_eq!(l.status_bar_height, 24.0);
    }

    #[test]
    fn the_editor_is_unbounded_and_left_aligned_by_default() {
        // editorMaxWidth = 0 表示不限宽，不是未设置。
        let l = tokens().layout;
        assert_eq!(l.editor_max_width, 0.0);
        assert_eq!(l.editor_padding_left, 24.0);
        assert_eq!(l.editor_padding_right, 24.0);
        assert_eq!(l.editor_padding_top, 32.0);
    }

    #[test]
    fn the_title_bar_leaves_room_for_the_system_window_buttons() {
        // 三个 46px 的窗口按钮
        assert_eq!(tokens().layout.title_bar_controls_width, 138.0);
    }

    #[test]
    fn the_compact_tab_bar_is_shorter_than_the_normal_one() {
        // py-1.5 比 py-2 每边少 2px，共 4px
        let l = tokens().layout;
        assert_eq!(l.tab_bar_height - l.tab_bar_compact_height, 4.0);
        assert_eq!(l.tab_bar_height, 37.0);
    }

    #[test]
    fn every_light_color_has_a_dark_counterpart() {
        // 少一个就意味着某个面板在暗色下会回落到亮色值。
        // 抽取脚本已经查过一遍，这里在 Rust 侧再钉一次——
        // 有人手改 JSON 时脚本不会跑，测试会。
        let light = raw_colors(false);
        let dark = raw_colors(true);
        assert!(!light.is_empty());
        assert_eq!(light.len(), dark.len());
        for key in light.keys() {
            assert!(dark.contains_key(key), "暗色主题缺 {key}");
        }
    }

    #[test]
    fn short_hex_expands_the_way_css_does() {
        assert_eq!(hex("#abc"), Some(0xAABBCC));
        assert_eq!(hex("#AABBCC"), Some(0xAABBCC));
        assert_eq!(hex("abc"), None);
        assert_eq!(hex("#12345"), None);
    }

    #[test]
    fn luminance_matches_the_typescript_reference_values() {
        // 期望值取自 areaColors.test.ts 的口径：纯白 1、纯黑 0
        assert!((luminance(0xFFFFFF) - 1.0).abs() < 1e-4);
        assert!(luminance(0x000000).abs() < 1e-6);
        // 绿色通道权重最高，纯绿应当明显亮于纯蓝
        assert!(luminance(0x00FF00) > luminance(0x0000FF));
    }

    #[test]
    fn a_light_custom_color_is_toned_down_under_the_dark_theme() {
        // 暗色主题下用户选了近白色：直接用会把面板刷成白的
        let toned = resolve_area_color(0xFFFFFF, true, 0x121516);
        assert_ne!(toned, 0xFFFFFF);
        assert!(luminance(toned) < 0.25);
    }

    #[test]
    fn a_mid_tone_color_passes_through_untouched() {
        // 亮度落在 0.14~0.42 之间的颜色两边都不动它
        let mid = 0x808080;
        assert!(luminance(mid) > 0.14 && luminance(mid) < 0.42);
        assert_eq!(resolve_area_color(mid, true, 0x121516), mid);
        assert_eq!(resolve_area_color(mid, false, 0xF6F7F8), mid);
    }

    #[test]
    fn mix_at_the_extremes_returns_the_endpoints() {
        assert_eq!(mix(0xFF0000, 0x0000FF, 1.0), 0xFF0000);
        assert_eq!(mix(0xFF0000, 0x0000FF, 0.0), 0x0000FF);
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.5), 0x808080);
    }

    #[test]
    fn the_ai_panel_clamp_follows_the_window_width() {
        let c = tokens().clamps;
        // 宽窗口：上界是 45% 窗宽
        assert_eq!(c.ai_panel(2000.0, 2000.0), 900.0);
        // 窄窗口：45% 低于下界 340 时，下界赢——否则面板会被压到看不见
        assert_eq!(c.ai_panel(100.0, 600.0), 340.0);
    }

    #[test]
    fn the_navigation_clamp_matches_the_electron_range() {
        let c = tokens().clamps;
        assert_eq!(c.navigation(10.0), 48.0);
        assert_eq!(c.navigation(9999.0), 300.0);
        assert_eq!(c.navigation(220.0), 220.0);
    }

    #[test]
    fn the_split_ratio_never_collapses_a_pane_entirely() {
        let c = tokens().clamps;
        assert_eq!(c.split_ratio(0.0), 0.2);
        assert_eq!(c.split_ratio(1.0), 0.8);
    }
}
