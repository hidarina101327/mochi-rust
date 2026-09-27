//! 内联预测与选区改写。结果绑定文档指纹/字节范围，过期响应绝不修改当前缓冲区。
use super::*;
use mochi_core::ai::{
    models::{AiCompletionRequest, AiProvider},
    service::AiService,
};
use std::{
    hash::{Hash, Hasher},
    ops::Range,
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Inline,
    Rewrite { append: bool },
}
#[derive(Clone, Debug)]
pub(super) struct Snapshot {
    pub path: PathBuf,
    pub hash: u64,
    pub range: Range<usize>,
    pub kind: Kind,
    pub prefix: String,
    pub suffix: String,
}
#[derive(Clone, Debug)]
pub(super) struct Suggestion {
    pub snapshot: Snapshot,
    pub text: String,
}
struct Response {
    generation: u64,
    snapshot: Snapshot,
    result: std::result::Result<String, String>,
}
#[derive(Default)]
pub(super) struct State {
    generation: u64,
    rx: Option<Receiver<Response>>,
    service: Option<Arc<AiService>>,
    pub ghost: Option<Suggestion>,
    pub timer: Option<u32>,
    scheduled: bool,
    pending_kind: Option<Kind>,
    last: Option<(PathBuf, u64, usize)>,
}
impl Drop for State {
    fn drop(&mut self) {
        self.invalidate();
    }
}
pub(super) fn fingerprint(source: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    h.finish()
}
fn document_profile(source: &str) -> String {
    let mut headings = [0usize; 6];
    let mut stars = 0;
    let mut dashes = 0;
    let mut chinese = 0;
    let mut latin = 0;
    let mut languages = Vec::new();
    for c in source.chars() {
        if matches!(c as u32,0x2e80..=0x9fff|0xac00..=0xd7a3|0x20000..=0x3134f) {
            chinese += 1
        } else if c.is_ascii_alphabetic() {
            latin += 1
        }
    }
    for line in source.lines() {
        let line = line.trim_start();
        let n = line.bytes().take_while(|b| *b == b'#').count();
        if (1..=6).contains(&n) && line.as_bytes().get(n) == Some(&b' ') {
            headings[n - 1] += 1
        }
        stars += usize::from(line.starts_with("* "));
        dashes += usize::from(line.starts_with("- "));
        if let Some(lang) = line.strip_prefix("```").filter(|s| {
            !s.is_empty()
                && s.len() < 24
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_+-".contains(c))
        }) {
            if languages.len() < 8 && !languages.contains(&lang) {
                languages.push(lang);
            }
        }
    }
    serde_json::json!({"language":if chinese>latin{"中日韩文字较多，保持上下文语言"}else{"保持上下文语言"},"headingLevels":headings,"listMarker":if stars>dashes{"*"}else{"-"},"codeLanguages":languages,"lineEndings":if source.contains("\r\n"){"CRLF"}else{"LF"}}).to_string()
}
fn tail(source: &str, n: usize) -> String {
    source
        .chars()
        .rev()
        .take(n)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}
fn clean(raw: &str, prefix: &str, suffix: &str) -> String {
    let re = regex::Regex::new(r"(?is)<think>.*?</think>").unwrap();
    let mut value = re.replace_all(raw, "").trim_end().to_owned();
    if value.trim_start().starts_with("```") {
        if let Some(i) = value.find('\n') {
            value = value[i + 1..].to_owned();
        }
        if let Some(s) = value.trim_end().strip_suffix("```") {
            value = s.trim_end().into();
        }
    }
    let p = tail(prefix, 240);
    for (i, _) in p.char_indices() {
        let candidate = &p[i..];
        if candidate.chars().count() > 2 && value.starts_with(candidate) {
            value = value[candidate.len()..].into();
            break;
        }
    }
    let end = suffix
        .char_indices()
        .nth(240)
        .map(|(i, _)| i)
        .unwrap_or(suffix.len());
    let s = &suffix[..end];
    for (i, _) in s.char_indices().rev() {
        let candidate = &s[..i];
        if candidate.chars().count() > 8 && value.ends_with(candidate) {
            value.truncate(value.len() - candidate.len());
            break;
        }
    }
    value
}
impl State {
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.ghost = None;
        self.timer = None;
        self.scheduled = false;
        if let Some(s) = &self.service {
            s.cancel();
        }
        self.rx = None;
        self.service = None;
        self.pending_kind = None;
    }
    fn start(
        &mut self,
        provider: AiProvider,
        request: AiCompletionRequest,
        snapshot: Snapshot,
        hwnd: isize,
    ) {
        self.invalidate();
        let generation = self.generation;
        self.pending_kind = Some(snapshot.kind.clone());
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let service = Arc::new(AiService::new(provider));
        self.service = Some(service.clone());
        std::thread::spawn(move || {
            let result = service
                .complete(&request, Some(&mut |_| {}))
                .map(|r| r.content)
                .map_err(|e| e.to_string());
            if tx
                .send(Response {
                    generation,
                    snapshot,
                    result,
                })
                .is_ok()
            {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut _)),
                        platform::WM_APP_EDITOR_AI_READY,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }
        });
    }
}
impl App {
    /// Tab 行内补全是否开启。共享设置是唯一真相来源；仅当新键缺失时
    /// 才读旧的原生配置，所以即便默认值将来改为关闭，
    /// 显式写下的旧值 `true` 依然生效。
    pub(super) fn inline_completion_enabled(&self) -> bool {
        let key = "ai.inlineCompletionEnabled";
        let storage_key = format!("app.{key}");
        if self.settings.get(&storage_key).is_some() {
            return app_settings::descriptor(key).is_some_and(|descriptor| {
                matches!(self.app_settings.read(descriptor), SettingValue::Bool(true))
            });
        }
        if let Some(value) = self.settings.get("ai.inline.enabled") {
            return matches!(value.trim(), "true" | "1");
        }
        app_settings::descriptor(key).is_some_and(|descriptor| {
            matches!(self.app_settings.read(descriptor), SettingValue::Bool(true))
        })
    }

    pub(super) fn set_inline_completion_enabled(&mut self, enabled: bool) -> anyhow::Result<()> {
        self.settings.set(
            "app.ai.inlineCompletionEnabled",
            if enabled { "true" } else { "false" },
        );
        self.prefs.providers.inline_enabled = enabled;
        self.editor_ai.invalidate();
        self.settings.flush()?;
        self.state.status_text = if enabled {
            "已开启 AI 预测"
        } else {
            "已关闭 AI 预测"
        }
        .into();
        Ok(())
    }

    pub(super) fn inline_prediction_running(&self) -> bool {
        self.editor_ai.scheduled
            || (self.editor_ai.rx.is_some()
                && self.editor_ai.pending_kind.as_ref() == Some(&Kind::Inline))
    }

    pub(super) fn inline_prediction_available(&self) -> bool {
        self.editor_ai.ghost.is_some()
    }

    pub(super) fn inline_setting(&self, key: &str, legacy: &str) -> String {
        if self.settings.get(&format!("app.{key}")).is_none() {
            if let Some(value) = self.settings.get(legacy) {
                return value;
            }
        }
        app_settings::descriptor(key)
            .map(|d| self.app_settings.read(d).to_storage())
            .unwrap_or_default()
    }
    pub(super) fn allow_ai_request(&mut self) -> bool {
        if !app_settings::descriptor("ai.rateLimitEnabled")
            .is_some_and(|d| matches!(self.app_settings.read(d), SettingValue::Bool(true)))
        {
            return true;
        }
        let now = std::time::Instant::now();
        while self
            .ai_requests
            .front()
            .is_some_and(|t| now.duration_since(*t).as_secs() >= 60)
        {
            self.ai_requests.pop_front();
        }
        let limit = app_settings::descriptor("ai.maxRequestsPerMinute")
            .and_then(|d| match self.app_settings.read(d) {
                SettingValue::Number(n) => Some(n as usize),
                _ => None,
            })
            .unwrap_or(20)
            .max(1);
        if self.ai_requests.len() >= limit {
            return false;
        }
        self.ai_requests.push_back(now);
        true
    }
    pub(super) fn editor_snapshot(&self, kind: Kind) -> Option<Snapshot> {
        let path = self.active_file_path()?;
        if self.ai.permissions.as_ref().is_some_and(|p| {
            !p.is_path_visible(&path.to_string_lossy())
                || p.assert_tool_action_allowed(
                    mochi_core::ai::permission::AiToolAction::ReadFile,
                    Some(&path.to_string_lossy()),
                )
                .is_err()
        }) {
            return None;
        }
        let buffer = self.shell.active()?.buffer()?;
        if buffer.composition().is_some() {
            return None;
        }
        let (a, b) = buffer.selection();
        let range = if matches!(kind, Kind::Inline) {
            buffer.cursor()..buffer.cursor()
        } else {
            a..b
        };
        Some(Snapshot {
            path,
            hash: fingerprint(buffer.text()),
            prefix: tail(&buffer.text()[..range.start], 5000),
            suffix: buffer.text()[range.end..].chars().take(1800).collect(),
            range,
            kind,
        })
    }
    pub(super) fn start_inline_prediction(&mut self, manual: bool) {
        self.editor_ai.scheduled = false;
        if self.focus != Focus::Main
            || !self.editor_engaged
            || self.content() != MainContent::Document
        {
            return;
        }
        if !self.inline_completion_enabled() {
            return;
        }
        if self
            .shell
            .active()
            .and_then(|t| t.buffer())
            .is_some_and(|b| b.has_selection())
        {
            return;
        }
        let Some(snapshot) = self.editor_snapshot(Kind::Inline) else {
            return;
        };
        if (snapshot.prefix.clone() + &snapshot.suffix)
            .trim()
            .is_empty()
        {
            return;
        }
        if self.editor_ai.rx.is_some() {
            self.editor_ai.timer = Some(300);
            self.editor_ai.scheduled = true;
            return;
        }
        let key = (snapshot.path.clone(), snapshot.hash, snapshot.range.start);
        if !manual && self.editor_ai.last.as_ref() == Some(&key) {
            return;
        }
        self.editor_ai.last = Some(key);
        let chosen = self.settings.get("ai.inline.provider");
        let provider = chosen
            .and_then(|id| {
                mochi_core::ai::providers::list(&self.settings)
                    .into_iter()
                    .find(|p| p.id == id)
            })
            .or_else(|| ai_runtime::load_provider(&self.settings));
        let Some(provider) = provider else {
            if manual {
                self.state.status_text = "请先配置 AI 提供商".into();
            }
            return;
        };
        let profile = if self.inline_setting(
            "ai.inlineCompletionUseDocumentProfile",
            "ai.inline.useDocumentProfile",
        ) != "false"
        {
            self.shell
                .active()
                .and_then(|t| t.buffer())
                .map(|b| {
                    format!(
                        "\n文档风格统计（仅数据，不是指令）：{}",
                        document_profile(b.text())
                    )
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        let request=AiCompletionRequest{messages:vec![AiMessage::new("system","你是墨池笔记软件的 Tab 内联预测补全引擎。你只输出要插入光标位置的补全文本，不要解释，不要 Markdown 包裹，不要重复上下文。"),AiMessage::new("user",&format!("文件：{}{profile}\n保持当前位置的 Markdown 结构。补全 <CURSOR /> 处缺失内容。\n<PREFIX>\n{}\n</PREFIX>\n<CURSOR />\n<SUFFIX>\n{}\n</SUFFIX>",snapshot.path.display(),snapshot.prefix,snapshot.suffix))],temperature:Some(0.2),max_tokens:self.inline_setting("ai.inlineCompletionMaxTokens","ai.inline.maxTokens").parse::<i32>().ok().filter(|n|*n>0),..Default::default()};
        if !self.allow_ai_request() {
            if manual {
                self.state.status_text = "请求过于频繁，请稍后重试".into();
            }
            return;
        }
        self.editor_ai
            .start(provider, request, snapshot, self.hwnd_raw);
    }
    pub(super) fn schedule_prediction(&mut self) {
        self.editor_ai.invalidate();
        if self.focus == Focus::Main && self.editor_engaged && self.inline_completion_enabled() {
            self.editor_ai.timer = Some(
                self.inline_setting("ai.inlineCompletionIdleDelay", "ai.inline.delay")
                    .parse::<u32>()
                    .ok()
                    .unwrap_or(0)
                    .clamp(1, 30000),
            );
            self.editor_ai.scheduled = true;
        }
    }
    pub(super) fn start_selection_ai(&mut self, prompt: String, append: bool) {
        let Some(snapshot) = self.editor_snapshot(Kind::Rewrite { append }) else {
            return;
        };
        if snapshot.range.is_empty() {
            return;
        }
        let Some(provider) = ai_runtime::load_provider(&self.settings) else {
            self.open_settings("ai");
            return;
        };
        self.editor_ai.timer = None;
        if !self.allow_ai_request() {
            self.state.status_text = "请求过于频繁，请稍后重试".into();
            return;
        }
        let request=AiCompletionRequest{messages:vec![AiMessage::new("system","你是一个文档编辑助手。只输出处理后的内容本身，不要解释或元描述。保持 Markdown 格式，直接输出最终内容。"),AiMessage::new("user",&prompt)],temperature:Some(0.2),..Default::default()};
        self.state.status_text = "AI 正在处理选区…".into();
        self.editor_ai
            .start(provider, request, snapshot, self.hwnd_raw);
    }
    pub fn take_editor_ai(&mut self) {
        let Some(response) = self.editor_ai.rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.editor_ai.rx = None;
        self.editor_ai.service = None;
        self.editor_ai.pending_kind = None;
        if response.generation != self.editor_ai.generation {
            return;
        }
        let valid = self.active_file_path().as_ref() == Some(&response.snapshot.path)
            && self
                .shell
                .active()
                .and_then(|t| t.buffer())
                .is_some_and(|b| fingerprint(b.text()) == response.snapshot.hash);
        if !valid {
            self.state.status_text = "文档已变化，已丢弃过期 AI 结果".into();
            return;
        }
        match response.result {
            Ok(raw) => match response.snapshot.kind {
                Kind::Inline => {
                    let value = clean(&raw, &response.snapshot.prefix, &response.snapshot.suffix);
                    if !value.trim().is_empty() {
                        self.editor_ai.ghost = Some(Suggestion {
                            snapshot: response.snapshot,
                            text: value,
                        });
                    }
                }
                Kind::Rewrite { append } => {
                    if let Some(buffer) = self.shell.active_buffer_mut() {
                        let range = if append {
                            response.snapshot.range.end..response.snapshot.range.end
                        } else {
                            response.snapshot.range
                        };
                        buffer.replace_range(range, &raw);
                        self.after_doc_edit(false);
                        self.state.status_text = "AI 处理已应用，可撤销".into();
                    }
                }
            },
            Err(e) => self.state.status_text = format!("AI 处理失败：{e}"),
        }
    }
    pub(super) fn accept_prediction(&mut self, mode: u8) -> bool {
        let Some(ghost) = self.editor_ai.ghost.take() else {
            return false;
        };
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return false;
        };
        if fingerprint(buffer.text()) != ghost.snapshot.hash
            || buffer.cursor() != ghost.snapshot.range.start
            || buffer.has_selection()
        {
            return false;
        }
        let n = match mode {
            1 => ghost
                .text
                .find('\n')
                .map(|i| i + 1)
                .unwrap_or(ghost.text.len()),
            2 => ghost.text.chars().next().map(char::len_utf8).unwrap_or(0),
            _ => ghost.text.len(),
        };
        buffer.insert(&ghost.text[..n]);
        let mut remaining = ghost.clone();
        remaining.text = ghost.text[n..].into();
        remaining.snapshot.range = buffer.cursor()..buffer.cursor();
        remaining.snapshot.hash = fingerprint(buffer.text());
        self.after_doc_edit(false);
        self.editor_ai.timer = None;
        if !remaining.text.is_empty() {
            self.editor_ai.ghost = Some(remaining);
        }
        true
    }
    pub(super) fn paint_prediction(&mut self, area: Rect, p: &Palette) {
        let Some(ghost) = &self.editor_ai.ghost else {
            return;
        };
        let Some(buffer) = self.shell.active().and_then(|t| t.buffer()) else {
            return;
        };
        if self.focus != Focus::Main
            || buffer.cursor() != ghost.snapshot.range.start
            || fingerprint(buffer.text()) != ghost.snapshot.hash
        {
            return;
        }
        let Some(caret) = self
            .doc
            .caret_rect(area, buffer, self.shell.active_scroll())
        else {
            return;
        };
        let below = !ghost
            .snapshot
            .suffix
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .is_empty();
        let style = self.doc.style_at(ghost.snapshot.range.start);
        let padding_left = crate::ui::editor_preferences::current().padding_left;
        let continuation_left = self
            .doc
            .line_left(area, ghost.snapshot.range.start)
            .unwrap_or(area.left + padding_left);
        let lines = prediction_lines(
            area,
            caret,
            below,
            padding_left,
            continuation_left,
            style,
            &ghost.text,
        );
        self.list.push_clip(area);
        for line in lines {
            // Electron 用 rgba(120, 128, 140, .72)、无背板。
            // 在配置的文档底色上混合同样的中性灰。
            self.list
                .text(line.rect, line.text, style, prediction_gray(p));
        }
        self.list.pop_clip();
    }
}

#[derive(Clone, Debug, PartialEq)]
struct PredictionLine {
    rect: Rect,
    text: String,
}

fn prediction_gray(p: &Palette) -> u32 {
    crate::ui::theme::mix(0x78808c, p.area_main_default, 0.72)
}

/// 为 Tab 将插入的每个字符排版。显式换行和软换行各成一行；
/// 空格按源码文本原样保留，预览才不会与最终接受的补全不一致。
fn prediction_lines(
    area: Rect,
    caret: Rect,
    below: bool,
    padding_left: f32,
    continuation_left: f32,
    style: crate::ui::draw::TextStyle,
    value: &str,
) -> Vec<PredictionLine> {
    let first = prediction_rect(area, caret, below, padding_left);
    let continuation_width = (area.right - 12.0 - continuation_left).max(0.0);
    let mut y = first.top;
    let mut output = Vec::new();
    let mut first_visual_row = true;

    for logical_line in value.split('\n') {
        let mut remaining = logical_line;
        loop {
            let left = if first_visual_row {
                first.left
            } else {
                continuation_left
            };
            let width = if first_visual_row {
                first.width()
            } else {
                continuation_width
            };
            let runs = crate::ui::text::wrap_source(remaining, style, width.max(1.0))
                .into_iter()
                .next()
                .unwrap_or_else(|| vec![crate::ui::text::Run::plain("")]);
            let text: String = runs.into_iter().map(|run| run.text).collect();
            output.push(PredictionLine {
                rect: Rect::from_size(left, y, width, caret.height()),
                text: text.clone(),
            });
            y += caret.height();
            first_visual_row = false;
            if remaining.is_empty() || text.len() >= remaining.len() {
                break;
            }
            remaining = &remaining[text.len()..];
        }
    }
    output
}

/// 幽灵文字与渲染出的光标放在同一行几何里。
///
/// 编辑器的文字格式由 DirectWrite 垂直居中。固定的 24px 预测矩形
/// 在文档行高被自定义时（例如 28px 字号、2 倍行高）会把字形上移。
/// 行高从光标推导，预测行与真实行在任何排版设置和 DPI 下
/// 都处在同一基线上。
fn prediction_rect(area: Rect, caret: Rect, below: bool, padding_left: f32) -> Rect {
    let x = if area.right - caret.left < 120.0 {
        area.left + padding_left
    } else {
        caret.left
    };
    let y = if below { caret.bottom } else { caret.top };
    Rect::from_size(x, y, (area.right - 12.0 - x).max(0.0), caret.height())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_profile_contains_structure_not_document_body() {
        let profile = document_profile(
            "# 标题\r\n\r\n* 事项\r\n```rust\r\nconst SECRET:&str=\"private-body\";\r\n```\r\n",
        );
        assert!(profile.contains("rust"));
        assert!(profile.contains("CRLF"));
        assert!(!profile.contains("private-body"));
        assert!(!profile.contains("SECRET"));
    }
    #[test]
    fn completion_cleanup_preserves_unicode_boundaries() {
        assert_eq!(
            clean("```text\n已有中文新的句子\n```", "已有中文", ""),
            "新的句子"
        );
        assert_eq!(clean("<think>隐藏</think>好", "", ""), "好");
    }
    #[test]
    fn fingerprint_changes_for_equal_length_edits() {
        assert_ne!(fingerprint("甲"), fingerprint("乙"));
    }

    #[test]
    fn prediction_rect_uses_caret_line_height_and_scroll_position() {
        let area = Rect::new(40.0, 100.0, 840.0, 700.0);
        let caret = Rect::new(312.0, 244.0, 314.0, 300.0);
        let r = prediction_rect(area, caret, false, 24.0);
        assert_eq!(r.left, caret.left);
        assert_eq!(r.top, caret.top);
        assert_eq!(r.height(), caret.height());

        let below = prediction_rect(area, caret, true, 24.0);
        assert_eq!(below.top, caret.bottom);
        assert_eq!(below.height(), caret.height());
    }

    #[test]
    fn prediction_rect_uses_configured_padding_when_wrapping_at_right_edge() {
        let area = Rect::new(40.0, 100.0, 840.0, 700.0);
        let caret = Rect::new(730.0, 244.0, 732.0, 300.0);
        let r = prediction_rect(area, caret, false, 56.0);
        assert_eq!(r.left, area.left + 56.0);
        assert_eq!(r.width(), area.right - 12.0 - r.left);
        assert_eq!(r.height(), caret.height());
    }

    #[test]
    fn prediction_rect_follows_a_nested_list_caret_in_scrolled_dip_geometry() {
        struct Restore(crate::ui::editor_preferences::Preferences);
        impl Drop for Restore {
            fn drop(&mut self) {
                crate::ui::editor_preferences::set(self.0.clone());
            }
        }

        let old = crate::ui::editor_preferences::current();
        let _restore = Restore(old.clone());
        let mut prefs = old;
        prefs.padding_left = 56.0;
        prefs.list_indent = 34.0;
        prefs.font_size = 28.0;
        prefs.line_height = 2.0;
        crate::ui::editor_preferences::set(prefs.clone());

        let source = "前文\n\n  - 仪表盘";
        let cursor = source.len();
        let live = crate::ui::live::layout(source, Some(cursor), 700.0, &|_| None);
        let mut buffer = crate::ui::editor::TextBuffer::new(source);
        buffer.set_cursor(cursor, false);
        let area = Rect::new(10.5, 20.25, 810.5, 620.25);
        let scroll = 17.75;
        let caret = crate::ui::live::caret_in_area(area, &live, &buffer, scroll).unwrap();
        let (line_index, local_x) = live.locate(cursor).unwrap();
        let line = &live.layout.lines[line_index];

        assert!(
            line.x >= prefs.list_indent,
            "list text lost its indentation"
        );
        assert!((caret.left - (area.left + prefs.padding_left + line.x + local_x)).abs() < 0.001);
        assert!((caret.top - (area.top - scroll + line.y)).abs() < 0.001);

        let prediction = prediction_rect(area, caret, false, prefs.padding_left);
        assert!((prediction.left - caret.left).abs() < 0.001);
        assert!((prediction.top - caret.top).abs() < 0.001);
        assert!((prediction.height() - line.height).abs() < 0.001);
    }

    #[test]
    fn prediction_preview_includes_every_explicit_and_wrapped_line() {
        let area = Rect::new(0.0, 0.0, 260.0, 400.0);
        let caret = Rect::new(130.0, 40.0, 132.0, 68.0);
        let value = "第一行很长，需要从光标处换到下一视觉行\n第二行完整显示\n";
        let lines = prediction_lines(
            area,
            caret,
            false,
            24.0,
            area.left + 24.0,
            crate::ui::draw::TextStyle::Document,
            value,
        );
        assert!(
            lines.len() >= 4,
            "soft and explicit wraps must all be visible"
        );
        assert_eq!(lines.last().map(|line| line.text.as_str()), Some(""));
        assert_eq!(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            value.replace('\n', "")
        );
        assert_eq!(lines[0].rect.left, caret.left);
        assert!(lines[1..]
            .iter()
            .all(|line| line.rect.left == area.left + 24.0));
        assert!(lines.windows(2).all(|rows| (rows[1].rect.top
            - rows[0].rect.top
            - caret.height())
        .abs()
            < 0.001));
    }

    #[test]
    fn prediction_uses_electron_neutral_gray_instead_of_accent() {
        let palette = crate::ui::theme::tokens().palette(false);
        assert_eq!(
            prediction_gray(palette),
            crate::ui::theme::mix(0x78808c, palette.area_main_default, 0.72)
        );
        assert_ne!(prediction_gray(palette), palette.accent);
    }
}
