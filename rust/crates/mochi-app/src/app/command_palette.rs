//! 管理命令面板的开关、选择和执行，并提供常用页面入口。
use super::*;

impl App {
    /// 当前标签是不是一个文件（渲染视图或源码模式）。
    pub(super) fn file_open(&self) -> bool {
        matches!(self.content(), MainContent::Document | MainContent::Source)
            && self.shell.active().and_then(|t| t.buffer()).is_some()
    }

    pub(super) fn base_viewer_open(&self) -> bool {
        matches!(self.viewer_tab(), Some((_, viewer::Content::Base(_))))
    }

    /// 打开（已开着同一模式则关掉；开着另一模式则切过去）。
    pub(super) fn toggle_command(&mut self, mode: command::Mode) {
        if self.command.as_ref().map(|c| c.mode) == Some(mode) {
            self.close_command();
            return;
        }
        let mut state = command::State::new(mode);
        if mode == command::Mode::Files {
            if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                state.files = command::collect_files(&root);
            }
            // TSX：`tabs.slice(0, 10).map(tab => tab.path)`——按标签顺序，不去重
            state.recent = self
                .shell
                .tabs()
                .iter()
                .filter_map(|t| t.path().map(Path::to_path_buf))
                .collect();
        }
        self.command = Some(state);
        self.focus = Focus::Command;
    }

    pub(super) fn close_command(&mut self) {
        self.command = None;
        if self.focus == Focus::Command {
            self.focus = Focus::Main;
        }
    }

    /// 执行选中项（Enter / 点击）。先关面板再执行——有的动作自己会开别的覆盖层。
    pub(super) fn run_command_selection(&mut self, index: usize) {
        let Some(c) = self.command.as_ref() else {
            return;
        };
        match c.mode {
            command::Mode::Commands => {
                let action = command::filter_commands(c.query.text())
                    .get(index)
                    .map(|i| command::COMMANDS[*i].action);
                self.close_command();
                if let Some(a) = action {
                    self.run_command_action(a);
                }
            }
            command::Mode::Files => {
                let path = command::filter_files(&c.files, &c.recent, c.query.text())
                    .get(index)
                    .map(|f| f.path.clone());
                self.close_command();
                if let Some(p) = path {
                    if p.is_file() {
                        self.open_file_from_ui(&p);
                        self.invalidate_main();
                        self.sync_state();
                    } else {
                        self.state.status_text = format!("文件不存在：{}", p.display());
                    }
                }
            }
        }
    }

    /// 执行命令面板动作，将文件、面板和编辑器操作分派给对应处理入口。
    pub(super) fn run_command_action(&mut self, action: CommandAction) {
        match action {
            CommandAction::NewFile => {
                if let Some(root) = self.shell.tree_root() {
                    self.open_template_picker(root);
                }
            }
            CommandAction::NewFolder => {
                if let Some(root) = self.shell.tree_root() {
                    self.open_create_dialog(root, true);
                }
            }
            CommandAction::NewLinkFile => {
                if let Some(root) = self.shell.tree_root() {
                    self.open_link_dialog(root);
                }
            }
            CommandAction::NewBase => {
                if let Some(root) = self.shell.tree_root() {
                    self.open_base_dialog(root);
                }
            }
            CommandAction::ToggleSidebar => {
                self.state.sidebar_visible = !self.state.sidebar_visible;
                self.invalidate_main();
            }
            CommandAction::ToggleSourceMode => self.toggle_source_mode(),
            CommandAction::QuickOpen => self.toggle_command(command::Mode::Files),
            CommandAction::GlobalSearch => self.open_search(),
            CommandAction::ToggleAiPanel => self.toggle_ai_panel(),
            CommandAction::VersionHistory => {
                self.state.ai_panel_open = true;
                self.set_right_panel(RightPanel::VersionHistory);
                self.invalidate_main();
            }
            CommandAction::Settings => self.open_settings("general"),
            CommandAction::SplitRight => {
                if let Some(path) = self.active_file_path() {
                    self.split_to_right(path);
                }
            }
            CommandAction::CloseSplit => {
                self.close_split_view();
            }
        }
    }

    pub(super) fn on_command_click(&mut self, x: f32, y: f32) {
        match self.command_layout.hit(x, y) {
            command::Hit::Backdrop => self.close_command(),
            command::Hit::Row(i) => self.run_command_selection(i),
            command::Hit::Input => {
                let left = self.command_layout.input.left + 12.0;
                if let Some(c) = self.command.as_mut() {
                    c.query.click(x - left, false);
                }
            }
            command::Hit::Inside => {}
        }
    }

    /// 面板里的按键。TSX 在 window 上监听：上下 / Enter / Esc；其余给输入框。
    pub(super) fn on_command_key(&mut self, key: u16, shift: bool, ctrl: bool) -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP};
        let Some(c) = self.command.as_mut() else {
            return false;
        };
        match key {
            k if k == VK_ESCAPE.0 => {
                self.close_command();
                true
            }
            k if k == VK_RETURN.0 => {
                let i = c.selected;
                self.run_command_selection(i);
                true
            }
            k if k == VK_DOWN.0 || k == VK_UP.0 => {
                c.move_selection(if k == VK_DOWN.0 { 1 } else { -1 });
                self.scroll_command_selection_into_view();
                true
            }
            _ => match c.query.key(key, shift, ctrl) {
                FieldKey::Edited => {
                    c.query_changed();
                    true
                }
                FieldKey::Ignored => false,
                _ => true,
            },
        }
    }

    /// `scrollIntoView({ block: 'nearest' })`。
    pub(super) fn scroll_command_selection_into_view(&mut self) {
        let Some(c) = self.command.as_mut() else {
            return;
        };
        let lay = &self.command_layout;
        let Some(r) = lay.item_rect(c.selected) else {
            return;
        };
        // 行矩形带着当前滚动偏移；换算成内容坐标
        let top = r.top - lay.list.top + c.scroll;
        let bottom = r.bottom - lay.list.top + c.scroll;
        let view_h = lay.list.height();
        if top < c.scroll {
            c.scroll = top;
        } else if bottom > c.scroll + view_h {
            c.scroll = bottom - view_h;
        }
        c.scroll = c.scroll.max(0.0);
    }
}
