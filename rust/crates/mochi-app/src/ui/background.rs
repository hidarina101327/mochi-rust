//! 自定义背景：cover / 百分比位置 / 中心缩放，按 index.css 对界面底色做透明处理。
use super::{
    draw::{DrawCmd, DrawList},
    layout::Rect,
    settings_values as values,
    theme::{self, Palette},
};
pub fn cover(area: Rect, size: (u32, u32), scale: f32, x: f32, y: f32) -> Rect {
    let ratio = (area.width() / size.0.max(1) as f32).max(area.height() / size.1.max(1) as f32);
    let w = size.0 as f32 * ratio;
    let h = size.1 as f32 * ratio;
    let left = area.left + (area.width() - w) * x.clamp(0.0, 1.0);
    let top = area.top + (area.height() - h) * y.clamp(0.0, 1.0);
    let cx = (area.left + area.right) / 2.0;
    let cy = (area.top + area.bottom) / 2.0;
    let scale = scale.clamp(0.2, 4.0);
    let left = cx + (left - cx) * scale;
    let top = cy + (top - cy) * scale;
    Rect::new(left, top, left + w * scale, top + h * scale)
}
pub fn apply(list: &mut DrawList, src: &str, size: (u32, u32), area: Rect, p: &Palette) {
    let dest = cover(
        area,
        size,
        values::number("background.scale", 100.0) / 100.0,
        values::number("background.positionX", 50.0) / 100.0,
        values::number("background.positionY", 50.0) / 100.0,
    );
    let opacity = (1.0 - values::number("background.imageOpacity", 30.0) / 100.0).clamp(0.0, 1.0);
    let dark = theme::is_dark(p);
    let alpha = |color: u32| {
        if color == p.surface_elevated {
            Some(opacity * if dark { 0.95 } else { 0.94 })
        } else if color == p.surface {
            Some(opacity * if dark { 0.9 } else { 0.85 })
        } else if color == p.background {
            Some(opacity * if dark { 0.92 } else { 0.9 })
        } else if [
            p.area_navigation_default,
            p.area_sidebar_default,
            p.area_main_default,
            p.area_assistant_default,
        ]
        .contains(&color)
        {
            Some(opacity * 0.9)
        } else {
            None
        }
    };
    let old = std::mem::take(list.commands_mut());
    let mut commands = vec![
        DrawCmd::PushClip { rect: area },
        DrawCmd::Image {
            rect: dest,
            src: src.into(),
            alt: String::new(),
            rotation: 0,
            fill: true,
            thumbnail: false,
        },
        DrawCmd::PopClip,
    ];
    for (i, command) in old.into_iter().enumerate() {
        commands.push(match command {
            DrawCmd::Rect { rect, .. } if i == 0 && rect == area => continue,
            DrawCmd::Rect { rect, color } => {
                if let Some(alpha) = alpha(color) {
                    DrawCmd::RectAlpha { rect, color, alpha }
                } else {
                    DrawCmd::Rect { rect, color }
                }
            }
            DrawCmd::RoundedRect {
                rect,
                radius,
                color,
            } => {
                if let Some(alpha) = alpha(color) {
                    DrawCmd::RoundedRectAlpha {
                        rect,
                        radius,
                        color,
                        alpha,
                    }
                } else {
                    DrawCmd::RoundedRect {
                        rect,
                        radius,
                        color,
                    }
                }
            }
            other => other,
        });
    }
    *list.commands_mut() = commands;
}
/// 返回背景图片操作按钮。`top` 允许设置面板把按钮放在连续内容中的对应
/// 分区标题上，而不是依赖编辑器区域的第一行。
pub fn buttons_at(area: Rect, top: f32) -> [(Rect, &'static str); 2] {
    [
        (
            Rect::new(
                area.right - 244.0,
                top + 14.0,
                area.right - 136.0,
                top + 46.0,
            ),
            "选择背景图",
        ),
        (
            Rect::new(
                area.right - 124.0,
                top + 14.0,
                area.right - 16.0,
                top + 46.0,
            ),
            "移除背景图",
        ),
    ]
}

#[allow(dead_code)]
pub fn buttons(area: Rect) -> [(Rect, &'static str); 2] {
    buttons_at(area, area.top)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cover_and_zoom_preserve_aspect_ratio() {
        let a = Rect::new(0.0, 0.0, 800.0, 600.0);
        let r = cover(a, (1600, 800), 1.0, 0.5, 0.5);
        assert_eq!(r, Rect::new(-200.0, 0.0, 1000.0, 600.0));
        let r = cover(a, (1600, 800), 2.0, 0.5, 0.5);
        assert_eq!(r.width() / r.height(), 2.0);
        assert_eq!((r.left + r.right) / 2.0, 400.0);
    }
}
