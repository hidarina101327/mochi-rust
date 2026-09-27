//! 每个页签的导航只存位置，绝不复制访问过的每篇文档。
//! 当前缓冲仍归 Shell 所有；加载/保存失败时历史不前进。
use super::*;

const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Back,
    Forward,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Destination {
    File(PathBuf),
    Library(PathBuf),
    Settings(String, String),
    AgentConfig(String),
}

#[derive(Clone, Debug)]
struct Page {
    destination: Destination,
    scroll: f32,
    selection: Option<(usize, usize)>,
    source_mode: bool,
    chunk: Option<usize>,
}

#[derive(Default)]
pub(super) struct History {
    back: Vec<Page>,
    forward: Vec<Page>,
}

impl History {
    fn stack(&self, direction: Direction) -> &[Page] {
        match direction {
            Direction::Back => &self.back,
            Direction::Forward => &self.forward,
        }
    }

    pub(super) fn remap(&mut self, from: &Path, to: &Path, old_side: &Path, new_side: &Path) {
        for page in self.back.iter_mut().chain(self.forward.iter_mut()) {
            if let Destination::File(path) = &mut page.destination {
                if path == from {
                    *path = to.to_path_buf();
                } else if let Ok(rest) = path.strip_prefix(from) {
                    *path = to.join(rest);
                } else if let Ok(rest) = path.strip_prefix(old_side) {
                    *path = new_side.join(rest);
                }
            }
        }
    }
}

fn push(stack: &mut Vec<Page>, page: Page) {
    if stack
        .last()
        .is_some_and(|last| last.destination == page.destination)
    {
        *stack.last_mut().unwrap() = page;
    } else {
        stack.push(page);
        if stack.len() > HISTORY_LIMIT {
            stack.remove(0);
        }
    }
}

impl OpenTab {
    fn page(&self) -> Page {
        Page {
            destination: match &self.kind {
                TabKind::Library { path } => Destination::Library(path.clone()),
                TabKind::File { path, .. } | TabKind::Viewer { path, .. } => {
                    Destination::File(path.clone())
                }
                TabKind::Settings { tab, section } => {
                    Destination::Settings(tab.clone(), section.clone())
                }
                TabKind::AgentConfig { section } => Destination::AgentConfig(section.clone()),
            },
            scroll: self.scroll,
            selection: self.buffer().map(|buffer| {
                let (start, end) = buffer.selection();
                (
                    if buffer.cursor() == start { end } else { start },
                    buffer.cursor(),
                )
            }),
            source_mode: self.source_mode(),
            chunk: self.navigation_chunk,
        }
    }

    pub(super) fn remember_page(&mut self) {
        let page = self.page();
        push(&mut self.history.back, page);
        self.history.forward.clear();
    }
}

impl Shell {
    pub fn set_navigation_chunk(&mut self, chunk: Option<usize>) {
        if let Some(tab) = self.active_mut() {
            tab.navigation_chunk = chunk;
        }
    }
    pub fn navigation_chunk(&self) -> Option<usize> {
        self.active().and_then(|tab| tab.navigation_chunk)
    }

    /// 同一文档的分屏视图必须读取持有焦点的编辑缓冲，
    /// 而不是一个干净的、等待下一次保存边界的历史访问。
    pub fn text_tab_index(&self, path: &Path) -> Option<usize> {
        self.active_tab
            .filter(|&i| self.tabs[i].path() == Some(path) && self.tabs[i].buffer().is_some())
            .or_else(|| {
                self.tabs
                    .iter()
                    .position(|tab| tab.path() == Some(path) && tab.buffer().is_some())
            })
    }
    pub fn can_navigate(&self, direction: Direction) -> bool {
        self.active()
            .is_some_and(|tab| !tab.history.stack(direction).is_empty())
    }

    // 历史访问可能显示另一个页签已打开的文件。保存边界只更新干净的
    // 副本；绝不动别人的脏缓冲。打字、滚动和记录历史时都不做
    // 全文档克隆。
    pub(super) fn sync_clean_navigation_copies(&mut self, index: usize, content: &str) {
        let Some(path) = self.tabs[index].path().map(Path::to_path_buf) else {
            return;
        };
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if i == index || tab.dirty() || !tab.path().is_some_and(|p| same_native_path(p, &path))
            {
                continue;
            }
            if let Some(buffer) = tab.buffer_mut() {
                if buffer.composition().is_some() || buffer.text() == content {
                    continue;
                }
                let cursor = buffer.cursor();
                let (start, end) = buffer.selection();
                let anchor = if cursor == start { end } else { start };
                buffer.replace_range(0..buffer.text().len(), content);
                buffer.set_cursor(anchor, false);
                buffer.set_cursor(cursor, true);
                buffer.mark_saved();
            } else if let TabKind::Viewer {
                content: crate::ui::viewer::Content::Base(state),
                ..
            } = &mut tab.kind
            {
                if let Ok(next) = crate::ui::base_view::State::parse(content.to_owned()) {
                    *state = next;
                }
            }
        }
    }

    pub(super) fn save_shared_page_before_switch(&mut self) -> bool {
        let Some(i) = self.active_tab else {
            return true;
        };
        let Some(path) = self.tabs[i].path() else {
            return true;
        };
        let shared = self.tabs.iter().enumerate().any(|(other, tab)| {
            other != i && tab.path().is_some_and(|p| same_native_path(p, path))
        });
        !shared || self.save_navigation_page(i)
    }

    fn save_navigation_page(&mut self, index: usize) -> bool {
        if self.tabs[index]
            .buffer()
            .is_some_and(|buffer| buffer.composition().is_some())
        {
            self.status = "请先完成输入，再切换页面".into();
            return false;
        }
        !self.tabs[index].dirty() || self.save_tab(index)
    }

    fn load_navigation_page(&mut self, page: &mut Page) -> Option<TabKind> {
        match &mut page.destination {
            Destination::Library(path) => Some(TabKind::Library { path: path.clone() }),
            Destination::File(path) => {
                // 不能把一个脏缓冲分叉成两份各自可编辑的副本。
                let existing = self
                    .tabs
                    .iter()
                    .position(|tab| tab.path().is_some_and(|p| same_native_path(p, path)));
                if let Some(i) = existing {
                    if !self.save_navigation_page(i) {
                        return None;
                    }
                    if let Some(saved_path) = self.tabs[i].path() {
                        *path = saved_path.to_path_buf();
                    }
                }
                let kind = crate::ui::viewer::detect(path);
                if kind != crate::ui::viewer::Kind::Text {
                    if !path.is_file() {
                        return None;
                    }
                    return Some(TabKind::Viewer {
                        path: path.clone(),
                        content: crate::ui::viewer::load(path, &kind),
                    });
                }
                let content = match std::fs::read_to_string(&*path) {
                    Ok(content) => content,
                    Err(error) => {
                        self.status = format!("无法打开历史页面：{error}");
                        return None;
                    }
                };
                let mut buffer = crate::ui::editor::TextBuffer::new(content);
                if let Some((anchor, cursor)) = page.selection {
                    buffer.set_cursor(anchor, false);
                    buffer.set_cursor(cursor, true);
                }
                Some(TabKind::File {
                    path: path.clone(),
                    buffer,
                    source_mode: page.source_mode,
                })
            }
            Destination::Settings(tab, section) => Some(TabKind::Settings {
                tab: tab.clone(),
                section: section.clone(),
            }),
            Destination::AgentConfig(section) => Some(TabKind::AgentConfig {
                section: section.clone(),
            }),
        }
    }

    fn replace_navigation_page(&mut self, index: usize, page: &Page, kind: TabKind) {
        let tab = &mut self.tabs[index];
        let keep_for_split = tab.path().is_some_and(|path| {
            self.protected_view_path
                .as_deref()
                .is_some_and(|p| same_native_path(p, path))
        });
        let title = match &page.destination {
            Destination::File(path) => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Destination::Library(path) => path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Destination::Settings(..) => "设置".into(),
            Destination::AgentConfig(..) => "Agent 配置".into(),
        };
        let previous = OpenTab {
            kind: std::mem::replace(&mut tab.kind, kind),
            title: std::mem::replace(&mut tab.title, title),
            scroll: std::mem::replace(&mut tab.scroll, page.scroll),
            pinned: false,
            history: History::default(),
            navigation_chunk: std::mem::replace(&mut tab.navigation_chunk, page.chunk),
        };
        if keep_for_split && !self.tabs.iter().any(|tab| tab.path() == previous.path()) {
            self.tabs.push(previous);
        }
        self.refresh_visible_files(None);
    }

    /// 浏览器式的页签内链接导航。显式的「新页签打开」继续走
    /// open_file_with_mode；历史从不参考新页签偏好。
    pub fn navigate_file(&mut self, path: &Path) -> bool {
        let Some(index) = self
            .active_tab
            .filter(|&i| self.tabs[i].path().is_some() && !self.tabs[i].pinned)
        else {
            return self.open_file_with_mode(path, true);
        };
        if self.tabs[index]
            .path()
            .is_some_and(|p| same_native_path(p, path))
        {
            return true;
        }
        let mut page = Page {
            destination: Destination::File(path.to_path_buf()),
            scroll: 0.0,
            selection: None,
            source_mode: false,
            chunk: None,
        };
        if !self.save_navigation_page(index) {
            return false;
        }
        let Some(kind) = self.load_navigation_page(&mut page) else {
            return false;
        };
        self.tabs[index].remember_page();
        self.replace_navigation_page(index, &page, kind);
        true
    }

    pub fn navigate(&mut self, direction: Direction) -> bool {
        let Some(index) = self.active_tab else {
            return false;
        };
        // 跳过已删除的页面，且不在绘制路径里碰文件系统。
        let candidate = self.tabs[index]
            .history
            .stack(direction)
            .iter()
            .enumerate()
            .rev()
            .find(|(_, page)| match &page.destination {
                Destination::File(path) => path.is_file(),
                _ => true,
            })
            .map(|(at, page)| (at, page.clone()));
        let Some((at, mut page)) = candidate else {
            match direction {
                Direction::Back => self.tabs[index].history.back.clear(),
                Direction::Forward => self.tabs[index].history.forward.clear(),
            }
            self.status = "没有可访问的历史页面（文件可能已被移动或删除）".into();
            return false;
        };
        if !self.save_navigation_page(index) {
            return false;
        }
        // 保存可能把 .md 迁移成 .mc，并重映射两条历史栈。
        page.destination = self.tabs[index].history.stack(direction)[at]
            .destination
            .clone();
        let Some(kind) = self.load_navigation_page(&mut page) else {
            return false;
        };
        let previous = self.tabs[index].page();
        let history = &mut self.tabs[index].history;
        match direction {
            Direction::Back => {
                history.back.truncate(at);
                push(&mut history.forward, previous);
            }
            Direction::Forward => {
                history.forward.truncate(at);
                push(&mut history.back, previous);
            }
        }
        self.replace_navigation_page(index, &page, kind);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        shell: Shell,
    }
    impl Fixture {
        fn new() -> Self {
            crate::ui::settings_values::reset();
            let root = std::env::temp_dir().join(format!(
                "mochi-tab-history-{}",
                mochi_core::paths::random_base36(12)
            ));
            std::fs::create_dir_all(&root).unwrap();
            for name in ["a.mc", "b.mc", "c.mc", "d.mc"] {
                std::fs::write(
                    root.join(name),
                    format!("# {name}\n\n中文 reading position\n"),
                )
                .unwrap();
            }
            Self {
                root,
                shell: Shell::new(),
            }
        }
        fn path(&self, name: &str) -> PathBuf {
            self.root.join(name)
        }
        fn open(&mut self, name: &str) {
            assert!(self.shell.open_file_with_mode(&self.path(name), true));
        }
        fn visit(&mut self, name: &str) {
            assert!(self.shell.navigate_file(&self.path(name)));
        }
        fn at(&self, name: &str) {
            assert_eq!(
                self.shell.active().unwrap().path(),
                Some(self.path(name).as_path())
            );
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn history_is_independent_per_tab_and_survives_tab_removal() {
        let mut f = Fixture::new();
        f.open("a.mc");
        f.visit("b.mc");
        f.open("c.mc");
        f.visit("d.mc");
        f.shell.select_tab(0);
        assert!(f.shell.navigate(Direction::Back));
        f.at("a.mc");
        assert_eq!(f.shell.tabs.len(), 2);
        f.shell.select_tab(1);
        assert!(f.shell.navigate(Direction::Back));
        f.at("c.mc");
        f.shell.close_tab(0);
        assert!(f.shell.navigate(Direction::Forward));
        f.at("d.mc");
        assert_eq!(f.shell.active_tab(), Some(0));
    }

    #[test]
    fn back_forward_restores_scroll_selection_mode_and_content_chunk() {
        let mut f = Fixture::new();
        f.open("a.mc");
        f.shell.set_active_scroll(173.5);
        f.shell.set_navigation_chunk(Some(48000));
        if let TabKind::File {
            buffer,
            source_mode,
            ..
        } = &mut f.shell.active_mut().unwrap().kind
        {
            *source_mode = true;
            buffer.set_cursor(2, false);
            buffer.set_cursor(5, true);
        }
        f.visit("b.mc");
        f.shell.set_active_scroll(48.0);
        assert!(f.shell.navigate(Direction::Back));
        f.at("a.mc");
        assert_eq!(f.shell.active_scroll(), 173.5);
        assert_eq!(f.shell.navigation_chunk(), Some(48000));
        assert!(f.shell.active().unwrap().source_mode());
        assert_eq!(
            f.shell.active().unwrap().buffer().unwrap().selection(),
            (2, 5)
        );
        assert!(f.shell.navigate(Direction::Forward));
        f.at("b.mc");
        assert_eq!(f.shell.active_scroll(), 48.0);
        assert!(!f.shell.can_navigate(Direction::Forward));
    }

    #[test]
    fn revisiting_same_page_is_noop_and_new_branch_clears_forward_only_here() {
        let mut f = Fixture::new();
        f.open("a.mc");
        f.visit("b.mc");
        assert!(f.shell.navigate(Direction::Back));
        f.visit("a.mc");
        assert!(f.shell.can_navigate(Direction::Forward));
        f.visit("c.mc");
        assert!(!f.shell.can_navigate(Direction::Forward));
        assert!(f.shell.navigate(Direction::Back));
        f.at("a.mc");
        assert!(!f.shell.can_navigate(Direction::Back));
    }

    #[test]
    fn save_conflict_and_failed_load_leave_current_page_and_history_intact() {
        let mut f = Fixture::new();
        f.open("a.mc");
        f.visit("b.mc");
        f.shell.active_buffer_mut().unwrap().insert("local");
        std::fs::write(f.path("b.mc"), "external").unwrap();
        assert!(!f.shell.navigate(Direction::Back));
        f.at("b.mc");
        assert!(f.shell.active().unwrap().dirty());
        assert!(f.shell.can_navigate(Direction::Back));
        assert!(!f.shell.can_navigate(Direction::Forward));
        assert!(f.shell.status().contains("磁盘文件已变化"));
        assert_eq!(std::fs::read_to_string(f.path("b.mc")).unwrap(), "external");
        let mut f = Fixture::new();
        f.open("a.mc");
        f.visit("b.mc");
        assert!(!f.shell.navigate_file(&f.path("missing.mc")));
        f.at("b.mc");
        assert_eq!(f.shell.active().unwrap().history.back.len(), 1);
    }

    #[test]
    fn dirty_page_is_saved_before_back_and_deleted_pages_are_skipped() {
        let mut f = Fixture::new();
        f.open("a.mc");
        f.visit("b.mc");
        f.visit("c.mc");
        f.shell.active_buffer_mut().unwrap().insert("saved");
        std::fs::remove_file(f.path("b.mc")).unwrap();
        assert!(f.shell.navigate(Direction::Back));
        f.at("a.mc");
        assert!(std::fs::read_to_string(f.path("c.mc"))
            .unwrap()
            .contains("saved"));
        assert!(f.shell.navigate(Direction::Forward));
        f.at("c.mc");
        std::fs::remove_file(f.path("a.mc")).unwrap();
        assert!(!f.shell.navigate(Direction::Back));
        f.at("c.mc");
        assert!(!f.shell.can_navigate(Direction::Back));
    }

    #[test]
    fn already_open_history_target_stays_in_same_tab_and_clean_copies_sync_on_save() {
        let mut f = Fixture::new();
        f.open("a.mc");
        f.visit("b.mc");
        f.open("a.mc");
        f.shell.active_buffer_mut().unwrap().insert("first");
        f.shell.select_tab(0);
        assert!(f.shell.navigate(Direction::Back));
        f.at("a.mc");
        assert_eq!(f.shell.active_tab(), Some(0));
        assert_eq!(f.shell.tabs.len(), 2);
        assert!(f
            .shell
            .active()
            .unwrap()
            .buffer()
            .unwrap()
            .text()
            .contains("first"));
        f.shell.active_buffer_mut().unwrap().insert("second");
        f.shell.select_tab(1);
        assert_eq!(f.shell.active_tab(), Some(1));
        assert!(f
            .shell
            .active()
            .unwrap()
            .buffer()
            .unwrap()
            .text()
            .contains("second"));
        assert!(!f.shell.active().unwrap().dirty());
        f.shell.active_buffer_mut().unwrap().insert("third");
        f.shell.select_tab(0);
        assert!(f
            .shell
            .active()
            .unwrap()
            .buffer()
            .unwrap()
            .text()
            .contains("third"));
        assert!(f.shell.navigate(Direction::Forward));
        f.at("b.mc");
        assert!(f.shell.tabs[1].history.back.is_empty());
    }

    #[test]
    fn a_history_visit_cannot_switch_away_from_a_shared_unsaved_conflict() {
        let mut f = Fixture::new();
        f.open("a.mc");
        f.visit("b.mc");
        f.open("a.mc");
        f.shell.select_tab(0);
        assert!(f.shell.navigate(Direction::Back));
        f.shell.active_buffer_mut().unwrap().insert("local");
        std::fs::write(f.path("a.mc"), "disk changed").unwrap();
        f.shell.select_tab(1);
        assert_eq!(f.shell.active_tab(), Some(0));
        assert!(!f.shell.open_file_with_mode(&f.path("c.mc"), true));
        assert_eq!(f.shell.tabs.len(), 2);
        assert!(f
            .shell
            .active()
            .unwrap()
            .buffer()
            .unwrap()
            .text()
            .contains("local"));
    }

    #[test]
    fn file_and_parent_rename_remap_history_without_loading_old_files() {
        let mut f = Fixture::new();
        f.open("a.mc");
        f.visit("b.mc");
        let to = f.path("renamed.mc");
        std::fs::rename(f.path("a.mc"), &to).unwrap();
        f.shell.accept_external_rename(&f.path("a.mc"), &to);
        assert!(f.shell.navigate(Direction::Back));
        f.at("renamed.mc");
        assert!(f.shell.navigate(Direction::Forward));
        f.at("b.mc");
    }

    #[test]
    fn split_owned_page_keeps_its_buffer_when_active_tab_navigates() {
        let mut f = Fixture::new();
        f.open("a.mc");
        f.visit("b.mc");
        f.shell
            .active_buffer_mut()
            .unwrap()
            .insert("split contents");
        f.shell.set_protected_view_path(Some(f.path("b.mc")));
        assert!(f.shell.navigate(Direction::Back));
        f.at("a.mc");
        assert_eq!(f.shell.tabs.len(), 2);
        assert!(f.shell.tabs[1]
            .buffer()
            .unwrap()
            .text()
            .contains("split contents"));
        assert_eq!(f.shell.tabs[1].path(), Some(f.path("b.mc").as_path()));
    }

    #[test]
    fn special_tab_sections_have_history_without_duplicate_noop_entries() {
        let mut f = Fixture::new();
        f.shell.open_special(
            TabKind::Settings {
                tab: "general".into(),
                section: "".into(),
            },
            "设置",
        );
        f.shell.open_special(
            TabKind::Settings {
                tab: "ai".into(),
                section: "providers".into(),
            },
            "设置",
        );
        f.shell.open_special(
            TabKind::Settings {
                tab: "ai".into(),
                section: "providers".into(),
            },
            "设置",
        );
        assert!(f.shell.navigate(Direction::Back));
        assert!(
            matches!(&f.shell.active().unwrap().kind, TabKind::Settings { tab, .. } if tab == "general")
        );
        assert!(!f.shell.can_navigate(Direction::Back));
        assert!(f.shell.navigate(Direction::Forward));
    }

    #[test]
    fn history_is_bounded_metadata_not_document_buffers() {
        let mut history = History::default();
        for i in 0..250 {
            push(
                &mut history.back,
                Page {
                    destination: Destination::File(PathBuf::from(format!("{i}.mc"))),
                    scroll: 0.0,
                    selection: None,
                    source_mode: false,
                    chunk: None,
                },
            );
        }
        assert_eq!(history.back.len(), HISTORY_LIMIT);
        assert_eq!(
            history.back[0].destination,
            Destination::File(PathBuf::from("150.mc"))
        );
    }
}
