//! 文件树/大纲的引导线，与命中布局独立。
use super::{draw::DrawList, layout::Rect, settings_values};
pub struct Style {
    pub enabled: bool,
    pub width: f32,
    pub gap: f32,
    pub color: u32,
    pub dashed: bool,
}
impl Style {
    pub fn read(tree: bool, fallback: u32) -> Self {
        let prefix = if tree {
            "sidebar.treeGuide"
        } else {
            "outline.guide"
        };
        Self {
            enabled: settings_values::boolean(
                if tree {
                    "sidebar.showTreeGuides"
                } else {
                    "outline.showGuides"
                },
                true,
            ),
            width: settings_values::number(&format!("{prefix}Width"), 1.0),
            gap: settings_values::number(&format!("{prefix}HorizontalGap"), 4.0),
            color: settings_values::color(&format!("{prefix}Color"), fallback),
            dashed: settings_values::text(&format!("{prefix}Style"), "solid") == "dashed",
        }
    }
    pub fn line(&self, list: &mut DrawList, x: f32, y: f32, end: f32, vertical: bool) {
        if !self.enabled || self.width <= 0.0 {
            return;
        }
        let start = if vertical { y } else { x };
        let mut pos = start;
        while pos < end {
            let stop = if self.dashed {
                (pos + self.width * 3.0).min(end)
            } else {
                end
            };
            list.rect(
                if vertical {
                    Rect::new(x, pos, x + self.width, stop)
                } else {
                    Rect::new(pos, y, stop, y + self.width)
                },
                self.color,
            );
            if !self.dashed {
                break;
            }
            pos = stop + self.width * 3.0;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dashed_lines_are_bounded_and_disabled_lines_are_empty() {
        let mut list = DrawList::new();
        let mut style = Style {
            enabled: true,
            width: 2.0,
            gap: 4.0,
            color: 1,
            dashed: true,
        };
        style.line(&mut list, 10.0, 0.0, 30.0, true);
        assert_eq!(list.cmds().len(), 3);
        style.enabled = false;
        style.line(&mut list, 0.0, 0.0, 300.0, false);
        assert_eq!(list.cmds().len(), 3);
        assert!(list.finish().is_ok());
    }
}
