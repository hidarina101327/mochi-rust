//! 管理 Agent 配置的加载、选择、编辑和重载。
use super::*;

#[cfg(test)]
#[path = "agent_configuration_tests.rs"]
mod tests;

#[path = "agent_updates.rs"]
mod updates;

impl App {
    /// Agent 配置独立工作区视图当前显示的分区。
    pub(super) fn agent_config_section(&self) -> Option<usize> {
        (self.state.view == WorkspaceView::AgentConfig).then_some(self.agent.section)
    }

    pub(super) fn ensure_agent_config_loaded(&mut self) {
        if self.agent.loaded {
            return;
        }
        self.reload_agent_config();
    }

    pub(super) fn reload_agent_config(&mut self) {
        self.agent.loaded = true;
        self.agent.error = None;
        match self.shell.workspace().map(|ws| ws.root.clone()) {
            Some(root) => self.agent.data = agent_config::Data::load(&root),
            None => self.agent.data = agent_config::Data::default(),
        }
        self.check_agent_updates(false);
    }

    pub(super) fn set_agent_config_section(&mut self, index: usize) {
        self.agent.update_review = None;
        self.park_agent_source();
        self.agent.section = index.min(agent_config::SECTIONS.len().saturating_sub(1));
        self.agent.scroll = 0.0;
        self.focus = Focus::Main;
        self.invalidate_main();
    }

    fn park_agent_source(&mut self) {
        if let Some(editor) = self
            .agent
            .source_editor
            .take()
            .filter(|editor| editor.dirty)
        {
            self.agent.source_drafts.insert(editor.path.clone(), editor);
        }
    }

    /// 在 AI 定义独享视图内编辑源文件，绝不创建知识库标签页。
    pub(super) fn open_agent_source(&mut self, path: &str) {
        self.agent.update_review = None;
        let path = PathBuf::from(path);
        if self
            .agent
            .source_editor
            .as_ref()
            .is_some_and(|editor| editor.path == path)
        {
            self.focus = Focus::AgentSource;
            return;
        }
        if let Some(editor) = self.agent.source_drafts.remove(&path) {
            self.park_agent_source();
            self.agent.source_editor = Some(editor);
            self.state.view = WorkspaceView::AgentConfig;
            self.focus = Focus::AgentSource;
            self.invalidate_main();
            return;
        }
        if !path.is_file() {
            self.state.status_text = format!("找不到源文件：{}", path.display());
            return;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            self.state.status_text = format!("无法读取源文件：{}", path.display());
            return;
        };
        self.park_agent_source();
        let mut field = TextField::new("配置源码").with_text(&text);
        field.buffer.set_cursor(0, false);
        self.agent.source_editor = Some(AgentSourceEditor {
            path,
            field,
            area: Rect::ZERO,
            dirty: false,
        });
        self.state.view = WorkspaceView::AgentConfig;
        self.focus = Focus::AgentSource;
        self.invalidate_main();
    }

    pub(super) fn save_agent_source(&mut self) {
        let Some(editor) = self.agent.source_editor.as_mut() else {
            return;
        };
        match std::fs::write(&editor.path, editor.field.text()) {
            Ok(()) => {
                editor.dirty = false;
                self.state.status_text = format!("已保存 AI 定义：{}", editor.path.display());
                self.reload_agent_config();
            }
            Err(error) => self.agent.error = Some(format!("保存 AI 定义失败：{error}")),
        }
    }

    /// 「新建」：按分区写模板文件（重名加 `-2`、`-3`……后缀），然后打开它。
    pub(super) fn create_agent_template(&mut self, section: usize) {
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            return;
        };
        let base = mochi_core::ai::agent_config::AgentConfigService::new(&root).root();
        let (rel, body) = agent_config::template(section);
        // `{n}` 在目录名或文件名上：第一次为空，之后 -2、-3……
        let mut chosen = None;
        for i in 1..1000 {
            let suffix = if i == 1 {
                String::new()
            } else {
                format!("-{i}")
            };
            let candidate = base.join(rel.replace("{n}", &suffix));
            // 目录型模板（Skills/x/SKILL.md）按目录查重，文件型按文件查重
            let probe = if rel.matches('/').count() > 1 {
                candidate
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| candidate.clone())
            } else {
                candidate.clone()
            };
            if !probe.exists() {
                chosen = Some(candidate);
                break;
            }
        }
        let Some(path) = chosen else { return };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                self.agent.error = Some(format!("创建目录失败：{e}"));
                return;
            }
        }
        if let Err(e) = std::fs::write(&path, body) {
            self.agent.error = Some(format!("写入模板失败：{e}"));
            return;
        }
        self.reload_agent_config();
        self.open_agent_source(&path.to_string_lossy());
    }

    pub(super) fn on_agent_config_click(&mut self, x: f32, y: f32) {
        // 打开源码编辑器时，固定侧栏仍可正常使用。
        if let Some(hit) = self.agent.nav_layout.hit(x, y) {
            match hit {
                agent_config::NavHit::Back => {
                    self.state.view = WorkspaceView::MochiAi;
                    self.focus = Focus::Main;
                    self.invalidate_main();
                }
                agent_config::NavHit::Section(i) => self.set_agent_config_section(i),
                agent_config::NavHit::Item(si, ii) => {
                    if let Some(card) = self.agent.data.cards(si).get(ii) {
                        let path = card.source_path.clone();
                        if card.read_only {
                            platform::show_in_explorer(Path::new(&path));
                        } else {
                            self.agent.section = si;
                            self.open_agent_source(&path);
                        }
                    }
                }
            }
            return;
        }
        if self.agent.update_review.is_some() {
            if let Some(hit) = self.agent.layout.hit(x, y) {
                self.on_agent_update_action(hit);
            }
            return;
        }
        if self.agent.update_banner.contains(x, y) && !self.agent.updates.is_empty() {
            let index = self
                .agent
                .updates
                .iter()
                .position(|u| !u.skipped)
                .unwrap_or(0);
            self.open_agent_update(index);
            return;
        }
        if let Some(editor) = self.agent.source_editor.as_mut() {
            let area = editor.area;
            let header_top = area.top - 64.0;
            let header_right = area.right + 16.0;
            let back = Rect::new(
                header_right - 174.0,
                header_top + 10.0,
                header_right - 98.0,
                header_top + 42.0,
            );
            let save = Rect::new(
                header_right - 86.0,
                header_top + 10.0,
                header_right - 20.0,
                header_top + 42.0,
            );
            if back.contains(x, y) {
                self.park_agent_source();
                self.focus = Focus::Main;
            } else if save.contains(x, y) {
                self.save_agent_source();
            } else if area.contains(x, y) {
                editor.field.multiline_click(area, x, y);
                self.focus = Focus::AgentSource;
            }
            return;
        }
        let section = self.agent_config_section().unwrap_or(0);
        match self.agent.layout.hit(x, y) {
            Some(agent_config::Hit::Export(i)) => {
                if let Some(card) = self.agent.data.cards(section).get(i).cloned() {
                    if let Some(path) =
                        platform::save_file(HWND(self.hwnd_raw as *mut _), "Agent.mochi-agent.zip")
                    {
                        let result = (|| -> anyhow::Result<()> {
                            let content = std::fs::read_to_string(&card.source_path)?;
                            mochi_core::transfer::Package {
                                kind: mochi_core::transfer::Kind::Agent,
                                name: card.name,
                                content,
                            }
                            .write(&path)
                        })();
                        match result {
                            Ok(()) => self
                                .show_global_notice(&format!("Agent 已导出：{}", path.display())),
                            Err(error) => self.agent.error = Some(format!("导出失败：{error:#}")),
                        }
                    }
                }
            }
            Some(agent_config::Hit::TestMcp(i)) => self.start_mcp_test(i),
            Some(agent_config::Hit::Refresh) => self.reload_agent_config(),
            Some(agent_config::Hit::New) => self.create_agent_template(section),
            Some(agent_config::Hit::SectionCard(i)) => self.set_agent_config_section(i),
            Some(agent_config::Hit::OpenSource(i)) => {
                if let Some(card) = self.agent.data.cards(section).get(i) {
                    let path = card.source_path.clone();
                    self.open_agent_source(&path);
                }
            }
            Some(agent_config::Hit::Blank) | None => {}
            Some(_) => {}
        }
    }

    pub(super) fn paint_agent_config(&mut self, mut area: Rect, p: &Palette) {
        self.ensure_agent_config_loaded();
        self.agent.update_banner = Rect::ZERO;
        if let Some(review) = self.agent.update_review.as_ref() {
            self.agent.layout = agent_config::updates::paint(
                &mut self.list,
                area,
                review,
                &self.agent.update_diff,
                self.agent.update_upstream,
                self.agent.error.as_deref(),
                &mut self.agent.scroll,
                p,
            );
            return;
        }
        if !self.agent.updates.is_empty() {
            let banner = Rect::new(area.left, area.top, area.right, area.top + 48.0);
            self.list.rect(banner, p.surface_muted);
            let pending = self.agent.updates.iter().filter(|u| !u.skipped).count();
            self.list.text(
                Rect::new(
                    banner.left + 16.0,
                    banner.top,
                    banner.right - 16.0,
                    banner.bottom,
                ),
                if pending > 0 {
                    format!("发现 {pending} 项内置 Agent 更新 · 查看 diff")
                } else {
                    "已跳过的 Agent 更新 · 查看 diff".into()
                },
                TextStyle::Caption,
                p.accent,
            );
            self.agent.update_banner = banner;
            area.top = banner.bottom;
        }
        if let Some(editor) = self.agent.source_editor.as_mut() {
            let header = Rect::new(area.left, area.top, area.right, area.top + 52.0);
            self.list.rect(header, p.surface);
            self.list.border_bottom(header, p.border);
            self.list.text(
                Rect::new(
                    header.left + 20.0,
                    header.top,
                    header.right - 190.0,
                    header.bottom,
                ),
                format!(
                    "配置源码 · {}",
                    editor
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("未命名")
                ),
                TextStyle::Label,
                p.foreground,
            );
            let back = Rect::new(
                header.right - 174.0,
                header.top + 10.0,
                header.right - 98.0,
                header.bottom - 10.0,
            );
            let save = Rect::new(
                header.right - 86.0,
                header.top + 10.0,
                header.right - 20.0,
                header.bottom - 10.0,
            );
            self.list.rounded_border(back, 6.0, p.border);
            self.list.rounded_border(save, 6.0, p.border);
            self.list.text_aligned(
                back,
                "返回",
                TextStyle::Caption,
                p.foreground,
                Align::Center,
            );
            self.list.text_aligned(
                save,
                if editor.dirty { "保存 *" } else { "保存" },
                TextStyle::Caption,
                p.accent,
                Align::Center,
            );
            let source_area = Rect::new(
                area.left + 16.0,
                header.bottom + 12.0,
                area.right - 16.0,
                area.bottom - 16.0,
            );
            editor.area = source_area;
            editor.field.paint_multiline(
                &mut self.list,
                source_area,
                self.focus == Focus::AgentSource,
                p,
            );
            return;
        }
        let section = self.agent_config_section().unwrap_or(0);
        self.ensure_agent_config_loaded();
        let mut lay = agent_config::layout(area, &self.agent.data, section, self.agent.scroll);
        let max = lay.max_scroll();
        if self.agent.scroll > max {
            self.agent.scroll = max;
            lay = agent_config::layout(area, &self.agent.data, section, max);
        }
        let model = agent_config::Model {
            data: &self.agent.data,
            section,
            loading: false,
            error: self.agent.error.as_deref(),
            hover: self.agent.hover,
        };
        agent_config::paint(&mut self.list, area, &lay, &model, self.agent.scroll, p);
        self.agent.layout = lay;
    }
}
