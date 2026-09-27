//! 处理编辑器格式工具栏、快捷键和文字样式修改。
use super::*;

impl App {
    /// 光标所在行的标题级别（工具栏按钮文字与宽度取决于它）。
    pub(super) fn current_heading_level(&self) -> u8 {
        self.shell
            .active()
            .and_then(|t| t.buffer())
            .map(|b| format::heading_level_at(b.text(), b.cursor()))
            .unwrap_or(0)
    }

    /// 上一帧工具栏的排布（命中测试用）。
    pub(super) fn toolbar_layout(&self) -> toolbar::Layout {
        toolbar::layout_mode(
            self.toolbar_area,
            self.current_heading_level(),
            crate::ui::settings_values::boolean("editor.simpleDocumentMode", true),
        )
    }

    /// 工具栏按钮。
    pub(super) fn on_toolbar_click(&mut self, hit: toolbar::Hit) {
        let lay = self.toolbar_layout();
        let viewport = self.renderer.viewport();
        match hit {
            toolbar::Hit::Color(highlight) => {
                let colors = [
                    ("默认", if highlight { 0xFFF2CC } else { 0x000000 }),
                    ("红色", 0xEF4444),
                    ("橙色", 0xF97316),
                    ("黄色", 0xEAB308),
                    ("绿色", 0x22C55E),
                    ("青色", 0x14B8A6),
                    ("蓝色", 0x3B82F6),
                    ("靛蓝", 0x6366F1),
                    ("紫色", 0xA855F7),
                    ("粉色", 0xEC4899),
                    ("灰色", 0x64748B),
                    ("白色", 0xFFFFFF),
                ];
                let items = colors
                    .into_iter()
                    .map(|(name, color)| {
                        MenuItem::new(
                            format!("●  {name}"),
                            MenuAction::TextColor(highlight, color),
                        )
                    })
                    .collect();
                let anchor = lay
                    .entries
                    .iter()
                    .find(|(_, candidate)| *candidate == toolbar::Hit::Color(highlight))
                    .map(|(rect, _)| *rect)
                    .unwrap_or(self.toolbar_area);
                self.menu = Some(Menu::open_at(
                    items,
                    anchor.left,
                    anchor.bottom + 4.0,
                    viewport,
                ));
            }
            toolbar::Hit::Align(alignment) => {
                if let Some(buffer) = self.shell.active_buffer_mut() {
                    let (a, b) = buffer.selection();
                    let (start, _) = crate::ui::format::line_range(buffer.text(), a);
                    let (_, end) = crate::ui::format::line_range(buffer.text(), b);
                    let replacement = buffer.text()[start..end]
                        .split('\n')
                        .map(|l| crate::ui::styles::align_line(l, alignment))
                        .collect::<Vec<_>>()
                        .join("\n");
                    buffer.replace_range(start..end, &replacement);
                    self.after_doc_edit(false);
                }
            }
            toolbar::Hit::ImportMarkdown => {
                if let Some(path) = platform::pick_file(HWND(self.hwnd_raw as *mut _)) {
                    match std::fs::read_to_string(path) {
                        Ok(source) => {
                            if let Some(buffer) = self.shell.active_buffer_mut() {
                                let len = buffer.text().len();
                                buffer.replace_range(0..len, &source);
                                self.after_edit(false);
                            }
                        }
                        Err(e) => self.state.status_text = e.to_string(),
                    }
                }
            }
            toolbar::Hit::ExportMarkdown => {
                if let Some(source) = self.active_file_path() {
                    // 工具栏和文件树右键共用同一个导出面板，避免工具栏只能导出
                    // Markdown、而用户还要另找菜单才能得到 PDF / HTML。
                    self.export_form =
                        Some(crate::ui::export_dialog::State::new(source, "pdf".into()));
                    self.focus = Focus::Dialog;
                }
            }
            toolbar::Hit::Format(f) => self.apply_format(f),
            toolbar::Hit::HeadingPicker => {
                let level = self.current_heading_level();
                let items: Vec<MenuItem<MenuAction>> = toolbar::HEADING_OPTIONS
                    .iter()
                    .map(|(l, label, icon)| {
                        let label = if *l == level {
                            format!("{label}   ✓")
                        } else {
                            (*label).to_owned()
                        };
                        MenuItem::new(label, MenuAction::Format(Format::Heading(*l))).icon(*icon)
                    })
                    .collect();
                self.menu = Some(Menu::open_at(
                    items,
                    lay.heading_rect.left,
                    lay.heading_rect.bottom + 6.0,
                    viewport,
                ));
                self.toolbar_menu = Some(ToolbarMenu::Heading);
            }
            toolbar::Hit::Insert => {
                let items = Self::editor_insert_items();
                self.menu = Some(Menu::open_at(
                    items,
                    lay.insert_rect.left,
                    lay.insert_rect.bottom + 6.0,
                    viewport,
                ));
                self.toolbar_menu = Some(ToolbarMenu::Insert);
            }
        }
    }

    pub fn on_alt_shortcut(&mut self, key: u16, shift: bool, ctrl: bool) -> bool {
        if ctrl && !shift && key == 0x53 {
            self.state.view = WorkspaceView::Schedule;
            self.focus = Focus::Main;
            self.invalidate_main();
            return true;
        }
        if ctrl && !shift && key == 0x44 {
            self.state.view = WorkspaceView::Home;
            self.focus = Focus::Main;
            self.reload_home_dashboard();
            self.invalidate_main();
            return true;
        }
        if ctrl && !shift && (0x30..=0x36).contains(&key) && self.editing_file() {
            self.apply_format(Format::Heading((key - 0x30) as u8));
            return true;
        }
        if key == 0x43 && shift && !ctrl && self.editing_file() {
            self.apply_last_color(true)
        } else {
            false
        }
    }

    pub(super) fn apply_last_color(&mut self, highlight: bool) -> bool {
        let color = self
            .settings
            .get(if highlight {
                "mochi:last-highlight-color"
            } else {
                "mochi:last-text-color"
            })
            .and_then(|s| crate::ui::styles::color(&s))
            .unwrap_or(if highlight { 0xfff2cc } else { 0x000000 });
        self.apply_text_color(highlight, color);
        true
    }

    pub(super) fn apply_text_color(&mut self, highlight: bool, color: u32) {
        if let Some(buffer) = self.shell.active_buffer_mut() {
            let (a, b) = buffer.selection();
            let tag = if highlight { "mark" } else { "span" };
            let css = if highlight {
                "background-color"
            } else {
                "color"
            };
            let open = format!("<{tag} style=\"{css}: #{color:06X}\">");
            let source = buffer.selected_text().to_owned();
            let wrapped = format!("{open}{source}</{tag}>");
            buffer.replace_range_select(a..b, &wrapped, open.len()..open.len() + source.len());
            self.after_doc_edit(false);
        }
    }

    /// 对当前文件执行一个格式化操作，然后走编辑收尾。
    pub(super) fn apply_format(&mut self, f: Format) {
        if f == Format::Table {
            if !self.commit_table_cell() {
                return;
            }
            let toolbar = self.toolbar_layout();
            let anchor = toolbar
                .entries
                .iter()
                .find(|(_, hit)| *hit == toolbar::Hit::Format(Format::Table))
                .map(|(rect, _)| *rect);
            let (x, y) = anchor
                .map(|rect| (rect.left, rect.bottom + 6.0))
                .unwrap_or((self.editor_area.left + 24.0, self.editor_area.top + 8.0));
            self.table_picker = Some(crate::ui::table_picker::Picker::open(
                x,
                y,
                self.renderer.viewport(),
            ));
            return;
        }
        let rich = self.content() == MainContent::Document
            && !crate::ui::editor_preferences::current().live_line_source;
        if rich && matches!(f, Format::Link | Format::Image) {
            let Some(path) = self.active_file_path() else {
                return;
            };
            let Some(buffer) = self.shell.active().and_then(|t| t.buffer()) else {
                return;
            };
            let action = DialogAction::InsertEditorLink {
                image: f == Format::Image,
                path,
                range: buffer.selection(),
                original: buffer.selected_text().into(),
                label: self.doc.selected_text(buffer),
            };
            self.dialog = Some(Dialog {
                title: if f == Format::Image {
                    "插入图片"
                } else {
                    "插入链接"
                }
                .into(),
                description: if f == Format::Image {
                    "输入图片地址或相对路径"
                } else {
                    "输入链接地址"
                }
                .into(),
                field: Some(TextField::new("https://")),
                error: String::new(),
                note: None,
                buttons: vec![
                    DialogButton {
                        label: "取消".into(),
                        kind: ButtonKind::Ghost,
                        action: DialogAction::Dismiss,
                    },
                    DialogButton {
                        label: "插入".into(),
                        kind: ButtonKind::Primary,
                        action,
                    },
                ],
                dismiss: DialogAction::Dismiss,
                hover: None,
            });
            if f == Format::Image {
                if let Some(dialog) = self.dialog.as_mut() {
                    dialog.buttons.insert(
                        1,
                        DialogButton {
                            label: "选择本地图片".into(),
                            kind: ButtonKind::Ghost,
                            action: DialogAction::PickEditorImage,
                        },
                    );
                }
            }
            self.focus = Focus::Dialog;
            return;
        }
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return;
        };
        if rich {
            crate::ui::rich::apply_format(buffer, f);
        } else {
            format::apply(buffer, f);
        }
        self.focus = Focus::Main;
        self.editor_engaged = true;
        self.after_edit(true);
    }
}
