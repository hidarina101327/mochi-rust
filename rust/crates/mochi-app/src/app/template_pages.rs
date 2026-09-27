//! 校验并展示模板、最近使用记录和模板选择对话框。
use super::*;

impl App {
    /// 在隔离的快照工作区中实际执行资料库筛选和复制操作。
    #[cfg(debug_assertions)]
    pub(super) fn verify_templates(
        &mut self,
        folder: &Path,
        output: &Path,
        snapshot: &crate::gfx::Snapshot,
        scenario: &str,
    ) -> anyhow::Result<()> {
        use crate::ui::templates::{Format, Hit};
        use anyhow::{ensure, Context};
        self.state.view = WorkspaceView::Templates;
        self.paint(HWND::default())?;
        ensure!(self.views.templates.items.len() == 24);
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.renderer.save_snapshot(snapshot, output)?;
        let group = if scenario.contains("code") {
            "开发"
        } else {
            "学习"
        };
        let index = self
            .views
            .templates
            .groups
            .iter()
            .position(|g| g == group)
            .context("missing group")?;
        for hit in [Hit::Group(index), Hit::Format(Format::Mochi)] {
            let r = self
                .views
                .templates_layout
                .entries
                .iter()
                .find(|(_, h)| *h == hit)
                .context("missing filter")?
                .0;
            self.on_templates_click(r.left + 8.0, r.top + 8.0);
            self.paint(HWND::default())?;
        }
        ensure!(self.views.templates_layout.visible_count == if group == "学习" { 2 } else { 4 });
        let stem = output.file_stem().unwrap_or_default().to_string_lossy();
        self.renderer.save_snapshot(
            snapshot,
            &output.with_file_name(format!("{stem}-filtered.png")),
        )?;
        let name = if group == "学习" {
            "错题复盘"
        } else {
            "代码笔记"
        };
        let index = self
            .views
            .templates
            .items
            .iter()
            .position(|t| t.name == name)
            .context("missing template")?;
        let template = self.views.templates.items[index].clone();
        let original = std::fs::read_to_string(&template.path)?;
        let mut r = self
            .views
            .templates_layout
            .entries
            .iter()
            .find(|(_, h)| *h == Hit::Use(index))
            .context("missing copy action")?
            .0;
        if r.bottom > self.views.templates_layout.list.bottom {
            self.views.templates.scroll += r.bottom - self.views.templates_layout.list.bottom;
            self.paint(HWND::default())?;
            r = self
                .views
                .templates_layout
                .entries
                .iter()
                .find(|(_, h)| *h == Hit::Use(index))
                .unwrap()
                .0;
        }
        self.on_templates_click(r.left + 8.0, r.top + 8.0);
        ensure!(self.state.view == WorkspaceView::Editor);
        let created = self.active_file_path().context("copy not opened")?;
        ensure!(
            created.starts_with(folder)
                && created.extension().and_then(|e| e.to_str()) == Some("mc")
        );
        ensure!(std::fs::read_to_string(&template.path)? == original);
        ensure!(!std::fs::read_to_string(&created)?.contains("{{日期}}"));
        self.paint(HWND::default())?;
        self.renderer.save_snapshot(
            snapshot,
            &output.with_file_name(format!("{stem}-document.png")),
        )?;
        println!(
            "template catalog, filters, copy and source preservation verified: {}",
            output.display()
        );
        Ok(())
    }

    pub(super) fn reload_recent(&mut self) {
        let Some(ws) = self.shell.workspace() else {
            return;
        };
        let libs: Vec<(String, PathBuf)> = ws
            .libraries
            .iter()
            .filter(|l| l.kind != navigation::HIDDEN_TYPE_ID)
            .map(|l| (l.name.clone(), PathBuf::from(&l.path)))
            .collect();
        match mochi_core::files::FileService::new().build_file_tree(&ws.root) {
            Ok(tree) => {
                self.views.recent.docs = views::recent::collect(&tree, &ws.root, &libs);
                self.views.recent.error.clear();
            }
            Err(_) => self.views.recent.error = "无法加载最近文档".into(),
        }
        self.console_filter_recent();
        self.views.recent.loaded = true;
    }

    pub(super) fn reload_templates(&mut self) {
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let service = mochi_core::templates::TemplateService::new(root);
        let result = service.ensure_builtins().and_then(|_| {
            self.views.templates.groups = service.groups()?;
            service.list()
        });
        match result {
            Ok(items) => {
                if items
                    .iter()
                    .any(|t| t.group == mochi_core::templates::UNGROUPED)
                    && !self
                        .views
                        .templates
                        .groups
                        .iter()
                        .any(|g| g == mochi_core::templates::UNGROUPED)
                {
                    self.views
                        .templates
                        .groups
                        .push(mochi_core::templates::UNGROUPED.into());
                }
                self.views.templates.items = items;
                if !self.views.templates.selected_group.is_empty()
                    && !self
                        .views
                        .templates
                        .groups
                        .contains(&self.views.templates.selected_group)
                {
                    self.views.templates.selected_group.clear();
                }
                self.views.templates.error.clear();
            }
            Err(error) => self.views.templates.error = error.to_string(),
        }
    }

    pub(super) fn paint_templates(&mut self, area: Rect, p: &Palette) {
        self.reload_templates();
        let layout = crate::ui::templates::layout(&self.views.templates, area);
        self.views.templates.scroll = self.views.templates.scroll.clamp(0.0, layout.max_scroll);
        crate::ui::templates::paint(&mut self.list, area, &self.views.templates, &layout, p);
        self.views.templates_layout = layout;
    }

    pub(super) fn on_templates_click(&mut self, x: f32, y: f32) {
        use crate::ui::templates::Hit;
        match self.views.templates_layout.hit(x, y) {
            Some(Hit::New) => self.open_template_dialog(String::new()),
            Some(Hit::NewGroup) => self.open_template_group_dialog(),
            Some(Hit::AllGroups) => {
                self.views.templates.selected_group.clear();
                self.views.templates.scroll = 0.0;
            }
            Some(Hit::Format(format)) => {
                self.views.templates.format = format;
                self.views.templates.scroll = 0.0;
            }
            Some(Hit::Group(index)) => {
                if let Some(group) = self.views.templates.groups.get(index).cloned() {
                    self.views.templates.selected_group = group;
                    self.views.templates.scroll = 0.;
                }
            }
            Some(Hit::Template(index)) => {
                if let Some(template) = self.views.templates.items.get(index).cloned() {
                    if self.open_file_from_ui(&template.path) {
                        self.state.view = WorkspaceView::Editor;
                        self.sync_state();
                    }
                }
            }
            Some(Hit::Use(index)) => {
                if let (Some(template), Some(parent)) = (
                    self.views.templates.items.get(index).cloned(),
                    self.shell.tree_root(),
                ) {
                    self.create_document_from_template(parent, template);
                }
            }
            Some(Hit::Delete(index)) => {
                if let Some(template) = self.views.templates.items.get(index).cloned() {
                    match self.shell.workspace().map(|ws| {
                        mochi_core::templates::TemplateService::new(&ws.root).delete(&template)
                    }) {
                        Some(Ok(())) => self.reload_templates(),
                        Some(Err(error)) => self.views.templates.error = error.to_string(),
                        None => {}
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn open_template_dialog(&mut self, content: String) {
        self.dialog = Some(Dialog {
            title: "保存为模板".into(),
            description: "模板会保存在当前工作区，可在新建文档时套用。".into(),
            field: Some(TextField::new("模板名称")),
            error: String::new(),
            note: None,
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "保存模板".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::CreateTemplate(content),
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    pub(super) fn open_template_group_dialog(&mut self) {
        self.dialog = Some(Dialog {
            title: "新建模板分组".into(),
            description: "分组用于整理模板，也会同步保存到当前工作区。".into(),
            field: Some(TextField::new("分组名称")),
            error: String::new(),
            note: None,
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "创建分组".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::CreateTemplateGroup,
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }

    pub(super) fn paint_recent(&mut self, area: Rect, p: &Palette) {
        if !self.views.recent.loaded {
            self.reload_recent();
        }
        let mut lay = views::recent::layout(&self.views.recent, area);
        let max = lay.max_scroll();
        if self.views.recent.scroll > max {
            self.views.recent.scroll = max;
            lay = views::recent::layout(&self.views.recent, area);
        }
        views::recent::paint(
            &mut self.list,
            area,
            &mut self.views.recent,
            &lay,
            self.focus == Focus::RecentQuery,
            Self::now_ms(),
            p,
        );
        self.views.recent_layout = lay;
    }

    pub(super) fn on_recent_click(&mut self, x: f32, y: f32) {
        use views::recent::Hit;
        let Some(hit) = self.views.recent_layout.hit(x, y) else {
            return;
        };
        if hit != Hit::Query && self.focus == Focus::RecentQuery {
            self.focus = Focus::Main;
        }
        match hit {
            Hit::Refresh => self.reload_recent(),
            Hit::Query => {
                self.focus = Focus::RecentQuery;
                if let Some(r) = self.views.recent_layout.rect_of(Hit::Query) {
                    self.views.recent.query.click(x - (r.left + 40.0), false);
                }
            }
            Hit::Doc(i) => {
                let Some(doc) = self.views.recent.filtered().get(i).map(|d| (*d).clone()) else {
                    return;
                };
                // TSX：先选中它所属的库，再开标签
                if let Some(lib) = &doc.library {
                    if let Some(idx) = self
                        .shell
                        .workspace()
                        .and_then(|ws| ws.libraries.iter().position(|l| &l.name == lib))
                    {
                        self.shell.select_library(idx);
                    }
                }
                if self.open_file_from_ui(&doc.path) {
                    self.state.view = WorkspaceView::Editor;
                    self.invalidate_main();
                    self.sync_state();
                }
            }
            Hit::Blank => {}
        }
    }
}
