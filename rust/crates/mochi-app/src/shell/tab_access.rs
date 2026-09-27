//! 提供当前标签页和活动标签页的访问接口。
use super::*;

impl Shell {
    pub fn open_library_landing(&mut self, index: usize) {
        let Some(library) = self
            .workspace
            .as_ref()
            .and_then(|w| w.libraries.get(index))
            .cloned()
        else {
            return;
        };
        self.select_library(index);
        self.open_special(
            TabKind::Library {
                path: PathBuf::from(&library.path),
            },
            &library.name,
        );
        self.selected = None;
    }

    pub fn tabs_mut(&mut self) -> &mut [OpenTab] {
        &mut self.tabs
    }

    pub fn tabs(&self) -> &[OpenTab] {
        &self.tabs
    }

    pub fn active_tab(&self) -> Option<usize> {
        self.active_tab
    }

    pub fn active(&self) -> Option<&OpenTab> {
        self.tabs.get(self.active_tab?)
    }

    pub fn active_mut(&mut self) -> Option<&mut OpenTab> {
        let i = self.active_tab?;
        self.tabs.get_mut(i)
    }

    /// 当前标签的可编辑缓冲区（非文件标签为 `None`）。
    pub fn active_buffer_mut(&mut self) -> Option<&mut crate::ui::editor::TextBuffer> {
        self.active_mut()?.buffer_mut()
    }

    /// 打开（或切到）一个非文件标签。同类标签只开一个——设置页开两份没有意义。
    pub fn open_special(&mut self, kind: TabKind, title: &str) {
        if !self.save_shared_page_before_switch() {
            return;
        }
        self.leave_favorites();
        let same = |t: &OpenTab| {
            matches!(
                (&t.kind, &kind),
                (TabKind::Settings { .. }, TabKind::Settings { .. })
                    | (TabKind::AgentConfig { .. }, TabKind::AgentConfig { .. })
            )
        };
        if let Some(i) = self.tabs.iter().position(same) {
            // 切过去并更新分区（点了不同的入口）
            let changed = match (&self.tabs[i].kind, &kind) {
                (
                    TabKind::Settings { tab: a, section: b },
                    TabKind::Settings { tab: c, section: d },
                ) => a != c || b != d,
                (TabKind::AgentConfig { section: a }, TabKind::AgentConfig { section: b }) => {
                    a != b
                }
                _ => true,
            };
            if changed {
                self.tabs[i].remember_page();
            }
            self.tabs[i].kind = kind;
            self.active_tab = Some(i);
            return;
        }
        self.tabs.push(OpenTab {
            kind,
            title: title.to_owned(),
            pinned: false,
            scroll: 0.0,
            history: navigation::History::default(),
            navigation_chunk: None,
        });
        self.active_tab = Some(self.tabs.len() - 1);
    }

    pub fn active_scroll(&self) -> f32 {
        self.active_tab
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.scroll)
            .unwrap_or(0.0)
    }

    pub fn set_active_scroll(&mut self, scroll: f32) {
        if let Some(tab) = self.active_tab.and_then(|i| self.tabs.get_mut(i)) {
            tab.scroll = scroll;
        }
    }

    /// 打开一个文件。已经开着就切过去，不重复开——
    /// 双击同一个文件打开两个标签页，是一个很让人困扰的问题。
    ///
    /// 读盘失败（权限、正被独占、二进制文件）返回 `false` 并且**不建标签**：
    /// 开一个空白标签会让人以为文件真的是空的。
    pub fn open_file(&mut self, path: &Path) -> bool {
        self.open_file_with_mode(path, false)
    }

    /// 使用修饰键点击时会绕过替换操作，但不会修改已保存的偏好设置。
    pub fn open_file_with_mode(&mut self, path: &Path, force_append: bool) -> bool {
        if !self.save_shared_page_before_switch() {
            return false;
        }
        if let Some(index) = self
            .tabs
            .iter()
            .position(|t| t.path().is_some_and(|open| same_native_path(open, path)))
        {
            self.select_tab(index);
            self.refresh_visible_files(None);
            return self.active_tab == Some(index);
        }
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        // 不是 Markdown / 纯文本：交给查看器（`FileViewer.tsx` 的分派）
        let kind = crate::ui::viewer::detect(path);
        if kind != crate::ui::viewer::Kind::Text {
            if !path.is_file() {
                return false;
            }
            let content = crate::ui::viewer::load(path, &kind);
            self.insert_file_tab(
                OpenTab {
                    kind: TabKind::Viewer {
                        path: path.to_path_buf(),
                        content,
                    },
                    title,
                    pinned: false,
                    scroll: 0.0,
                    history: navigation::History::default(),
                    navigation_chunk: None,
                },
                force_append,
            );
            return true;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            return false;
        };
        self.insert_file_tab(
            OpenTab {
                kind: TabKind::File {
                    path: path.to_path_buf(),
                    buffer: crate::ui::editor::TextBuffer::new(content),
                    source_mode: false,
                },
                title,
                pinned: false,
                scroll: 0.0,
                history: navigation::History::default(),
                navigation_chunk: None,
            },
            force_append,
        );
        true
    }

    pub fn set_protected_view_path(&mut self, path: Option<PathBuf>) {
        self.protected_view_path = path;
    }

    pub(super) fn insert_file_tab(&mut self, mut tab: OpenTab, force_append: bool) {
        use crate::ui::settings_values::text;
        if !force_append && text("tabs.openFileBehavior", "new-tab") == "replace" {
            if let Some(i) = self.active_tab.filter(|i| {
                self.tabs.get(*i).is_some_and(|t| {
                    !t.pinned
                        && !t.dirty()
                        && t.path().is_some()
                        && t.path() != self.protected_view_path.as_deref()
                })
            }) {
                self.tabs[i].remember_page();
                tab.history = std::mem::take(&mut self.tabs[i].history);
                self.tabs[i] = tab;
                return;
            }
        }
        let i = if text("tabs.newTabPosition", "end") == "after-active" {
            self.active_tab
                .map(|i| i + 1)
                .unwrap_or(self.tabs.len())
                .min(self.tabs.len())
        } else {
            self.tabs.len()
        };
        self.tabs.insert(i, tab);
        self.active_tab = Some(i);
    }

    pub fn select_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            if self.active_tab != Some(index) && !self.save_shared_page_before_switch() {
                return;
            }
            if self.tabs[index].path().is_none() {
                self.leave_favorites();
            }
            let library = match &self.tabs[index].kind {
                TabKind::Library { path } => self.workspace.as_ref().and_then(|w| {
                    w.libraries
                        .iter()
                        .position(|lib| Path::new(&lib.path) == path)
                }),
                _ => None,
            };
            if let Some(library) = library {
                self.select_library(library);
            }
            self.active_tab = Some(index);
            self.refresh_visible_files(None);
        }
    }

    /// 关掉一个标签。激活的那个被关掉时接管**左边**的邻居——
    /// 与 VS Code 一致；接管右边会让连关几个标签时焦点一路往右跑。
    pub fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if self.tabs[index].dirty() && !self.save_tab(index) {
            return;
        }
        self.discard_tab(index);
    }

    pub(super) fn discard_tab(&mut self, index: usize) {
        self.tabs.remove(index);
        self.active_tab = match self.active_tab {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => Some(index.saturating_sub(1)),
            other => other,
        };
        if self.favorites_selected && self.active().is_some_and(|tab| tab.path().is_none()) {
            self.active_tab = self.tabs.iter().rposition(|tab| tab.path().is_some());
        }
    }
}
