//! 处理侧边栏点击、展开和文件树拖放。
use super::*;

impl App {
    pub(super) fn on_sidebar_click(&mut self, x: f32, y: f32) {
        let Some(hit) = self.side.layout.hit(x, y) else {
            return;
        };
        let hit = match hit {
            SidebarHit::Row(i) | SidebarHit::RowChevron(i) => {
                let Some(row) = self
                    .side
                    .layout
                    .row_path(i)
                    .and_then(|path| self.shell.rows().iter().position(|row| row.path == path))
                else {
                    return;
                };
                if matches!(hit, SidebarHit::RowChevron(_)) {
                    SidebarHit::RowChevron(row)
                } else {
                    SidebarHit::Row(row)
                }
            }
            hit => hit,
        };
        match hit {
            SidebarHit::Title => {
                if self.shell.favorites_selected() {
                    return;
                }
                self.finish_sidebar_edit();
                let Some(ws) = self.shell.workspace() else {
                    return;
                };
                let kind = self.shell.scope_type_id().unwrap_or("knowledge").to_owned();
                let name = ws
                    .library_types
                    .iter()
                    .find(|t| t.id == kind)
                    .map(|t| t.name.as_str())
                    .unwrap_or("知识库");
                let mut items = vec![MenuItem::new(
                    format!("全部{name}"),
                    MenuAction::LibraryScope(None, kind.clone()),
                )
                .icon(Icon::FOLDER)];
                for (i, library) in ws
                    .libraries
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.kind == kind)
                {
                    let mut item = MenuItem::new(
                        &library.name,
                        MenuAction::LibraryScope(Some(i), kind.clone()),
                    );
                    if self.shell.selected_library() == Some(i) {
                        item = item.icon(Icon::CHECK);
                    }
                    items.push(item);
                }
                let r = self
                    .side
                    .layout
                    .rect_of(SidebarHit::Title)
                    .unwrap_or_default();
                self.menu = Some(Menu::open_at(
                    items,
                    r.left,
                    r.bottom + 4.0,
                    self.renderer.viewport(),
                ));
            }
            SidebarHit::ExpandAll => {
                if self.shell.all_expanded() {
                    self.shell.collapse_all();
                } else {
                    self.shell.expand_all();
                }
            }
            SidebarHit::Refresh => {
                self.shell.refresh_tree();
                self.sync_state();
            }
            SidebarHit::Search => {
                self.focus = Focus::SidebarSearch;
                if let Some(r) = self.side.layout.rect_of(SidebarHit::Search) {
                    // 文字起点 = 框左 + pl-8
                    self.side.search.click(x - (r.left + 32.0), false);
                }
            }
            SidebarHit::SearchClear => {
                self.side.search.clear();
                self.side.scroll = 0.0;
            }
            SidebarHit::RowChevron(i) => {
                self.shell.select(i);
                self.shell.toggle(i);
            }
            SidebarHit::Row(i) => {
                self.begin_sidebar_tree_drag(i, x, y);
            }
            SidebarHit::Editor => {
                self.focus = Focus::SidebarEditor;
                if let (Some(e), Some(left)) = (
                    self.side.editing.as_mut(),
                    self.side.layout.editor_text_left(),
                ) {
                    e.field.click(x - left, false);
                }
            }
            SidebarHit::Blank => {
                self.focus = Focus::Main;
            }
        }
    }

    pub(super) fn begin_sidebar_tree_drag(&mut self, row: usize, x: f32, y: f32) {
        // 搜索结果和收藏树并不是完整的同级列表，拖放会让落点语义含混，保留单击。
        if self.shell.favorites_selected()
            || !self.side.search.text().is_empty()
            || self.side.editing.is_some()
        {
            self.activate_sidebar_row(row);
            return;
        }
        let Some(source_row) = self.shell.rows().get(row).cloned() else {
            return;
        };
        if source_row.is_mapped_folder {
            self.activate_sidebar_row(row);
            return;
        }
        let source = source_row.path;
        self.shell.select(row);
        self.side.tree_drag = Some(SidebarTreeDrag {
            source_path: source,
            source_is_dir: source_row.is_dir,
            start_x: x,
            start_y: y,
            active: false,
            drop: None,
        });
        // 立即请求鼠标捕获；是否真拖拽由 8 DIP 阈值决定，松开未越界就是普通点击。
        self.drag = Some(Drag {
            target: DragTarget::SidebarTree,
            grab_offset: 0.0,
        });
    }

    pub(super) fn activate_sidebar_row(&mut self, row: usize) {
        // TSX 的 handleClick：目录切展开，文件单击就开标签。
        self.shell.select(row);
        let Some(item) = self.shell.rows().get(row) else {
            return;
        };
        if item.is_dir {
            self.shell.toggle(row);
        } else {
            let path = item.path.clone();
            if self.open_file_from_ui(&path) {
                self.invalidate_main();
                self.state.view = WorkspaceView::Editor;
                self.sync_state();
            }
        }
        self.focus = Focus::Main;
    }

    pub(super) fn update_sidebar_tree_drag(&mut self, x: f32, y: f32) -> bool {
        let Some(drag) = self.side.tree_drag.as_mut() else {
            return false;
        };
        if !drag.active {
            let dx = x - drag.start_x;
            let dy = y - drag.start_y;
            if dx.hypot(dy) < TREE_DRAG_THRESHOLD {
                return false;
            }
            drag.active = true;
        }
        let next = self.side.layout.drop_row(x, y).and_then(|(row, rect)| {
            let path = self.side.layout.row_path(row)?;
            let target = self.shell.rows().iter().find(|row| row.path == path)?;
            if target.is_mapped_folder
                || target.path == drag.source_path
                || (drag.source_is_dir && target.path.starts_with(&drag.source_path))
            {
                None
            } else {
                // 与 Electron 一致：左侧 30%、中间 40%、右侧 30%。
                let center_band = rect.height() * 0.20;
                let middle = (rect.top + rect.bottom) / 2.0;
                let kind = if target.is_dir && (y - middle).abs() <= center_band {
                    SidebarTreeDropKind::Into
                } else if y < middle {
                    SidebarTreeDropKind::Before
                } else {
                    SidebarTreeDropKind::After
                };
                Some(SidebarTreeDrop {
                    path: target.path.clone(),
                    kind,
                })
            }
        });
        if drag.drop == next {
            return false;
        }
        drag.drop = next;
        true
    }

    pub(super) fn finish_sidebar_tree_drag(&mut self) {
        let Some(drag) = self.side.tree_drag.take() else {
            return;
        };
        if !drag.active {
            if let Some(row) = self
                .shell
                .rows()
                .iter()
                .position(|row| row.path == drag.source_path)
            {
                self.activate_sidebar_row(row);
            }
            return;
        }
        let Some(drop) = drag.drop else {
            return;
        };
        let target = drop.path;
        let position = match drop.kind {
            SidebarTreeDropKind::Before => TreeDropPosition::Before,
            SidebarTreeDropKind::After => TreeDropPosition::After,
            SidebarTreeDropKind::Into => TreeDropPosition::Into,
        };
        if !self.shell.manual_sort_enabled()
            && matches!(position, TreeDropPosition::Before | TreeDropPosition::After)
        {
            self.show_global_notice("当前按名称排序\n切换为手动排序后，可拖拽调整顺序");
            return;
        }
        match self
            .shell
            .move_for_tree_drop(&drag.source_path, &target, position)
        {
            Ok(destination) => {
                if let Some(row) = self
                    .shell
                    .rows()
                    .iter()
                    .position(|row| row.path == destination)
                {
                    self.shell.select(row);
                }
                self.side.hover = None;
                self.sync_state();
                self.invalidate_main();
            }
            Err(error) => self.show_global_notice(&format!("移动失败：{error}")),
        }
    }

    pub fn cancel_sidebar_tree_drag(&mut self) -> bool {
        let changed = self.side.tree_drag.take().is_some();
        if self
            .drag
            .is_some_and(|drag| drag.target == DragTarget::SidebarTree)
        {
            self.drag = None;
        }
        changed
    }

    pub(super) fn start_sidebar_edit(&mut self, kind: EditKind, initial: &str) {
        // 目录下新建：先把目录展开，输入框才有地方出现在它的子项中
        if let EditKind::NewFile { parent } | EditKind::NewFolder { parent } = &kind {
            if !parent.as_os_str().is_empty() {
                self.shell.expand(parent);
            }
        }
        self.side.editing = Some(Editing::new(kind, initial));
        self.focus = Focus::SidebarEditor;
    }

    /// 提交内联编辑。返回 `true` 表示编辑框应关闭。
    pub(super) fn confirm_sidebar_edit(&mut self) -> bool {
        let Some(e) = self.side.editing.as_mut() else {
            return true;
        };
        let is_folder = matches!(e.kind, EditKind::NewFolder { .. })
            || matches!(e.kind, EditKind::Rename { row } if self.shell.rows().get(row).map(|r| r.is_dir).unwrap_or(false));
        let extension = if let EditKind::Rename { row } = &e.kind {
            self.shell
                .rows()
                .get(*row)
                .and_then(|row| row.path.extension())
                .map(|extension| format!(".{}", extension.to_string_lossy()))
                .unwrap_or_default()
        } else {
            sidebar::default_document_extension().to_owned()
        };
        let name =
            match sidebar::finalize_name_with_extension(e.field.text(), is_folder, &extension) {
                Ok(n) => n,
                Err(err) => {
                    e.error = err.to_owned();
                    return false;
                }
            };
        let kind = e.kind.clone();
        let result: anyhow::Result<Option<PathBuf>> = match kind {
            EditKind::NewFile { parent } => self.shell.create_file(&parent, &name).map(Some),
            EditKind::NewFolder { parent } => {
                self.shell.create_folder(&parent, &name).map(|_| None)
            }
            EditKind::Rename { row } => {
                let Some(from) = self.shell.rows().get(row).map(|r| r.path.clone()) else {
                    return true;
                };
                let renamed = self.shell.rename(&from, &name);
                if let Ok(to) = &renamed {
                    if let Some(rest) = self
                        .split
                        .other
                        .as_deref()
                        .and_then(|other| other.strip_prefix(&from).ok())
                    {
                        self.split.other = Some(to.join(rest));
                    }
                    self.remember_split_active();
                }
                renamed.map(|_| None)
            }
        };
        match result {
            Ok(created) => {
                self.side.editing = None;
                self.focus = Focus::Main;
                if let Some(path) = created {
                    if self.open_file_from_ui(&path) {
                        self.state.view = WorkspaceView::Editor;
                    }
                }
                self.invalidate_main();
                self.sync_state();
                true
            }
            Err(err) => {
                if let Some(e) = self.side.editing.as_mut() {
                    e.error = err.to_string();
                }
                false
            }
        }
    }

    /// 失焦：有内容就确认，否则取消（`InlineEditor.handleBlur`）。
    pub(super) fn finish_sidebar_edit(&mut self) {
        let has_text = self
            .side
            .editing
            .as_ref()
            .map(|e| !e.field.text().trim().is_empty())
            .unwrap_or(false);
        if !has_text || !self.confirm_sidebar_edit() {
            self.side.editing = None;
            if self.focus == Focus::SidebarEditor {
                self.focus = Focus::Main;
            }
        }
    }
}
