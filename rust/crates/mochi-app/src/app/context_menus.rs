//! 生成并处理右键菜单和标签页菜单。
use super::*;

impl App {
    /// 根据实际绘制的导航和内容位置处理右键菜单。
    pub fn on_right_click(&mut self, x: f32, y: f32) {
        if self.global_import.is_some() {
            return;
        }
        if self.desktop_manager_captures_pointer(x, y) {
            return;
        }
        if self.notification_open() {
            return;
        }
        if self.automation.panel.is_some() {
            return;
        }
        if self.object_picker.is_some() {
            return;
        }
        if self.image_preview.is_some() {
            return;
        }
        if self.export_form.is_some() {
            return;
        }
        if self.commands.review.is_some() {
            return;
        }
        if self.dialog.is_some() || self.search.is_some() {
            return;
        }
        if self.settings_overlay.is_some() {
            return;
        }
        self.cancel_navigation_library_drag();
        let title_bar_setting = match self.build_chrome().hit(x, y) {
            Some(NodeKey::TitleBarAiToggle) => Some("navigation.titleBar.assistant"),
            Some(NodeKey::TitleBarNotifications) => Some("navigation.titleBar.notifications"),
            Some(NodeKey::TitleBarDesktop) => Some("navigation.titleBar.desktop"),
            Some(NodeKey::TitleBarMarketplace) => Some("navigation.titleBar.marketplace"),
            Some(NodeKey::TitleBarTemplates) => Some("navigation.titleBar.templates"),
            Some(NodeKey::TitleBarAutomations) => Some("navigation.titleBar.automations"),
            _ => None,
        };
        if let Some(key) = title_bar_setting {
            self.menu = Some(Menu::open_at(
                vec![MenuItem::new("隐藏", MenuAction::HideTitleBarEntry(key))],
                x,
                y,
                self.renderer.viewport(),
            ));
            return;
        }
        if self.workflows_active() && self.workflows.view.area.contains(x, y) {
            self.workflows_context_menu(x, y);
            return;
        }
        if self.build_chrome().hit(x, y) == Some(NodeKey::Navigation) {
            self.menu = self.nav_layout.hit(x, y).and_then(|hit| match hit {
                NavHit::Item(item) => Some(Menu::open_at(
                    vec![MenuItem::new("隐藏", MenuAction::HideNavigation(item))],
                    x,
                    y,
                    self.renderer.viewport(),
                )),
                _ => None,
            });
            return;
        }
        // 原生 .mcb 单元格没有浏览器的右键菜单层。根据
        // 实际绘制时使用的布局定位，避免菜单指向已滚动到
        // 视口外或被裁切的相邻单元格。AI 快照会显示为编辑器中的
        // 上下文标签，而不是一段无法辨认来源的提示文本。
        if let Some((path, viewer::Content::Base(state))) = self.viewer_tab() {
            if let Some(base_view::Hit::Cell(record, field)) =
                base_view::layout(state, self.base_interaction_area()).hit(x, y)
            {
                if let Some(context) = state.cell_ai_context(record, field) {
                    let mut items = Vec::new();
                    if state.editing {
                        items.extend([
                            MenuItem::new(
                                "向上新增记录",
                                MenuAction::BaseCellInsert(
                                    record,
                                    field,
                                    base_view::InsertDirection::Above,
                                ),
                            )
                            .icon(Icon::ARROW_UP),
                            MenuItem::new(
                                "向下新增记录",
                                MenuAction::BaseCellInsert(
                                    record,
                                    field,
                                    base_view::InsertDirection::Below,
                                ),
                            )
                            .icon(Icon::ARROW_DOWN),
                            MenuItem::new(
                                "向左新增字段",
                                MenuAction::BaseCellInsert(
                                    record,
                                    field,
                                    base_view::InsertDirection::Left,
                                ),
                            )
                            .icon(Icon::ARROW_LEFT),
                            MenuItem::new(
                                "向右新增字段",
                                MenuAction::BaseCellInsert(
                                    record,
                                    field,
                                    base_view::InsertDirection::Right,
                                ),
                            )
                            .icon(Icon::ARROW_RIGHT),
                        ]);
                    }
                    items.push(
                        MenuItem::new(
                            "发送至 AI 助手",
                            MenuAction::BaseCellToAssistant(path.to_path_buf(), context),
                        )
                        .icon(Icon::BOT),
                    );
                    self.menu = Some(Menu::open_at(items, x, y, self.renderer.viewport()));
                    return;
                }
            }
        }
        if self.ai_selection_contains(x, y) {
            if let Some(text) = self.ai_selected_text() {
                self.menu = Some(Menu::open_at(
                    vec![MenuItem::new("复制", MenuAction::CopyObjectLink(text))],
                    x,
                    y,
                    self.renderer.viewport(),
                ));
                return;
            }
        }
        if let Some(url) = self.schedule_link_at(x, y) {
            self.menu = Some(Menu::open_at(
                vec![
                    MenuItem::new("复制为 Mochi 链接", MenuAction::CopyObjectLink(url))
                        .icon(Icon::LINK2),
                ],
                x,
                y,
                self.renderer.viewport(),
            ));
            return;
        }
        if self.ai_message_context(x, y) {
            return;
        }
        if self.ai_session_context(x, y) {
            return;
        }
        self.menu = None;
        if self.state.view == WorkspaceView::Editor
            && matches!(self.content(), MainContent::Document | MainContent::Source)
            && self.split.pane_area.contains(x, y)
            && !self.focus_other_editor()
        {
            return;
        }
        let chrome = self.build_chrome();
        if chrome.hit(x, y) == Some(NodeKey::Editor) && self.state.view == WorkspaceView::Recent {
            self.on_document_list_context(x, y);
            return;
        }
        if self.settings_overlay.is_some() {
            return;
        }
        if chrome.hit(x, y) == Some(NodeKey::Editor) && self.state.view == WorkspaceView::Home {
            return;
        }
        if chrome.hit(x, y) == Some(NodeKey::Editor)
            && matches!(self.content(), MainContent::Document | MainContent::Source)
        {
            self.focus = Focus::Main;
            self.editor_engaged = true;
        }
        if chrome.hit(x, y) == Some(NodeKey::Editor) && self.content() == MainContent::Document {
            if !self.commit_table_cell() {
                return;
            }
            if let Some(start) =
                self.doc
                    .image_at(self.editor_area, self.shell.active_scroll(), x, y)
            {
                self.open_block_menu(start, false, x, y);
                return;
            }
            if let Some(buffer) = self.shell.active().and_then(|t| t.buffer()) {
                if let Some(cell) = self
                    .doc
                    .table_cells(self.editor_area, buffer.text(), self.shell.active_scroll())
                    .into_iter()
                    .find(|c| c.rect.contains(x, y))
                {
                    let mut items = Self::clipboard_menu_items();
                    items.extend(table_edit::ACTIONS.iter().map(|(label, action)| {
                        MenuItem::new(*label, MenuAction::Table(cell.range.start, *action))
                    }));
                    if let Some((_, start, false)) =
                        self.doc
                            .block_handle(self.editor_area, self.shell.active_scroll(), x, y)
                    {
                        items.extend(Self::editor_block_items(start));
                    }
                    self.begin_table_cell(cell, x);
                    self.menu = Some(Menu::open_at(items, x, y, self.renderer.viewport()));
                    return;
                }
            }
            if let Some((start, _)) =
                self.doc
                    .container_at(self.editor_area, self.shell.active_scroll(), x, y)
            {
                self.open_container_menu(start, x, y);
                return;
            }
            let selected = self
                .shell
                .active()
                .and_then(|tab| tab.buffer())
                .is_some_and(|buffer| buffer.has_selection());
            if !selected {
                if let Some((_, start, false)) =
                    self.doc
                        .block_handle(self.editor_area, self.shell.active_scroll(), x, y)
                {
                    let functional = self
                        .shell
                        .active()
                        .and_then(|tab| tab.buffer())
                        .is_some_and(|buffer| {
                            crate::ui::document::parse_ranged(buffer.text())
                                .blocks
                                .iter()
                                .any(|block| {
                                    block.start == start
                                        && matches!(
                                            block.block,
                                            crate::ui::document::Block::Code { .. }
                                                | crate::ui::document::Block::Math(_)
                                                | crate::ui::document::Block::Table { .. }
                                                | crate::ui::document::Block::Divider
                                                | crate::ui::document::Block::ObjectReference(_)
                                                | crate::ui::document::Block::AiLocator(_)
                                        )
                                })
                        });
                    if functional {
                        self.open_block_menu(start, false, x, y);
                        return;
                    }
                }
            }
        }
        if chrome.hit(x, y) == Some(NodeKey::Editor)
            && self
                .shell
                .active()
                .and_then(|t| t.buffer())
                .is_some_and(|b| b.has_selection())
        {
            let configured = self
                .shell
                .workspace()
                .map(|ws| {
                    mochi_core::ai::agent_config::AgentConfigService::new(&ws.root)
                        .load_quick_actions()
                })
                .unwrap_or_default();
            let mut items = if configured.is_empty() {
                [
                ("继续写作","继续写作以下内容，保持风格和格式一致，直接输出续写部分：\n\n{{selection}}",true),
                ("润色","润色以下文本，修正错别字，优化表达，保持原有格式和结构不变，直接输出完整内容：\n\n{{selection}}",false),
                ("总结","用简洁的语言总结以下内容：\n\n{{selection}}",false),
                ("翻译","将以下文本翻译成{{targetLanguage}}，保持原有格式：\n\n{{selection}}",false),
                ("格式调整","调整以下内容的 Markdown 格式使其更易读，保持内容不变：\n\n{{selection}}",false),
                ("重点标注","用**粗体**标记关键概念和核心术语，保持原有结构不变：\n\n{{selection}}",false),
            ].into_iter().map(|(label,prompt,append)|MenuItem::new(label,MenuAction::SelectionAi(prompt.into(),append)).icon(Icon::SPARKLES)).collect::<Vec<_>>()
            } else {
                configured
                    .into_iter()
                    .filter(|a| a.enabled)
                    .map(|a| {
                        MenuItem::new(
                            a.name,
                            MenuAction::SelectionAi(a.prompt_template, a.apply == "append"),
                        )
                        .icon(Icon::SPARKLES)
                    })
                    .collect()
            };
            items.push(
                MenuItem::new("发送选区到 AI 助手", MenuAction::SelectionToAssistant)
                    .icon(Icon::BOT)
                    .separated(),
            );
            items.insert(0, MenuItem::new("剪切", MenuAction::EditorCut));
            items.insert(
                0,
                MenuItem::new("复制", MenuAction::EditorCopy).icon(Icon::COPY),
            );
            items.insert(2, MenuItem::new("粘贴", MenuAction::EditorPaste));
            items.insert(3, MenuItem::new("纯文本粘贴", MenuAction::EditorPastePlain));
            items.insert(
                4,
                MenuItem::new("全选", MenuAction::EditorSelectAll).separated(),
            );
            items.insert(
                5,
                MenuItem::new("评论选中内容", MenuAction::CommentSelection)
                    .icon(Icon::MESSAGE_SQUARE)
                    .separated(),
            );
            if self.content() == MainContent::Document {
                if let Some((_, start, false)) =
                    self.doc
                        .block_handle(self.editor_area, self.shell.active_scroll(), x, y)
                {
                    items.extend(Self::editor_block_items(start));
                }
            }
            self.menu = Some(Menu::open_at(items, x, y, self.renderer.viewport()));
            return;
        }
        if chrome.hit(x, y) == Some(NodeKey::Editor) && self.editing_file() {
            let mut items = Self::clipboard_menu_items();
            items.push(MenuItem::new("全选", MenuAction::EditorSelectAll).separated());
            if self.content() == MainContent::Document {
                if let Some((_, start, false)) =
                    self.doc
                        .block_handle(self.editor_area, self.shell.active_scroll(), x, y)
                {
                    items.extend(Self::editor_block_items(start));
                }
            }
            self.menu = Some(Menu::open_at(items, x, y, self.renderer.viewport()));
            return;
        }
        if chrome.hit(x, y) == Some(NodeKey::TabBar) {
            let tabs = self.tab_projection();
            if let Some(hit) = tab_bar::hit_scrolled_with_fixed_width(
                tab_bar::tabs_content_area(chrome.tree.rect(chrome.tab_bar)),
                &tabs,
                self.state.compact_tab_bar,
                self.tab_fixed_width(),
                self.tab_scroll,
                x,
                y,
            ) {
                let index = match hit {
                    tab_bar::Hit::Select(i) | tab_bar::Hit::Close(i) => i,
                };
                self.menu = Some(Menu::open_at(
                    self.tab_context_menu_items(index),
                    x,
                    y,
                    self.renderer.viewport(),
                ));
            }
            return;
        }
        if chrome.hit(x, y) != Some(NodeKey::Sidebar) {
            return;
        }
        let viewport = self.renderer.viewport();
        let items = match self.side.layout.hit(x, y) {
            Some(SidebarHit::Row(i)) | Some(SidebarHit::RowChevron(i)) => {
                self.shell.select(i);
                self.side.hover = Some(i);
                let row = &self.shell.rows()[i];
                let mut items = Self::row_menu_items(i, &row.path, row.is_dir);
                if !row.is_dir {
                    items.insert(0, self.favorite_menu_item(&row.path));
                }
                items
            }
            Some(SidebarHit::Blank) => Self::blank_menu_items(),
            _ => return,
        };
        // Radix 的 sideOffset=5：菜单离指针 5px
        self.menu = Some(Menu::open_at(items, x + 5.0, y + 5.0, viewport));
    }

    /// 标签右键菜单与 Electron 的 `TabContextMenu.tsx` 同序；模板是原生版
    /// 额外保留的工作区功能。把菜单构造集中在这里，避免某条点击路径只显示
    /// “另存为模板 / 关闭”这样的残缺版本。
    pub(super) fn tab_context_menu_items(&self, index: usize) -> Vec<MenuItem<MenuAction>> {
        let Some(tab) = self.shell.tabs().get(index) else {
            return Vec::new();
        };
        let path = tab.path().map(PathBuf::from);
        let file_tab = path.is_some();
        let has_tabs_to_right = self
            .shell
            .tabs()
            .iter()
            .skip(index + 1)
            .any(|candidate| !candidate.pinned);
        let has_other_tabs = self
            .shell
            .tabs()
            .iter()
            .enumerate()
            .any(|(candidate_index, candidate)| candidate_index != index && !candidate.pinned);
        let has_unpinned_tabs = self.shell.tabs().iter().any(|candidate| !candidate.pinned);

        let mut items = vec![
            MenuItem::new(
                "刷新",
                MenuAction::RefreshTab(path.clone().unwrap_or_default()),
            )
            .icon(Icon::REFRESH_CW)
            .disabled(!file_tab),
            MenuItem::new("关闭", MenuAction::CloseTab(index))
                .icon(Icon::X)
                .disabled(tab.pinned),
            MenuItem::new("关闭其他", MenuAction::CloseTabs(index, "others"))
                .icon(Icon::X)
                .disabled(!has_other_tabs),
            MenuItem::new("关闭右侧标签页", MenuAction::CloseTabs(index, "right"))
                .icon(Icon::CHEVRON_RIGHT)
                .disabled(!has_tabs_to_right),
            MenuItem::new("全部关闭", MenuAction::CloseTabs(index, "all"))
                .icon(Icon::TRASH2)
                .disabled(!has_unpinned_tabs)
                .separated(),
        ];

        if let Some(path) = &path {
            items.push(
                MenuItem::new("复制路径", MenuAction::CopyPath(path.clone())).icon(Icon::COPY),
            );
            items.push(
                MenuItem::new("复制相对路径", MenuAction::CopyRelativePath(path.clone()))
                    .icon(Icon::COPY)
                    .separated(),
            );
        }

        items.push(
            MenuItem::new(
                if tab.pinned { "取消固定" } else { "固定" },
                MenuAction::PinTab(index),
            )
            .icon(Icon::PIN),
        );
        items.push(
            MenuItem::new(
                "向右拆分",
                MenuAction::SplitRight(path.clone().unwrap_or_default()),
            )
            .icon(Icon::CHEVRON_RIGHT)
            .disabled(tab.buffer().is_none())
            .separated(),
        );

        if let Some(path) = &path {
            items.push(
                MenuItem::new(
                    "在文件管理器中显示",
                    MenuAction::ShowInExplorer(path.clone()),
                )
                .icon(Icon::FOLDER_OPEN),
            );
            items.push(
                MenuItem::new("在文件树中显示", MenuAction::ShowInFileTree(path.clone()))
                    .icon(Icon::FILE_TEXT)
                    .separated(),
            );
            // Electron 没有模板中心，原生版必须把该已有能力保留在文件标签上。
            items.push(
                MenuItem::new("另存为模板", MenuAction::SaveAsTemplate(path.clone()))
                    .icon(Icon::LAYOUT_GRID)
                    .disabled(tab.buffer().is_none()),
            );
        }
        // 与 Electron 一致：非文件标签仍展示最后两项，但保持禁用；`.mcb`
        // 本身不可导出，因此不展示导出项。
        if !path.as_ref().is_some_and(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("mcb"))
        }) {
            items.push(
                MenuItem::new(
                    "导出",
                    MenuAction::ExportFile(path.clone().unwrap_or_default()),
                )
                .icon(Icon::FILE_DOWN)
                .disabled(!file_tab || tab.buffer().is_none()),
            );
        }
        items.push(
            MenuItem::new(
                "添加到AI助手",
                MenuAction::FileToAssistant(path.unwrap_or_default()),
            )
            .icon(Icon::SPARKLES)
            .disabled(!file_tab),
        );
        items
    }

    /// Electron 的「在文件树中显示」会切回工作区树并选中目标文件。原生树是
    /// 按需展开的，所以需要先展开从当前库根到文件的祖先目录。
    pub(super) fn show_in_file_tree(&mut self, path: &Path) {
        let Some(root) = self
            .shell
            .workspace()
            .map(|workspace| workspace.root.clone())
        else {
            return;
        };
        if !path.starts_with(&root) {
            self.show_global_notice("该文件不在当前工作区内");
            return;
        }
        self.shell.leave_favorites();
        self.shell.refresh_tree();
        // 当前打开的标签页可能属于与侧栏不同的资料库。
        let library = self.shell.workspace().and_then(|workspace| {
            workspace
                .libraries
                .iter()
                .enumerate()
                .filter(|(_, library)| path.starts_with(Path::new(&library.path)))
                .max_by_key(|(_, library)| Path::new(&library.path).components().count())
                .map(|(index, _)| index)
        });
        if let Some(library) = library {
            if self.shell.selected_library() != Some(library) {
                self.shell.select_library(library);
            }
        }
        let mut parents = path
            .ancestors()
            .skip(1)
            .take_while(|parent| parent.starts_with(&root) && parent != &root)
            .map(Path::to_path_buf)
            .collect::<Vec<_>>();
        parents.reverse();
        for parent in parents {
            self.shell.expand(&parent);
        }
        if let Some(row) = self
            .shell
            .rows()
            .iter()
            .position(|entry| entry.path == path)
        {
            self.shell.select(row);
            self.show_global_notice("已在文件树中显示");
        } else {
            self.show_global_notice("文件未包含在当前知识库范围内");
        }
        self.invalidate_main();
        self.sync_state();
    }

    /// 构建文件或文件夹的右键菜单，保留共享的项目顺序和分组。
    /// 当前上下文不可用的操作显示为禁用项。
    pub(super) fn row_menu_items(
        row: usize,
        path: &std::path::Path,
        is_dir: bool,
    ) -> Vec<MenuItem<MenuAction>> {
        let p = path.to_path_buf();
        let mut items = Vec::new();
        if is_dir {
            items.push(MenuItem::new("新建文件", MenuAction::NewFile(p.clone())).icon(Icon::FILE));
            items.push(
                MenuItem::new("新建文件夹", MenuAction::NewFolder(p.clone())).icon(Icon::FOLDER),
            );
            items.push(
                MenuItem::new("新增映射文件夹", MenuAction::NewMappedFolder(p.clone()))
                    .icon(Icon::FOLDER_COG),
            );
            items.push(
                MenuItem::new("新建链接", MenuAction::NewLink(path.to_path_buf())).icon(Icon::LINK),
            );
            items.push(
                MenuItem::new("新建多维表格", MenuAction::NewBase(path.to_path_buf()))
                    .icon(Icon::TABLE),
            );
            items.push(
                MenuItem::new("新建画布", MenuAction::NewCanvas(path.to_path_buf()))
                    .icon(Icon::LAYOUT_GRID),
            );
            items.push(
                MenuItem::new("导入文件", MenuAction::ImportFiles(p.clone())).icon(Icon::UPLOAD),
            );
        } else {
            items.push(
                MenuItem::new("新建子文档", MenuAction::NewSubdocument(p.clone()))
                    .icon(Icon::FILE_PLUS2),
            );
        }
        items.push(
            MenuItem::new("重命名", MenuAction::Rename(row))
                .icon(Icon::EDIT2)
                .separated(),
        );
        items.push(MenuItem::new("图标选择", MenuAction::FileIcon(p.clone())).icon(Icon::SMILE));
        if !is_dir {
            items.push(
                MenuItem::new("复制为副本", MenuAction::DuplicateFile(p.clone())).icon(Icon::COPY),
            );
            items.push(
                MenuItem::new("另存为模板", MenuAction::SaveAsTemplate(p.clone()))
                    .icon(Icon::LAYOUT_GRID),
            );
        }
        items.push(
            MenuItem::new("删除", MenuAction::Delete(row))
                .icon(Icon::TRASH2)
                .danger(),
        );
        items.push(
            MenuItem::new("复制路径", MenuAction::CopyPath(p.clone()))
                .icon(Icon::COPY)
                .separated(),
        );
        items.push(
            MenuItem::new(
                "复制为 Mochi 链接",
                MenuAction::CopyMochiUrl(p.clone(), is_dir),
            )
            .icon(Icon::LINK2),
        );
        items.push(
            MenuItem::new("在文件管理器中显示", MenuAction::ShowInExplorer(p.clone()))
                .icon(Icon::FOLDER_OPEN),
        );
        if !is_dir {
            items.push(
                MenuItem::new("添加到标签栏内", MenuAction::OpenTab(p.clone()))
                    .icon(Icon::PLUS)
                    .separated(),
            );
            if !p
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mcb"))
            {
                items.push(
                    MenuItem::new("导出", MenuAction::ExportFile(p.clone())).icon(Icon::FILE_DOWN),
                );
            }
            items.push(
                MenuItem::new("添加到AI助手", MenuAction::FileToAssistant(p.clone()))
                    .icon(Icon::SPARKLES),
            );
        }
        items.push(
            MenuItem::new("AI 权限设置", MenuAction::AiPermission(p.clone()))
                .icon(Icon::SHIELD)
                .separated(),
        );
        items.push(
            MenuItem::new("属性", MenuAction::Properties(p))
                .icon(Icon::INFO)
                .separated(),
        );
        items
    }

    /// 空白处的右键菜单。新建走对话框（TSX 的 `handleNewFileDialog`），不是内联编辑。
    pub(super) fn blank_menu_items() -> Vec<MenuItem<MenuAction>> {
        vec![
            MenuItem::new("新建文件", MenuAction::NewFile(PathBuf::new())).icon(Icon::FILE),
            MenuItem::new("新建多维表格", MenuAction::NewBase(PathBuf::new())).icon(Icon::TABLE),
            MenuItem::new("新建画布", MenuAction::NewCanvas(PathBuf::new()))
                .icon(Icon::LAYOUT_GRID),
            MenuItem::new("新建文件夹", MenuAction::NewFolder(PathBuf::new())).icon(Icon::FOLDER),
            MenuItem::new(
                "新增映射文件夹",
                MenuAction::NewMappedFolder(PathBuf::new()),
            )
            .icon(Icon::FOLDER_COG),
            MenuItem::new("导入文件", MenuAction::ImportFiles(PathBuf::new())).icon(Icon::UPLOAD),
            MenuItem::new("刷新", MenuAction::Refresh)
                .icon(Icon::ROTATE_CW)
                .separated(),
            MenuItem::new("排序方式", MenuAction::SortFiles).separated(),
        ]
    }
}
