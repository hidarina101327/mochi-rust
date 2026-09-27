//! 单 UI 线程的正文排版设置；界面标签字号不跟随正文字号变化。
use mochi_core::app_settings::{self, AppSettings, SettingValue};
use std::cell::RefCell;
#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub slash_menu_enabled: bool,
    pub live_line_source: bool,
    pub padding_left: f32,
    pub padding_right: f32,
    pub padding_top: f32,
    pub padding_bottom: f32,
    pub max_width: f32,
    pub alignment: super::draw::Align,
    pub font_size: f32,
    pub line_height: f32,
    pub code_size: f32,
    pub code_height: f32,
    pub table_size: f32,
    pub heading_size: [f32; 6],
    pub heading_weight: [i32; 6],
    pub heading_top: [f32; 6],
    pub heading_bottom: [f32; 6],
    pub heading_left: [f32; 6],
    pub heading_underline: [bool; 6],
    /// 下划线粗细与它离文字底部的距离。
    pub heading_underline_height: [f32; 6],
    pub heading_underline_offset: [f32; 6],
    pub paragraph_spacing: f32,
    pub block_spacing: f32,
    pub table_padding: f32,
    pub table_border: f32,
    pub table_margin: f32,
    pub list_indent: f32,
    pub list_spacing: f32,
    pub code_padding: f32,
    pub code_radius: f32,
    pub code_margin: f32,
    pub code_style: u8,
    /// 关闭时省去标题工具栏，只在代码卡片右上角保留语言标识。
    pub code_show_title: bool,
    /// 长代码行默认在卡片内软换行；关闭后可用 Shift+滚轮横向浏览。
    pub code_wrap: bool,
    /// 代码块左侧的行号栏。
    pub code_show_line_numbers: bool,
    pub quote_padding: f32,
    pub quote_border: f32,
    pub document_text_color: Option<u32>,
    pub heading_text_color: Option<u32>,
    pub quote_text_color: Option<u32>,
    pub quote_border_color: Option<u32>,
    pub list_marker_color: Option<u32>,
    pub code_background_color: Option<u32>,
    pub code_foreground_color: Option<u32>,
    pub code_border_color: Option<u32>,
    pub table_text_color: Option<u32>,
    pub table_header_background_color: Option<u32>,
    pub table_border_color: Option<u32>,
}
impl Default for Preferences {
    fn default() -> Self {
        let l = super::theme::tokens().layout;
        Self {
            slash_menu_enabled: false,
            live_line_source: false,
            padding_left: l.editor_padding_left,
            padding_right: l.editor_padding_right,
            padding_top: l.editor_padding_top,
            padding_bottom: l.editor_padding_top,
            max_width: l.editor_max_width,
            alignment: super::draw::Align::Center,
            font_size: super::theme::BODY_FONT_SIZE,
            line_height: 1.75,
            code_size: 13.0,
            code_height: 1.7,
            table_size: 14.0,
            heading_size: [26.0, 21.0, 17.0, 17.0, 17.0, 17.0],
            heading_weight: [700; 6],
            heading_top: [16.0; 6],
            heading_bottom: [4.0; 6],
            heading_left: [0.0; 6],
            heading_underline: [false; 6],
            heading_underline_height: [1.0; 6],
            heading_underline_offset: [8.0; 6],
            paragraph_spacing: 0.0,
            block_spacing: 10.0,
            table_padding: 10.0,
            table_border: 1.0,
            table_margin: 20.0,
            list_indent: 22.0,
            list_spacing: 0.0,
            code_padding: 20.0,
            code_radius: 10.0,
            code_margin: 20.0,
            code_style: 0,
            code_show_title: true,
            code_wrap: true,
            code_show_line_numbers: true,
            quote_padding: 16.0,
            quote_border: 3.0,
            document_text_color: None,
            heading_text_color: None,
            quote_text_color: None,
            quote_border_color: None,
            list_marker_color: None,
            code_background_color: None,
            code_foreground_color: None,
            code_border_color: None,
            table_text_color: None,
            table_header_background_color: None,
            table_border_color: None,
        }
    }
}
thread_local! { static CURRENT:RefCell<Preferences>=RefCell::new(Preferences::default()); }
pub fn current() -> Preferences {
    CURRENT.with(|p| p.borrow().clone())
}
pub fn set(value: Preferences) {
    CURRENT.with(|p| *p.borrow_mut() = value);
}
impl Preferences {
    /// 整体移动编辑区域，让绘制位置与指针坐标保持一致。
    pub fn aligned_area(&self, mut area: super::layout::Rect) -> super::layout::Rect {
        if self.max_width <= 0.0 {
            return area;
        }
        let width = (self.max_width + self.padding_left + self.padding_right).min(area.width());
        let free = (area.width() - width).max(0.0);
        area.left += match self.alignment {
            super::draw::Align::Leading => 0.0,
            super::draw::Align::Center => free / 2.0,
            super::draw::Align::Trailing => free,
        };
        area.right = area.left + width;
        area
    }

    pub fn read(settings: &AppSettings) -> Self {
        let mut p = Self {
            slash_menu_enabled: app_settings::descriptor("editor.slashMenuEnabled")
                .is_some_and(|d| matches!(settings.read(d), SettingValue::Bool(true))),
            live_line_source: app_settings::descriptor("editorLayout.liveLineSource")
                .is_some_and(|d| matches!(settings.read(d), SettingValue::Bool(true))),
            ..Default::default()
        };
        let number = |key: &str, fallback: f32| {
            app_settings::descriptor(key)
                .and_then(|d| match settings.read(d) {
                    SettingValue::Number(n) => Some(n as f32),
                    _ => None,
                })
                .unwrap_or(fallback)
        };
        p.padding_left = number("editorLayout.paddingLeft", p.padding_left);
        p.padding_right = number("editorLayout.paddingRight", p.padding_right);
        p.padding_top = number("editorLayout.paddingTop", p.padding_top).max(0.0);
        p.padding_bottom = number("editorLayout.paddingBottom", p.padding_bottom).max(0.0);
        p.max_width = number("editorLayout.maxWidth", p.max_width);
        p.alignment = match app_settings::descriptor("editorLayout.alignment")
            .map(|d| settings.read(d).to_storage())
            .as_deref()
        {
            Some("left") => super::draw::Align::Leading,
            Some("right") => super::draw::Align::Trailing,
            _ => super::draw::Align::Center,
        };
        p.font_size = number("typography.fontSize", p.font_size);
        p.line_height = number("typography.lineHeight", p.line_height);
        p.code_size = number("code.fontSize", p.code_size);
        p.code_height = number("code.lineHeight", p.code_height);
        p.table_size = number("tables.fontSize", p.table_size);
        p.paragraph_spacing = number("typography.paragraphSpacing", p.paragraph_spacing);
        p.block_spacing = number("typography.blockSpacing", p.block_spacing);
        p.table_padding = number("tables.cellPadding", p.table_padding);
        p.table_border = number("tables.borderWidth", p.table_border);
        p.table_margin = number("tables.margin", p.table_margin).max(0.0);
        p.list_indent = number("lists.indent", p.list_indent);
        p.list_spacing = number("lists.itemSpacing", p.list_spacing);
        p.code_padding = number("code.padding", p.code_padding);
        p.code_radius = number("code.borderRadius", p.code_radius);
        p.code_margin = number("code.margin", p.code_margin).max(0.0);
        p.code_show_title = app_settings::descriptor("code.showTitle")
            .is_none_or(|d| matches!(settings.read(d), SettingValue::Bool(true)));
        p.code_wrap = app_settings::descriptor("code.overflow")
            .is_none_or(|d| !matches!(settings.read(d).to_storage().as_str(), "scroll"));
        p.code_show_line_numbers = app_settings::descriptor("code.showLineNumbers")
            .is_none_or(|d| matches!(settings.read(d), SettingValue::Bool(true)));
        p.quote_padding = number("lists.blockquotePadding", p.quote_padding);
        p.quote_border = number("lists.blockquoteBorderWidth", p.quote_border);
        let color = |key: &str| {
            app_settings::descriptor(key).and_then(|d| match settings.read(d) {
                SettingValue::Text(value) => super::styles::color(&value),
                _ => None,
            })
        };
        p.document_text_color = color("typography.textColor");
        p.heading_text_color = color("headings.textColor");
        p.quote_text_color = color("lists.blockquoteTextColor");
        p.quote_border_color = color("lists.blockquoteBorderColor");
        p.list_marker_color = color("lists.markerColor");
        p.code_background_color = color("code.backgroundColor");
        p.code_foreground_color = color("code.foregroundColor");
        p.code_border_color = color("code.borderColor");
        p.table_text_color = color("tables.textColor");
        p.table_header_background_color = color("tables.headerBackgroundColor");
        p.table_border_color = color("tables.borderColor");
        p.code_style = app_settings::descriptor("code.style")
            .map(|d| match settings.read(d).to_storage().as_str() {
                "dark" => 1,
                "paper" => 2,
                "terminal" => 3,
                _ => 0,
            })
            .unwrap_or(0);
        for i in 0..6 {
            let h = i + 1;
            p.heading_size[i] = number(&format!("headings.h{h}.fontSize"), p.heading_size[i]);
            p.heading_weight[i] = number(&format!("headings.h{h}.fontWeight"), 700.0) as i32;
            p.heading_top[i] = number(&format!("headings.h{h}.marginTop"), p.heading_top[i]);
            p.heading_bottom[i] =
                number(&format!("headings.h{h}.marginBottom"), p.heading_bottom[i]);
            p.heading_left[i] = number(&format!("headings.h{h}.marginLeft"), 0.0);
            p.heading_underline[i] = app_settings::descriptor(&format!("headings.h{h}.underline"))
                .is_some_and(|d| matches!(settings.read(d), SettingValue::Bool(true)));
            p.heading_underline_height[i] =
                number(&format!("headings.h{h}.underlineHeight"), 1.0).max(1.0);
            p.heading_underline_offset[i] =
                number(&format!("headings.h{h}.underlineOffset"), 8.0).max(0.0);
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{
        document,
        draw::{DrawList, TextStyle},
        layout::Rect,
        theme,
    };
    #[test]
    fn registered_layout_and_color_settings_are_read_into_preferences() {
        use std::sync::Arc;

        static SEQUENCE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "mochi-editor-preferences-{}-{}.json",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let settings = AppSettings::new(Arc::new(mochi_core::settings::SettingsService::new(
            Some(path.clone()),
        )));
        let write = |key: &str, value| {
            let descriptor = app_settings::descriptor(key).expect(key);
            settings.write(descriptor, &value);
        };
        write("code.showTitle", SettingValue::Bool(false));
        write("editorLayout.paddingTop", SettingValue::Number(11.0));
        write("editorLayout.paddingBottom", SettingValue::Number(23.0));
        write("code.margin", SettingValue::Number(31.0));
        write("tables.margin", SettingValue::Number(9.0));
        write("typography.textColor", SettingValue::Text("#abcdef".into()));

        let preferences = Preferences::read(&settings);
        assert!(!preferences.code_show_title);
        assert_eq!(preferences.padding_top, 11.0);
        assert_eq!(preferences.padding_bottom, 23.0);
        assert_eq!(preferences.code_margin, 31.0);
        assert_eq!(preferences.table_margin, 9.0);
        assert_eq!(preferences.document_text_color, Some(0xabcdef));
        drop(settings);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn document_alignment_keeps_vertical_geometry_and_fits_narrow_panes() {
        let area = Rect::new(100.0, 20.0, 900.0, 620.0);
        let mut prefs = Preferences::default();
        prefs.max_width = 400.0;
        prefs.padding_left = 30.0;
        prefs.padding_right = 10.0;
        for (alignment, left) in [
            (super::super::draw::Align::Leading, 100.0),
            (super::super::draw::Align::Center, 280.0),
            (super::super::draw::Align::Trailing, 460.0),
        ] {
            prefs.alignment = alignment;
            let aligned = prefs.aligned_area(area);
            assert_eq!(aligned.left, left);
            assert_eq!(aligned.width(), 440.0);
            assert_eq!(aligned.top, area.top);
            assert_eq!(aligned.bottom, area.bottom);
            let narrow = Rect::new(100.0, 20.0, 300.0, 620.0);
            assert_eq!(prefs.aligned_area(narrow), narrow);
        }
        prefs.max_width = 0.0;
        assert_eq!(prefs.aligned_area(area), area);
    }
    struct Restore(Preferences);
    impl Drop for Restore {
        fn drop(&mut self) {
            set(self.0.clone());
        }
    }
    #[test]
    fn heading_underlines_are_opt_in_for_each_level() {
        let _restore = Restore(current());
        let raw = "# H1\n\n## H2\n\n### H3\n\n#### H4\n\n##### H5\n\n###### H6";
        let blocks = document::parse_ranged(raw);
        set(Preferences::default());
        let lay = document::layout_live(&blocks.blocks, raw, None, 500.0, &|_| None);
        assert!(!lay
            .lines
            .iter()
            .any(|l| l.decoration == document::Decoration::HeadingUnderline));
        let mut prefs = Preferences::default();
        prefs.heading_underline[3] = true;
        set(prefs);
        let lay = document::layout_live(&blocks.blocks, raw, None, 500.0, &|_| None);
        let underlines = lay
            .lines
            .iter()
            .filter(|l| l.decoration == document::Decoration::HeadingUnderline)
            .collect::<Vec<_>>();
        assert_eq!(underlines.len(), 1);
        assert_eq!(underlines[0].style, TextStyle::Heading4);
    }

    #[test]
    fn custom_vertical_margins_move_document_ink_and_extend_scroll_height() {
        let _restore = Restore(current());
        let mut prefs = Preferences::default();
        prefs.padding_top = 12.0;
        prefs.padding_bottom = 27.0;
        set(prefs);

        let source = "正文";
        let parsed = document::parse_ranged(source);
        let layout = document::layout_live(&parsed.blocks, source, None, 500.0, &|_| None);
        let line = layout.lines.first().unwrap();
        assert_eq!(line.y, 12.0);
        assert_eq!(layout.height, line.y + line.height + 27.0);
    }

    #[test]
    fn code_and_table_outer_margins_change_their_layout_positions() {
        let _restore = Restore(current());
        let code_source = "前文\n\n```text\ncode\n```";
        let code_parsed = document::parse_ranged(code_source);
        let mut prefs = Preferences::default();
        set(prefs.clone());
        let baseline_code =
            document::layout_live(&code_parsed.blocks, code_source, None, 600.0, &|_| None);
        let baseline_code_y = baseline_code
            .lines
            .iter()
            .find(|line| matches!(line.decoration, document::Decoration::CodeHeader { .. }))
            .unwrap()
            .y;
        prefs.code_margin = 35.0;
        set(prefs.clone());
        let adjusted_code =
            document::layout_live(&code_parsed.blocks, code_source, None, 600.0, &|_| None);
        let adjusted_code_y = adjusted_code
            .lines
            .iter()
            .find(|line| matches!(line.decoration, document::Decoration::CodeHeader { .. }))
            .unwrap()
            .y;

        let table_source = "前文\n\n| 表头 |\n| --- |\n| 内容 |";
        let table_parsed = document::parse_ranged(table_source);
        set(prefs.clone());
        let baseline_table =
            document::layout_live(&table_parsed.blocks, table_source, None, 600.0, &|_| None);
        let baseline_table_y = baseline_table
            .lines
            .iter()
            .find(|line| {
                matches!(
                    line.decoration,
                    document::Decoration::TableRow { header: true, .. }
                )
            })
            .unwrap()
            .y;
        prefs.table_margin = 7.0;
        set(prefs);
        let adjusted_table =
            document::layout_live(&table_parsed.blocks, table_source, None, 600.0, &|_| None);
        let adjusted_table_y = adjusted_table
            .lines
            .iter()
            .find(|line| {
                matches!(
                    line.decoration,
                    document::Decoration::TableRow { header: true, .. }
                )
            })
            .unwrap()
            .y;

        assert_eq!(adjusted_code_y, baseline_code_y + 15.0);
        assert_eq!(adjusted_table_y, baseline_table_y - 13.0);
    }

    #[test]
    fn editor_color_overrides_reach_blocks_without_replacing_inline_styles() {
        let _restore = Restore(current());
        let mut prefs = Preferences::default();
        prefs.document_text_color = Some(0x123456);
        prefs.heading_text_color = Some(0x223344);
        prefs.quote_text_color = Some(0x334455);
        prefs.quote_border_color = Some(0x445566);
        prefs.list_marker_color = Some(0x556677);
        prefs.code_background_color = Some(0x182838);
        prefs.code_foreground_color = Some(0x667788);
        prefs.code_border_color = Some(0x778899);
        prefs.table_text_color = Some(0x8899aa);
        prefs.table_header_background_color = Some(0x99aabb);
        prefs.table_border_color = Some(0xaabbcc);
        set(prefs);

        let source = "正文 <span style=\"color:#ab1234\">显色</span>\n\n# 标题\n\n> 引用\n\n- 项目\n\n```text\ncode\n```\n\n| 表头 | 内容 |\n| --- | --- |\n| 单元格 | 值 |";
        let parsed = document::parse_ranged(source);
        let layout = document::layout_live(&parsed.blocks, source, None, 600.0, &|_| None);
        let mut list = DrawList::new();
        let palette = *theme::tokens().palette(false);
        document::paint(
            &mut list,
            Rect::new(0.0, 0.0, 600.0, 1200.0),
            &layout,
            0.0,
            &palette,
        );

        let has_text_color = |text: &str, color: u32| {
            list.cmds().iter().any(|command| {
                matches!(
                    command,
                    crate::ui::draw::DrawCmd::Text { text: painted, color: painted_color, .. }
                        if painted == text && *painted_color == color
                )
            })
        };
        assert!(has_text_color("正文 ", 0x123456));
        assert!(has_text_color("标题", 0x223344));
        assert!(has_text_color("引用", 0x334455));
        assert!(has_text_color("•", 0x556677));
        assert!(has_text_color("code", 0x667788));
        assert!(has_text_color("表头", 0x8899aa));
        assert!(has_text_color("单元格", 0x8899aa));
        assert!(list.cmds().iter().any(|command| matches!(
            command,
            crate::ui::draw::DrawCmd::Text { text, emphasis: crate::ui::text::Emphasis::Styled { fg: Some(0xab1234), .. }, .. }
                if text == "显色"
        )));
        assert!(list.cmds().iter().any(|command| matches!(
            command,
            crate::ui::draw::DrawCmd::Rect { color, .. } if *color == 0x445566
        )));
        assert!(list.cmds().iter().any(|command| matches!(
            command,
            crate::ui::draw::DrawCmd::RoundedRect { color, .. } if *color == 0x182838
        )));
        assert!(list.cmds().iter().any(|command| matches!(
            command,
            crate::ui::draw::DrawCmd::RoundedBorder { color, .. } if *color == 0x778899
        )));
        assert!(list.cmds().iter().any(|command| matches!(
            command,
            crate::ui::draw::DrawCmd::Rect { color, .. } if *color == 0xaabbcc
        )));
        assert!(list.cmds().iter().any(|command| matches!(
            command,
            crate::ui::draw::DrawCmd::Rect { color, .. } if *color == 0x99aabb
        )));
        assert!(list.finish().is_ok());
    }

    #[test]
    fn typography_changes_layout_without_enlarging_application_labels() {
        let _restore = Restore(current());
        let mut p = Preferences::default();
        let label = TextStyle::Label.font_size();
        let body = TextStyle::Body.font_size();
        p.font_size = 28.0;
        p.line_height = 2.0;
        p.heading_size[3] = 22.0;
        p.padding_left = 60.0;
        p.padding_right = 20.0;
        p.max_width = 400.0;
        p.table_size = 24.0;
        p.table_padding = 16.0;
        set(p);
        assert_eq!(TextStyle::Document.line_height(), 56.0);
        assert_eq!(TextStyle::Heading4.font_size(), 22.0);
        assert_eq!(TextStyle::Label.font_size(), label);
        assert_eq!(TextStyle::Body.font_size(), body);
        let area = Rect::new(0.0, 0.0, 800.0, 600.0);
        assert_eq!(document::content_width(area), 400.0);
        let raw = "#### 四级标题\n\n| 中文 | 表格 |\n| --- | --- |\n| 内容 | 内容 |\n";
        let blocks = document::parse_ranged(raw);
        let layout = document::layout_live(&blocks.blocks, raw, None, 400.0, &|_| None);
        assert!(layout.lines.iter().any(|l| l.style == TextStyle::Heading4));
        assert!(layout
            .lines
            .iter()
            .filter(|l| l.style == TextStyle::Table)
            .all(|l| l.height >= 67.0));
        let mut list = DrawList::new();
        document::paint(
            &mut list,
            area,
            &layout,
            0.0,
            theme::tokens().palette(false),
        );
        assert!(list.finish().is_ok());
    }
}
