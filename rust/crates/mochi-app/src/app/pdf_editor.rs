//! 处理 PDF 标注的保存、删除、定位和指针交互。
use super::*;

impl App {
    pub(super) fn pdf_save_annotations(
        &mut self,
        path: &Path,
        items: Vec<sidecars::PdfAnnotation>,
    ) -> bool {
        match sidecars::save_pdf_annotations(&path.to_string_lossy(), items.clone()) {
            Ok(_) => {
                if let Some(viewer::Content::Pdf(s)) = self.viewer_content_for(path) {
                    s.annotations.items = items;
                }
                self.state.status_text = "PDF 标注已保存".into();
                true
            }
            Err(e) => {
                self.state.status_text = format!("保存 PDF 标注失败：{e}");
                false
            }
        }
    }

    pub(super) fn pdf_delete_selected(&mut self) {
        let Some((path, viewer::Content::Pdf(s))) = self.viewer_tab() else {
            return;
        };
        let Some(id) = s.annotations.selected.clone() else {
            return;
        };
        let path = path.to_path_buf();
        let items = s
            .annotations
            .items
            .iter()
            .filter(|a| a.id != id)
            .cloned()
            .collect();
        if self.pdf_save_annotations(&path, items) {
            if let Some(viewer::Content::Pdf(s)) = self.viewer_content_mut() {
                s.annotations.selected = None;
            }
        }
    }

    pub(super) fn pdf_text_dialog(&mut self, path: PathBuf, a: sidecars::PdfAnnotation) {
        let mut field = TextField::new("标注文字");
        field.set_text(a.text.as_deref().unwrap_or(""));
        self.pdf_text_pending = Some((path, a));
        self.dialog = Some(Dialog {
            title: "标注文字".into(),
            description: String::new(),
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
                    label: "保存".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::PdfText,
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    pub(super) fn pdf_pointer_down(&mut self, x: f32, y: f32) {
        if !crate::ui::settings_values::boolean("editorLayout.annotationsVisible", true) {
            return;
        }
        let body = self.viewer_layout.body;
        let Some((path, viewer::Content::Pdf(s))) = self.viewer_tab() else {
            return;
        };
        let path = path.to_path_buf();
        let found = viewer::pdf_page_rects(body, s)
            .into_iter()
            .enumerate()
            .find_map(|(i, r)| {
                let r = Rect::new(r.left, r.top - s.scroll, r.right, r.bottom - s.scroll);
                r.contains(x, y).then_some((i as i64 + 1, r))
            });
        let Some((page, r)) = found else { return };
        let selected = pdf_annotations::hit(&s.annotations, r, page, x, y);
        let tool = s.annotations.tool;
        let point = pdf_annotations::position(r, x, y);
        self.focus = Focus::Main;
        if let Some(viewer::Content::Pdf(s)) = self.viewer_content_mut() {
            s.annotations.selected = selected.clone();
            if selected.is_some() || tool == pdf_annotations::Tool::Select {
                return;
            }
            if tool != pdf_annotations::Tool::Text {
                s.annotations.begin(page, point, uuid_v4(), Self::now_ms());
            }
        }
        if tool == pdf_annotations::Tool::Text {
            self.pdf_text_dialog(
                path,
                pdf_annotations::new_annotation(tool, page, point, uuid_v4(), Self::now_ms()),
            );
        } else {
            self.drag = Some(Drag {
                target: DragTarget::PdfAnnotation,
                grab_offset: 0.0,
            });
        }
    }

    pub(super) fn pdf_drag_to(&mut self, x: f32, y: f32) -> bool {
        let body = self.viewer_layout.body;
        let Some(viewer::Content::Pdf(s)) = self.viewer_content_mut() else {
            return false;
        };
        let Some(d) = &s.annotations.draft else {
            return false;
        };
        let Some(r) = viewer::pdf_page_rects(body, s)
            .get((d.page - 1) as usize)
            .copied()
        else {
            return false;
        };
        let page = Rect::new(r.left, r.top - s.scroll, r.right, r.bottom - s.scroll);
        s.annotations.move_to(pdf_annotations::position(page, x, y));
        true
    }

    pub(super) fn pdf_locate_annotation(&mut self, id: &str) {
        let body = self.viewer_layout.body;
        let max = self.viewer_layout.max_scroll();
        if let Some(viewer::Content::Pdf(s)) = self.viewer_content_mut() {
            if let Some(a) = s.annotations.items.iter().find(|a| a.id == id) {
                if let Some(r) = viewer::pdf_page_rects(body, s).get((a.page - 1) as usize) {
                    s.scroll = (r.top - body.top - viewer::PDF_LABEL_H).clamp(0.0, max);
                    s.annotations.selected = Some(id.into());
                }
            }
        }
        self.focus = Focus::Main;
    }

    pub(super) fn ensure_pdf_job(&mut self, path: &Path) {
        if self.pdf_jobs.contains_key(path) {
            return;
        }
        let document_path = match self.viewer_content_for(path) {
            Some(viewer::Content::Pdf(s)) => s.document_path.clone(),
            _ => None,
        };
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
            && document_path.is_none()
        {
            if let Some(viewer::Content::Pdf(s)) = self.viewer_content_for(path) {
                if s.converting {
                    return;
                }
                s.converting = true;
            }
            let path = path.to_path_buf();
            self.file_jobs.submit(path.clone(), self.hwnd_raw, move || {
                mochi_core::office::convert_pdf(&path, false)
                    .map(crate::file_runtime::Payload::Office)
            });
            return;
        }
        let (handle, rx) = crate::pdf::open(
            path.to_path_buf(),
            document_path.unwrap_or_else(|| path.to_path_buf()),
            self.hwnd_raw,
        );
        let resume = self
            .settings
            .get(&format!("pdf.last-page:{}", path.to_string_lossy()))
            .and_then(|v| v.parse::<usize>().ok());
        if let Some(viewer::Content::Pdf(s)) = self.viewer_content_for(path) {
            s.resume_page = resume;
        }
        self.pdf_jobs.insert(path.to_path_buf(), (handle, rx));
    }

    /// 把可见页（含前后各一页）交给工作线程渲染。渲染倍率 = 显示宽度 / 页宽 × DPI 缩放，
    /// 这样在 144 DPI 屏上 800 DIP 宽的页面渲成 1200 像素，不糊。
    pub(super) fn request_visible_pdf_pages(&mut self, area: Rect) {
        if let Some(viewer::Content::Pdf(s)) = self.viewer_content_mut() {
            if !s.loading {
                if let Some(page) = s.resume_page.take() {
                    let body = viewer::layout(area, &viewer::Content::Pdf(s.clone())).body;
                    if let Some(r) = viewer::pdf_page_rects(body, s).get(page.saturating_sub(1)) {
                        s.scroll = (r.top - body.top - viewer::PDF_LABEL_H).max(0.0);
                    }
                }
            }
        }
        let dpi_scale = if self.hwnd_raw != 0 {
            let dpi = unsafe {
                windows::Win32::UI::HiDpi::GetDpiForWindow(HWND(self.hwnd_raw as *mut _))
            };
            (dpi.max(96) as f32 / 96.0).max(1.0)
        } else {
            1.0
        };
        let Some((path, viewer::Content::Pdf(s))) = self.viewer_tab() else {
            return;
        };
        if s.loading || s.error.is_some() {
            return;
        }
        let path = path.to_path_buf();
        let lay = viewer::layout(area, &viewer::Content::Pdf(s.clone()));
        let body = lay.body;
        let wanted = viewer::pdf_visible_pages(body, s);
        let evicted = s
            .ready
            .iter()
            .enumerate()
            .filter(|(i, ready)| ready.is_some() && !wanted.contains(i))
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        let rects = viewer::pdf_page_rects(body, s);
        let mut to_request = Vec::new();
        for i in wanted {
            if s.requested.get(i).copied().unwrap_or(true) {
                continue;
            }
            let (w, _) = s.sizes[i];
            let scale = (rects[i].width() / w.max(1.0)) * dpi_scale;
            to_request.push((i, scale));
        }
        if let Some(viewer::Content::Pdf(s)) = self.viewer_content_mut() {
            for i in &evicted {
                s.ready[*i] = None;
                s.requested[*i] = false;
            }
        }
        for i in evicted {
            self.renderer
                .forget_bytes(&viewer::PdfState::page_key(&path, i));
        }
        if to_request.is_empty() {
            return;
        }
        if let Some(viewer::Content::Pdf(s)) = self.viewer_content_mut() {
            for (i, _) in &to_request {
                s.requested[*i] = true;
            }
        }
        if let Some((handle, _)) = self.pdf_jobs.get(&path) {
            for (i, scale) in to_request {
                handle.request(i as u32, scale);
            }
        }
    }

    /// `WM_APP_PDF_EVENT`：把所有工作线程的事件收干。
    pub fn on_pdf_event(&mut self) {
        let mut events = Vec::new();
        for (_, (_, rx)) in self.pdf_jobs.iter() {
            while let Ok(e) = rx.try_recv() {
                events.push(e);
            }
        }
        for e in events {
            match e {
                crate::pdf::PdfEvent::Loaded { path, sizes } => {
                    if let Some(viewer::Content::Pdf(s)) = self.viewer_content_for(&path) {
                        s.loaded(sizes);
                    }
                }
                crate::pdf::PdfEvent::Page {
                    path,
                    index,
                    png,
                    width,
                    height,
                } => {
                    let key = viewer::PdfState::page_key(&path, index as usize);
                    self.renderer.register_bytes(&key, png);
                    if let Some(viewer::Content::Pdf(s)) = self.viewer_content_for(&path) {
                        if let Some(slot) = s.ready.get_mut(index as usize) {
                            *slot = Some((width, height));
                        }
                    }
                }
                crate::pdf::PdfEvent::Failed { path, message } => {
                    if let Some(viewer::Content::Pdf(s)) = self.viewer_content_for(&path) {
                        s.loading = false;
                        s.error = Some(message);
                    }
                }
            }
        }
    }

    /// 某个路径的查看器内容（不一定是当前激活的标签）。
    pub(super) fn viewer_content_for(&mut self, path: &Path) -> Option<&mut viewer::Content> {
        self.shell
            .tabs_mut()
            .iter_mut()
            .find_map(|t| match &mut t.kind {
                TabKind::Viewer { path: p, content } if p == path => Some(content),
                _ => None,
            })
    }

    /// 关掉标签后释放对应的 PDF 工作线程与位图。`sync_state` 每次都调。
    pub(super) fn prune_pdf_jobs(&mut self) {
        let open: HashSet<PathBuf> = self
            .shell
            .tabs()
            .iter()
            .filter_map(|t| t.path().map(Path::to_path_buf))
            .collect();
        let stale: Vec<PathBuf> = self
            .pdf_jobs
            .keys()
            .filter(|p| !open.contains(*p))
            .cloned()
            .collect();
        for p in stale {
            self.pdf_jobs.remove(&p);
            self.renderer
                .forget_bytes_with_prefix(&format!("mem://pdf/{}#", p.to_string_lossy()));
        }
    }

    /// 翻页：把目标页的顶滚到视口顶。
    pub(super) fn pdf_go_to_page(&mut self, delta: i32) {
        let body = self.viewer_layout.body;
        let max = self.viewer_layout.max_scroll();
        if let Some(viewer::Content::Pdf(s)) = self.viewer_content_mut() {
            let n = s.page_count();
            if n == 0 {
                return;
            }
            let current = viewer::pdf_current_page(body, s) as i32 - 1;
            let target = (current + delta).clamp(0, n as i32 - 1) as usize;
            let rects = viewer::pdf_page_rects(body, s);
            s.scroll = (rects[target].top - viewer::PDF_LABEL_H - body.top).clamp(0.0, max);
        }
        self.remember_pdf_page();
    }

    pub(super) fn remember_pdf_page(&mut self) {
        if let Some((path, viewer::Content::Pdf(s))) = self.viewer_tab() {
            if s.page_count() > 0 {
                self.settings.set(
                    &format!("pdf.last-page:{}", path.to_string_lossy()),
                    &viewer::pdf_current_page(self.viewer_layout.body, s).to_string(),
                );
                let _ = self.settings.flush();
            }
        }
    }

    /// 拖动平移中。返回是否需要重画。
    pub(super) fn on_viewer_pan(&mut self, x: f32, y: f32) -> bool {
        if let Some(viewer::Content::Image(s)) = self.viewer_content_mut() {
            if let Some((sx, sy)) = s.dragging {
                s.offset = (x - sx, y - sy);
                return true;
            }
        }
        false
    }

    pub(super) fn on_viewer_wheel(&mut self, x: f32, y: f32, notches: f32) {
        if self.editor_area.contains(x, y) {
            let area = self.editor_area;
            let horizontal = shift_down();
            if let Some(viewer::Content::Base(state)) = self.viewer_content_mut() {
                state.scroll(area, notches, horizontal);
                return;
            }
        }
        if !self.viewer_layout.body.contains(x, y) {
            return;
        }
        let max = self.viewer_layout.max_scroll();
        let body = self.viewer_layout.body;
        let ctrl = unsafe {
            windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(
                windows::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL.0 as i32,
            ) < 0
        };
        let horizontal = shift_down();
        let area = self.editor_area;
        match self.viewer_content_mut() {
            Some(viewer::Content::Base(s)) => s.scroll(area, notches, horizontal),
            Some(viewer::Content::Exam(s)) => {
                s.scroll =
                    (s.scroll - notches * 90.0).clamp(0.0, exam_view::layout(s, area).max_scroll())
            }
            Some(viewer::Content::Spreadsheet(s)) => {
                let lay = sheet_view::layout(s, area);
                if horizontal {
                    s.scroll_x = (s.scroll_x - notches * 160.0 * 3.0).clamp(0.0, lay.max_x);
                } else {
                    s.scroll_y = (s.scroll_y - notches * 30.0 * 3.0).clamp(0.0, lay.max_y);
                }
            }
            Some(viewer::Content::Canvas(_)) if ctrl => {
                self.canvas_zoom_at(x, y, 1.2_f64.powf(f64::from(notches)));
            }
            Some(viewer::Content::Canvas(_)) => self.canvas_scroll(notches, horizontal),
            Some(viewer::Content::Image(s)) => {
                if notches > 0.0 {
                    s.zoom_in();
                } else {
                    s.zoom_out();
                }
            }
            Some(viewer::Content::Code(c)) => {
                c.scroll = (c.scroll - notches * 24.0 * 3.0).clamp(0.0, max);
            }
            Some(viewer::Content::Link(s)) => {
                s.scroll = (s.scroll - notches * 72.0).clamp(0.0, max)
            }
            Some(viewer::Content::Pdf(s)) => {
                if ctrl {
                    s.zoom_by(body, if notches > 0.0 { 1.1 } else { 1.0 / 1.1 });
                } else {
                    s.scroll = (s.scroll - notches * 40.0 * 3.0).clamp(0.0, max);
                }
            }
            _ => {}
        }
        self.remember_pdf_page();
    }
}
