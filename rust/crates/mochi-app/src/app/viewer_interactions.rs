//! 处理查看器中的点击操作。
use super::*;

impl App {
    pub(super) fn on_viewer_click(&mut self, x: f32, y: f32) {
        if self.on_base_pointer_down(x, y) {
            return;
        }
        let hit = match self.viewer_tab() {
            Some((_, viewer::Content::Base(s))) => base_view::layout(s, self.editor_area)
                .hit(x, y)
                .map(viewer::Hit::Base),
            Some((_, viewer::Content::Spreadsheet(s))) => sheet_view::layout(s, self.editor_area)
                .hit(x, y)
                .map(viewer::Hit::Sheet),
            Some((_, viewer::Content::Exam(s))) => exam_view::layout(s, self.editor_area)
                .hit(x, y)
                .map(viewer::Hit::Exam),
            _ => self.viewer_layout.hit(x, y),
        };
        let Some(hit) = hit else { return };
        let path = self.viewer_tab().map(|(p, _)| p.to_path_buf());
        match hit {
            viewer::Hit::Canvas(hit) => self.canvas_click(hit, x, y),
            viewer::Hit::Base(hit) => self.on_base_click(hit),
            viewer::Hit::Exam(hit) => self.on_exam_click(hit),
            viewer::Hit::Sheet(hit) => match hit {
                sheet_view::Hit::Sheet(index) => {
                    let name = match self.viewer_tab() {
                        Some((_, viewer::Content::Spreadsheet(s))) => {
                            s.data.as_ref().and_then(|w| w.names.get(index).cloned())
                        }
                        _ => None,
                    };
                    if let (Some(path), Some(name)) = (path, name) {
                        self.start_sheet_job(&path, Some(name));
                    }
                }
                sheet_view::Hit::Reload => {
                    if let Some(path) = path {
                        self.start_sheet_job(&path, None);
                    }
                }
                sheet_view::Hit::OpenExternal => {
                    if let Some(path) = path {
                        platform::open_external(&path.to_string_lossy());
                    }
                }
                sheet_view::Hit::ShowInFolder => {
                    if let Some(path) = path {
                        platform::show_in_explorer(&path);
                    }
                }
                sheet_view::Hit::PreviousTabs | sheet_view::Hit::NextTabs => {
                    if let Some(viewer::Content::Spreadsheet(s)) = self.viewer_content_mut() {
                        let max = s
                            .data
                            .as_ref()
                            .map(|d| d.names.len().saturating_sub(1))
                            .unwrap_or(0);
                        s.tab_start = if hit == sheet_view::Hit::NextTabs {
                            (s.tab_start + 1).min(max)
                        } else {
                            s.tab_start.saturating_sub(1)
                        };
                    }
                }
                sheet_view::Hit::Body => {}
            },
            viewer::Hit::ZoomIn
            | viewer::Hit::ZoomOut
            | viewer::Hit::Rotate
            | viewer::Hit::Reset => {
                if let Some(viewer::Content::Image(s)) = self.viewer_content_mut() {
                    match hit {
                        viewer::Hit::ZoomIn => s.zoom_in(),
                        viewer::Hit::ZoomOut => s.zoom_out(),
                        viewer::Hit::Rotate => s.rotate(),
                        _ => s.reset(),
                    }
                }
            }
            viewer::Hit::ImageCanvas => {
                // 拖动平移：记住指针与当前偏移的差
                if let Some(viewer::Content::Image(s)) = self.viewer_content_mut() {
                    s.dragging = Some((x - s.offset.0, y - s.offset.1));
                    self.drag = Some(Drag {
                        target: DragTarget::ViewerPan,
                        grab_offset: 0.0,
                    });
                }
            }
            viewer::Hit::Copy => {
                let now = Self::now_ms().max(0) as u64;
                if let Some(viewer::Content::Code(c)) = self.viewer_content_mut() {
                    let copied = platform::copy_to_clipboard(&c.text());
                    if copied {
                        c.copied_until = Some(now + 2000);
                    }
                    self.show_global_notice(if copied {
                        "已复制代码"
                    } else {
                        "复制失败，请重试"
                    });
                }
            }
            viewer::Hit::OpenExternal => {
                if let Some(p) = path {
                    platform::open_external(&p.to_string_lossy());
                }
            }
            viewer::Hit::OpenLink => {
                if let Some((_, viewer::Content::Link(l))) = self.viewer_tab() {
                    let url = l.url.clone();
                    platform::open_external(&url);
                }
            }
            viewer::Hit::RefreshCache => self.start_link_cache(),
            viewer::Hit::PdfTool("删除选中标注") => self.pdf_delete_selected(),
            viewer::Hit::PdfTool(what) => {
                if let (Some(tool), Some(viewer::Content::Pdf(s))) = (
                    pdf_annotations::Tool::from_label(what),
                    self.viewer_content_mut(),
                ) {
                    s.annotations.tool = tool;
                    s.annotations.draft = None;
                }
                self.focus = Focus::Main;
            }
            viewer::Hit::PdfEditText => {
                if let Some((path, viewer::Content::Pdf(s))) = self.viewer_tab() {
                    if let Some(a) = s.annotations.selected().cloned() {
                        self.pdf_text_dialog(path.to_path_buf(), a);
                    }
                }
            }
            viewer::Hit::PdfPage => {
                if let Some((_, viewer::Content::Pdf(s))) = self.viewer_tab() {
                    let mut field = TextField::new("页码");
                    field.set_text(
                        &viewer::pdf_current_page(self.viewer_layout.body, s).to_string(),
                    );
                    field.select_all();
                    self.dialog = Some(Dialog {
                        title: "跳转到页".into(),
                        description: format!("共 {} 页", s.page_count()),
                        field: Some(field),
                        error: String::new(),
                        note: None,
                        buttons: vec![
                            DialogButton {
                                label: "取消".into(),
                                kind: ButtonKind::Ghost,
                                action: DialogAction::Dismiss,
                            },
                            DialogButton {
                                label: "跳转".into(),
                                kind: ButtonKind::Primary,
                                action: DialogAction::PdfPage,
                            },
                        ],
                        dismiss: DialogAction::Dismiss,
                        hover: None,
                    });
                    self.focus = Focus::Dialog;
                }
            }
            viewer::Hit::PdfReload => {
                // 丢掉工作线程与位图，从头再来
                if let Some(p) = path {
                    self.pdf_jobs.remove(&p);
                    self.renderer
                        .forget_bytes_with_prefix(&format!("mem://pdf/{}#", p.to_string_lossy()));
                    if let Some(viewer::Content::Pdf(s)) = self.viewer_content_mut() {
                        let annotations = s.annotations.clone();
                        *s = viewer::PdfState::new();
                        s.annotations = annotations;
                    }
                }
            }
            viewer::Hit::PdfShowInFolder => {
                if let Some(p) = path {
                    platform::show_in_explorer(&p);
                }
            }
            viewer::Hit::PdfOpenExternal => {
                if let Some(p) = path {
                    platform::open_external(&p.to_string_lossy());
                }
            }
            viewer::Hit::PdfPrev => self.pdf_go_to_page(-1),
            viewer::Hit::PdfNext => self.pdf_go_to_page(1),
            viewer::Hit::Body => self.pdf_pointer_down(x, y),
        }
    }
}
