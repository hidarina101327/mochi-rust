//! 加载、绘制并编辑收件箱内容。
use super::*;

impl App {
    pub(super) fn reload_inbox(&mut self) {
        let Some(ws) = self.shell.workspace() else {
            return;
        };
        let svc = CaptureService::new(&ws.root);
        let _ = svc.initialize();
        self.views.inbox.items = svc.list_items("all");
        // 可归档的库：排除 ai-prompts（TSX 的 archivableLibraries）
        self.views.inbox.libraries = ws
            .libraries
            .iter()
            .filter(|l| l.kind != navigation::HIDDEN_TYPE_ID)
            .map(|l| (l.name.clone(), PathBuf::from(&l.path)))
            .collect();
        if self.views.inbox.target_library >= self.views.inbox.libraries.len() {
            self.views.inbox.target_library = 0;
        }
        self.views.inbox.loaded = true;
        self.nav.inbox_count = svc.count_inbox();
    }

    pub(super) fn paint_inbox(&mut self, area: Rect, p: &Palette) {
        if !self.views.inbox.loaded {
            self.reload_inbox();
        }
        let mut lay = views::inbox::layout(&self.views.inbox, area);
        let max = lay.max_scroll();
        if self.views.inbox.scroll > max {
            self.views.inbox.scroll = max;
            lay = views::inbox::layout(&self.views.inbox, area);
        }
        views::inbox::paint(
            &mut self.list,
            area,
            &mut self.views.inbox,
            &lay,
            self.focus == Focus::InboxEdit,
            Self::now_ms(),
            p,
        );
        self.views.inbox_layout = lay;
    }

    pub(super) fn on_inbox_click(&mut self, x: f32, y: f32) {
        use views::inbox::{Hit, Tab};
        let Some(hit) = self.views.inbox_layout.hit(x, y) else {
            return;
        };
        if hit != Hit::EditField && self.focus == Focus::InboxEdit {
            // 点到编辑框外：TSX 没有失焦保存，这里也只是移开焦点，编辑框留着
            self.focus = Focus::Main;
        }
        let Some(ws) = self.shell.workspace() else {
            return;
        };
        let root = ws.root.clone();
        let svc = CaptureService::new(&root);
        let visible_ids: Vec<String> = self
            .views
            .inbox
            .visible()
            .iter()
            .map(|i| i.id.clone())
            .collect();
        match hit {
            Hit::Refresh => self.reload_inbox(),
            Hit::Capture => {
                self.show_capture();
            }
            Hit::TabInbox => {
                self.views.inbox.tab = Tab::Inbox;
                self.views.inbox.scroll = 0.0;
            }
            Hit::TabArchived => {
                self.views.inbox.tab = Tab::Archived;
                self.views.inbox.scroll = 0.0;
            }
            Hit::TargetLibrary => {
                let Some(r) = self.views.inbox_layout.rect_of(Hit::TargetLibrary) else {
                    return;
                };
                let current = self.views.inbox.target_library;
                let items: Vec<MenuItem<MenuAction>> = self
                    .views
                    .inbox
                    .libraries
                    .iter()
                    .enumerate()
                    .map(|(i, (name, _))| {
                        let mut item = MenuItem::new(name.clone(), MenuAction::PickInboxLibrary(i));
                        if i == current {
                            item = item.icon(Icon::CHECK);
                        }
                        item
                    })
                    .collect();
                self.menu = Some(Menu::open_anchored(items, r, self.renderer.viewport()));
            }
            Hit::Archive(i) => {
                let (Some(id), Some((_, dir))) = (
                    visible_ids.get(i),
                    self.views
                        .inbox
                        .libraries
                        .get(self.views.inbox.target_library),
                ) else {
                    return;
                };
                match svc.archive_to_note(id, dir) {
                    Ok(_) => self.views.inbox.error.clear(),
                    Err(e) => self.views.inbox.error = e.to_string(),
                }
                self.shell.refresh_tree();
                self.reload_inbox();
            }
            Hit::ToTask(i) => {
                let Some(id) = visible_ids.get(i) else { return };
                let agenda = mochi_core::agenda::AgendaStore::new(&root);
                match svc.convert_to_task(id, &agenda) {
                    Ok(_) => self.views.inbox.error.clear(),
                    Err(e) => self.views.inbox.error = e.to_string(),
                }
                self.reload_inbox();
                if self.sched.view.data.is_some() {
                    self.reload_schedule();
                }
            }
            Hit::Edit(i) => {
                if let Some(item) = visible_ids
                    .get(i)
                    .and_then(|id| self.views.inbox.items.iter().find(|x| &x.id == id))
                    .cloned()
                {
                    self.views.inbox.start_edit(&item);
                    self.focus = Focus::InboxEdit;
                }
            }
            Hit::Delete(i) => {
                if let Some(id) = visible_ids.get(i) {
                    if let Err(error) = svc.remove(id) {
                        self.views.inbox.error = error.to_string();
                    }
                }
                self.reload_inbox();
            }
            Hit::SaveEdit(_) => self.save_inbox_edit(),
            Hit::CancelEdit(_) => {
                self.views.inbox.editing = None;
                self.focus = Focus::Main;
            }
            Hit::OpenArchived(i) => {
                let path = visible_ids
                    .get(i)
                    .and_then(|id| self.views.inbox.items.iter().find(|x| &x.id == id))
                    .and_then(|x| {
                        x.archived_path.clone().or_else(|| {
                            x.web_clip
                                .as_ref()
                                .map(|m| root.join(&m.document).to_string_lossy().into_owned())
                        })
                    });
                if let Some(path) = path {
                    if Path::new(&path)
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("html"))
                    {
                        platform::open_external(&path);
                        return;
                    }
                    if self.open_file_from_ui(Path::new(&path)) {
                        self.state.view = WorkspaceView::Editor;
                        self.invalidate_main();
                        self.sync_state();
                    }
                }
            }
            Hit::EditField => {
                self.focus = Focus::InboxEdit;
                if let (Some((_, f)), Some(r)) = (
                    self.views.inbox.editing.as_mut(),
                    self.views.inbox_layout.rect_of(Hit::EditField),
                ) {
                    f.click(x - (r.left + 8.0), false);
                }
            }
            Hit::Blank => {}
        }
    }

    pub(super) fn save_inbox_edit(&mut self) {
        let Some((id, field)) = self.views.inbox.editing.take() else {
            return;
        };
        self.focus = Focus::Main;
        let Some(ws) = self.shell.workspace() else {
            return;
        };
        let content = field.text().trim().to_owned();
        if content.is_empty() {
            return;
        }
        if let Err(error) = CaptureService::new(&ws.root).update(&id, Some(&content), None, None) {
            self.views.inbox.error = error.to_string();
        }
        self.reload_inbox();
    }
}
