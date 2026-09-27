//! 高级选项使用预留高度，展开时不移动表单。

use std::path::PathBuf;

use crate::ui::draw::{Align, DrawList, TextStyle};
use crate::ui::icons::Icon;
use crate::ui::layout::Rect;
use crate::ui::theme::Palette;
use crate::ui::widgets::{FieldLook, TextField};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Include,
    Exclude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Close,
    Outside,
    Source,
    Name,
    Advanced,
    Help,
    Include,
    Exclude,
    Cancel,
    Create,
    RulesPopover,
    Inside,
}

fn large_title_extra() -> f32 {
    (TextStyle::Large.line_height() - 28.0).max(0.0)
}

#[derive(Debug, Clone)]
pub struct State {
    pub library: PathBuf,
    pub source: Option<PathBuf>,
    pub name: TextField,
    pub include: TextField,
    pub exclude: TextField,
    pub advanced: bool,
    pub active: Field,
    pub error: String,
    pub hover: Option<Hit>,
    /// 点击问号时记录指针位置，让规则说明成为就近的浮窗而不是全局 Toast。
    pub rule_help_anchor: Option<(f32, f32)>,
}

impl State {
    pub fn new(library: PathBuf, include: &[String], exclude: &[String]) -> Self {
        Self {
            library,
            source: None,
            name: TextField::new("例如：工作资料"),
            include: TextField::new("**（多个规则用 ; 分隔）").with_text(&include.join("; ")),
            exclude: TextField::new("例如：node_modules/**; .git/**")
                .with_text(&exclude.join("; ")),
            advanced: false,
            active: Field::Name,
            error: String::new(),
            hover: None,
            rule_help_anchor: None,
        }
    }

    /// 固定高度；高级区仅切换内容可见性。
    pub fn rect(&self, viewport: Rect) -> Rect {
        const WIDTH: f32 = 520.0;
        let height = 480.0 + large_title_extra();
        Rect::from_size(
            ((viewport.left + viewport.right - WIDTH) / 2.0).round(),
            ((viewport.top + viewport.bottom - height) / 2.0).round(),
            WIDTH,
            height,
        )
    }

    fn parts(&self, viewport: Rect) -> (Rect, Rect, Rect, Rect, Rect, Rect, Rect, Rect, Rect) {
        let r = self.rect(viewport);
        let title_extra = large_title_extra();
        let source = Rect::new(
            r.left + 24.0,
            r.top + 94.0 + title_extra,
            r.right - 24.0,
            r.top + 132.0 + title_extra,
        );
        let name = Rect::new(
            r.left + 24.0,
            r.top + 174.0 + title_extra,
            r.right - 24.0,
            r.top + 212.0 + title_extra,
        );
        let advanced = Rect::new(
            r.left + 24.0,
            r.top + 238.0 + title_extra,
            r.left + 112.0,
            r.top + 262.0 + title_extra,
        );
        let help = Rect::from_size(advanced.right + 2.0, advanced.top + 2.0, 20.0, 20.0);
        let include = Rect::new(
            r.left + 24.0,
            r.top + 292.0 + title_extra,
            r.right - 24.0,
            r.top + 330.0 + title_extra,
        );
        let exclude = Rect::new(
            r.left + 24.0,
            r.top + 352.0 + title_extra,
            r.right - 24.0,
            r.top + 390.0 + title_extra,
        );
        let bottom = r.bottom - 20.0;
        let create = Rect::new(r.right - 24.0 - 76.0, bottom - 34.0, r.right - 24.0, bottom);
        let cancel = Rect::new(create.left - 84.0, bottom - 34.0, create.left - 8.0, bottom);
        (
            source, name, advanced, help, include, exclude, cancel, create, r,
        )
    }

    pub fn hit(&self, viewport: Rect, x: f32, y: f32) -> Hit {
        let (source, name, advanced, help, include, exclude, cancel, create, r) =
            self.parts(viewport);
        if self
            .rule_help_rect(viewport)
            .is_some_and(|popover| popover.contains(x, y))
        {
            return Hit::RulesPopover;
        }
        if !r.contains(x, y) {
            return Hit::Outside;
        }
        let close = Rect::from_size(r.right - 40.0, r.top + 16.0, 24.0, 24.0);
        if close.contains(x, y) {
            return Hit::Close;
        }
        if source.contains(x, y) {
            return Hit::Source;
        }
        if name.contains(x, y) {
            return Hit::Name;
        }
        if advanced.contains(x, y) {
            return Hit::Advanced;
        }
        if help.contains(x, y) {
            return Hit::Help;
        }
        if self.advanced && include.contains(x, y) {
            return Hit::Include;
        }
        if self.advanced && exclude.contains(x, y) {
            return Hit::Exclude;
        }
        if cancel.contains(x, y) {
            return Hit::Cancel;
        }
        if create.contains(x, y) {
            return Hit::Create;
        }
        Hit::Inside
    }

    pub fn toggle_rule_help(&mut self, x: f32, y: f32) {
        self.rule_help_anchor = if self.rule_help_anchor.is_some() {
            None
        } else {
            Some((x, y))
        };
    }

    pub fn close_rule_help(&mut self) {
        self.rule_help_anchor = None;
    }

    fn rule_help_rect(&self, viewport: Rect) -> Option<Rect> {
        let (x, y) = self.rule_help_anchor?;
        const WIDTH: f32 = 308.0;
        const HEIGHT: f32 = 102.0;
        // 优先在鼠标右下方展开；贴边时翻到左侧/上方，始终留在窗口内。
        let left = if x + 12.0 + WIDTH <= viewport.right - 8.0 {
            x + 12.0
        } else {
            x - 12.0 - WIDTH
        }
        .clamp(viewport.left + 8.0, viewport.right - WIDTH - 8.0);
        let top = if y + 12.0 + HEIGHT <= viewport.bottom - 8.0 {
            y + 12.0
        } else {
            y - 12.0 - HEIGHT
        }
        .clamp(viewport.top + 8.0, viewport.bottom - HEIGHT - 8.0);
        Some(Rect::from_size(left, top, WIDTH, HEIGHT))
    }

    pub fn set_hover(&mut self, viewport: Rect, x: f32, y: f32) -> bool {
        let hit = self.hit(viewport, x, y);
        let hover = matches!(
            hit,
            Hit::Source | Hit::Advanced | Hit::Help | Hit::Cancel | Hit::Create
        )
        .then_some(hit);
        let changed = self.hover != hover;
        self.hover = hover;
        changed
    }

    pub fn field_text_left(&self, viewport: Rect, field: Field) -> f32 {
        let (_, name, _, _, include, exclude, _, _, _) = self.parts(viewport);
        match field {
            Field::Name => name.left + 12.0,
            Field::Include => include.left + 12.0,
            Field::Exclude => exclude.left + 12.0,
        }
    }

    pub fn set_source(&mut self, source: PathBuf) {
        if self.name.is_empty() {
            let label = source
                .file_name()
                .and_then(|part| part.to_str())
                .unwrap_or("映射文件夹");
            self.name.set_text(label);
        }
        self.source = Some(source);
        self.error.clear();
    }

    pub fn patterns(field: &TextField) -> Vec<String> {
        field
            .text()
            .split(';')
            .map(str::trim)
            .filter(|rule| !rule.is_empty())
            .map(str::to_owned)
            .collect()
    }

    pub fn paint(&mut self, list: &mut DrawList, viewport: Rect, p: &Palette) {
        list.rect_alpha(viewport, 0x000000, 0.5);
        let (source, name, advanced, help, include, exclude, cancel, create, r) =
            self.parts(viewport);
        let close = Rect::from_size(r.right - 40.0, r.top + 16.0, 24.0, 24.0);
        list.rounded_rect(r, 8.0, p.surface);
        list.rounded_border(r, 8.0, p.border);
        list.text(
            Rect::new(
                r.left + 24.0,
                r.top + 22.0,
                r.right - 56.0,
                r.top + 50.0 + large_title_extra(),
            ),
            "新增映射文件夹",
            TextStyle::Large,
            p.foreground,
        );
        list.icon_centered(close, Icon::X, 16.0, p.muted);

        list.text(
            Rect::new(
                source.left,
                source.top - 23.0,
                source.right,
                source.top - 3.0,
            ),
            "计算机上的文件夹",
            TextStyle::Label,
            p.muted,
        );
        list.rounded_rect(
            source,
            6.0,
            if self.hover == Some(Hit::Source) {
                p.background
            } else {
                p.surface
            },
        );
        list.rounded_border(source, 6.0, p.border);
        let source_text = self
            .source
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| "选择文件夹…".into());
        list.text(
            Rect::new(
                source.left + 12.0,
                source.top + 10.0,
                source.right - 30.0,
                source.bottom - 8.0,
            ),
            source_text,
            TextStyle::Label,
            if self.source.is_some() {
                p.foreground
            } else {
                p.muted
            },
        );
        list.icon_centered(
            Rect::from_size(source.right - 30.0, source.top + 7.0, 24.0, 24.0),
            Icon::FOLDER_PLUS,
            15.0,
            p.muted,
        );

        list.text(
            Rect::new(name.left, name.top - 23.0, name.right, name.top - 3.0),
            "显示名称",
            TextStyle::Label,
            p.muted,
        );
        self.name.style = TextStyle::Label;
        self.name.paint(
            list,
            name,
            self.active == Field::Name,
            p,
            FieldLook::dialog(p),
        );

        if self.hover == Some(Hit::Advanced) {
            list.rounded_rect(advanced, 4.0, p.background);
        }
        list.text(
            advanced,
            if self.advanced {
                "⌄  高级设置"
            } else {
                "›  高级设置"
            },
            TextStyle::Caption,
            p.muted,
        );
        list.rounded_border(
            help,
            10.0,
            if self.rule_help_anchor.is_some() {
                p.accent
            } else {
                p.border
            },
        );
        list.text_aligned(help, "?", TextStyle::Caption, p.muted, Align::Center);

        if self.advanced {
            list.text(
                Rect::new(
                    include.left,
                    include.top - 23.0,
                    include.right,
                    include.top - 3.0,
                ),
                "白名单（允许读取）",
                TextStyle::Label,
                p.muted,
            );
            list.text(
                Rect::new(
                    exclude.left,
                    exclude.top - 23.0,
                    exclude.right,
                    exclude.top - 3.0,
                ),
                "黑名单（排除读取）",
                TextStyle::Label,
                p.muted,
            );
            self.include.style = TextStyle::Label;
            self.exclude.style = TextStyle::Label;
            self.include.paint(
                list,
                include,
                self.active == Field::Include,
                p,
                FieldLook::dialog(p),
            );
            self.exclude.paint(
                list,
                exclude,
                self.active == Field::Exclude,
                p,
                FieldLook::dialog(p),
            );
        } else {
            list.text(
                Rect::new(
                    advanced.left,
                    advanced.bottom + 8.0,
                    r.right - 24.0,
                    advanced.bottom + 28.0,
                ),
                "默认规则已填入高级设置；展开后可以修改。",
                TextStyle::Caption,
                p.muted,
            );
        }
        if !self.error.is_empty() {
            list.text(
                Rect::new(
                    r.left + 24.0,
                    r.bottom - 76.0,
                    r.right - 24.0,
                    r.bottom - 56.0,
                ),
                self.error.clone(),
                TextStyle::Caption,
                p.danger,
            );
        }
        if self.hover == Some(Hit::Cancel) {
            list.rounded_rect(cancel, 8.0, p.background);
        }
        list.text_aligned(
            cancel,
            "取消",
            TextStyle::Label,
            p.foreground,
            Align::Center,
        );
        list.glass_button(create, 8.0, p, self.hover == Some(Hit::Create));
        list.text_aligned(
            create,
            "创建",
            TextStyle::Label,
            p.button_foreground(),
            Align::Center,
        );
        self.paint_rule_help(list, viewport, p);
    }

    fn paint_rule_help(&self, list: &mut DrawList, viewport: Rect, p: &Palette) {
        let Some(popover) = self.rule_help_rect(viewport) else {
            return;
        };
        let shadow = Rect::new(
            popover.left + 2.0,
            popover.top + 3.0,
            popover.right + 2.0,
            popover.bottom + 3.0,
        );
        list.rounded_rect_alpha(shadow, 8.0, 0x000000, 0.18);
        list.rounded_rect(popover, 8.0, p.surface_elevated);
        list.rounded_border(popover, 8.0, p.border);
        list.text(
            Rect::new(
                popover.left + 12.0,
                popover.top + 10.0,
                popover.right - 12.0,
                popover.top + 29.0,
            ),
            "通配符规则（类似 .gitignore）",
            TextStyle::Caption,
            p.foreground,
        );
        list.text(
            Rect::new(
                popover.left + 12.0,
                popover.top + 33.0,
                popover.right - 12.0,
                popover.top + 51.0,
            ),
            "* 不跨目录；** 可跨目录；? 匹配一个字符",
            TextStyle::Tiny,
            p.muted,
        );
        list.text(
            Rect::new(
                popover.left + 12.0,
                popover.top + 55.0,
                popover.right - 12.0,
                popover.top + 73.0,
            ),
            "用 ; 分隔多条规则，! 前缀可重新包含",
            TextStyle::Tiny,
            p.muted,
        );
        list.text(
            Rect::new(
                popover.left + 12.0,
                popover.top + 77.0,
                popover.right - 12.0,
                popover.bottom - 8.0,
            ),
            "点击问号或浮窗外即可收起",
            TextStyle::Tiny,
            p.muted,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_help_is_a_clickable_popover_next_to_its_anchor() {
        let viewport = Rect::from_size(0.0, 0.0, 800.0, 600.0);
        let mut state = State::new(PathBuf::new(), &[], &[]);
        state.toggle_rule_help(760.0, 560.0);
        let popover = state.rule_help_rect(viewport).unwrap();

        assert!(popover.left >= viewport.left);
        assert!(popover.top >= viewport.top);
        assert!(popover.right <= viewport.right);
        assert!(popover.bottom <= viewport.bottom);
        assert_eq!(
            state.hit(viewport, popover.left + 4.0, popover.top + 4.0),
            Hit::RulesPopover
        );
    }
}
