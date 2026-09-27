//! AIProviderSettings：多提供商列表、选择、编辑与显式连接测试。
use super::{
    draw::{Align, DrawList, TextStyle},
    layout::Rect,
    text,
    theme::Palette,
    widgets::{FieldLook, TextField},
};
use mochi_core::ai::models::AiProvider;
mod interaction;
#[cfg(test)]
mod preset_tests;
pub const LABELS: [&str; 4] = ["名称", "Base URL", "模型", "API Key"];
pub const OPENAI_PROTOCOL: &str = "openai-completions";
pub const ANTHROPIC_PROTOCOL: &str = "anthropic-messages";
pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const OPENAI_DEFAULT_MODEL: &str = "gpt-4";
pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1";
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-sonnet-5";

/// 原生请求服务支持的协议。此列表应保持精简：
/// 上游预设可能还会提供其他协议，但若在此选择这些协议，
/// 已保存的服务方就会在请求层尚未支持时
/// 看起来仍可用。
pub const PROTOCOLS: [(&str, &str); 2] = [
    (OPENAI_PROTOCOL, "OpenAI Completions"),
    (ANTHROPIC_PROTOCOL, "Anthropic Messages"),
];

pub fn protocol_label(protocol: &str) -> &'static str {
    PROTOCOLS
        .iter()
        .find_map(|(value, label)| (*value == protocol).then_some(*label))
        .unwrap_or("未知协议")
}

fn normalized_protocol(protocol: &str) -> String {
    if protocol.trim().is_empty() {
        OPENAI_PROTOCOL.into()
    } else {
        protocol.into()
    }
}

pub struct Form {
    pub original: AiProvider,
    pub editing_existing: bool,
    pub id: String,
    pub fields: Vec<TextField>,
    pub protocol: String,
    pub stream: bool,
    pub focus: usize,
    pub preset: Option<usize>,
}
impl Form {
    pub fn new(p: AiProvider) -> Self {
        let original = p.clone();
        let fields = [p.name, p.base_url, p.model, p.api_key]
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                let mut f = TextField::new(LABELS[i]);
                f.set_text(&s);
                f.style = TextStyle::Label;
                f
            })
            .collect();
        Self {
            original,
            editing_existing: false,
            id: p.id,
            fields,
            protocol: normalized_protocol(&p.protocol),
            stream: p.stream,
            focus: 0,
            preset: None,
        }
    }
    pub fn value(&self) -> AiProvider {
        AiProvider {
            id: self.id.clone(),
            name: self.fields[0].text().trim().into(),
            base_url: self.fields[1].text().trim().into(),
            model: self.fields[2].text().trim().into(),
            api_key: self.fields[3].text().into(),
            protocol: normalized_protocol(&self.protocol),
            stream: self.stream,
        }
    }

    pub fn apply_protocol(&mut self, protocol: &str) -> bool {
        if !PROTOCOLS.iter().any(|(value, _)| *value == protocol) {
            return false;
        }
        let previous = normalized_protocol(&self.protocol);
        if previous == protocol {
            self.focus = 3;
            return true;
        }
        // 用户更改协议时，让内置的“新建服务方”默认值仍然实用，
        // 同时保留用户已经填写的端点和模型
        // 不变。
        let base_url = self.fields[1].text().trim();
        let model = self.fields[2].text().trim();
        let mut endpoint_changed = false;
        if previous == OPENAI_PROTOCOL
            && protocol == ANTHROPIC_PROTOCOL
            && base_url == OPENAI_BASE_URL
            && model == OPENAI_DEFAULT_MODEL
        {
            self.fields[1].set_text(ANTHROPIC_BASE_URL);
            self.fields[2].set_text(ANTHROPIC_DEFAULT_MODEL);
            endpoint_changed = true;
        } else if previous == ANTHROPIC_PROTOCOL
            && protocol == OPENAI_PROTOCOL
            && base_url == ANTHROPIC_BASE_URL
            && model == ANTHROPIC_DEFAULT_MODEL
        {
            self.fields[1].set_text(OPENAI_BASE_URL);
            self.fields[2].set_text(OPENAI_DEFAULT_MODEL);
            endpoint_changed = true;
        }
        if endpoint_changed {
            self.fields[3].set_text("");
        }
        self.protocol = protocol.into();
        self.preset = None;
        self.focus = 3;
        true
    }

    pub fn preset_label(&self) -> String {
        let Some(p) = self
            .preset
            .and_then(|i| mochi_core::ai::provider_presets::all().get(i))
        else {
            return "自定义  ▾".into();
        };
        let edited = self.fields[0].text().trim() != p.name
            || self.fields[1].text().trim() != p.base_url
            || self.fields[2].text().trim() != p.model
            || normalized_protocol(&self.protocol) != p.protocol;
        format!("{}{}  ▾", p.name, if edited { "（已修改）" } else { "" })
    }
    pub fn apply_preset(&mut self, index: Option<usize>) -> bool {
        let Some(index) = index else {
            self.preset = None;
            return true;
        };
        let Some(p) = mochi_core::ai::provider_presets::all()
            .get(index)
            .filter(|p| p.supported)
        else {
            return false;
        };
        // 应用预设可能会将表单切换到另一个服务。端点或协议范围发生变化时，
        // 应清除密钥；如果用户只切换协议，
        // 而端点不变，则保留该端点对应的密钥。
        if self.fields[1].text().trim().trim_end_matches('/') != p.base_url.trim_end_matches('/')
            || normalized_protocol(&self.protocol) != p.protocol
        {
            self.fields[3].set_text("");
        }
        for (field, value) in self.fields.iter_mut().zip([&p.name, &p.base_url, &p.model]) {
            field.set_text(value);
        }
        self.protocol = normalized_protocol(&p.protocol);
        self.preset = Some(index);
        self.focus = 3;
        true
    }
    pub fn merge_edited_fields(&self, mut current: AiProvider) -> AiProvider {
        let edited = self.value();
        let endpoint_changed = edited.base_url != self.original.base_url;
        if edited.name != self.original.name {
            current.name = edited.name;
        }
        if edited.base_url != self.original.base_url {
            current.base_url = edited.base_url;
        }
        if edited.model != self.original.model {
            current.model = edited.model;
        }
        if edited.protocol != normalized_protocol(&self.original.protocol) {
            current.protocol = edited.protocol.clone();
        }
        if edited.api_key != self.original.api_key || endpoint_changed {
            current.api_key = edited.api_key;
        }
        if edited.stream != self.original.stream {
            current.stream = edited.stream;
        }
        current
    }
}
#[derive(Default)]
pub struct State {
    pub interaction: interaction::Interaction,
    pub actions: mochi_core::ai::permission::ActionPermissions,
    pub inline_enabled: bool,
    pub all: Vec<AiProvider>,
    pub selected: Option<String>,
    pub form: Option<Form>,
    pub loaded: bool,
    pub scroll: f32,
    pub status: String,
    /// 已保存的服务方正在进行连接测试。将此状态放入界面数据中，
    /// 可让同一个按钮明确变为“取消”操作。
    pub test_provider_id: Option<String>,
    /// 服务方表单正在加载实际返回的 `/models` 列表。
    pub models_provider_id: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Options,
    Permission(usize),
    ToggleInline,
    Add,
    Select(usize),
    Edit(usize),
    Delete(usize),
    Test(usize),
    Field(usize),
    Stream,
    Save,
    Cancel,
    /// 为当前服务方获取或取消模型列表请求。
    Models,
    /// 选择服务方使用的传输协议。
    Protocol,
    Preset,
}
#[derive(Default)]
pub struct Layout {
    pub entries: Vec<(Rect, Hit)>,
    pub title: Rect,
    pub body: Rect,
    pub height: f32,
    pub rows: Vec<(Rect, usize)>,
    pub empty: Option<Rect>,
    /// 模型字段中的子控件。它不放在 `entries` 中，
    /// 这样布局重叠测试就能清晰描述字段控件。
    pub model_button: Option<Rect>,
}
impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        if self
            .model_button
            .is_some_and(|r| r.contains(x, y) && self.body.contains(x, y))
        {
            return Some(Hit::Models);
        }
        self.entries
            .iter()
            .rev()
            .find(|(r, h)| {
                r.contains(x, y)
                    && (matches!(h, Hit::Add | Hit::Options) || self.body.contains(x, y))
            })
            .map(|(_, h)| *h)
    }
    pub fn max_scroll(&self) -> f32 {
        (self.height - self.body.height()).max(0.0)
    }
    pub fn field(&self, i: usize) -> Option<Rect> {
        self.entries
            .iter()
            .find(|(_, h)| *h == Hit::Field(i))
            .map(|(r, _)| *r)
    }
}
fn button_rows(
    hits: &[Hit],
    left: f32,
    top: f32,
    width: f32,
    size: [f32; 2],
    gap: f32,
) -> (Vec<(Rect, Hit)>, f32) {
    let columns = (((width + gap) / (size[0] + gap)).floor() as usize).clamp(1, hits.len());
    let button_width = size[0].min(width);
    let entries = hits
        .iter()
        .enumerate()
        .map(|(i, hit)| {
            let x = left + (i % columns) as f32 * (button_width + gap);
            let y = top + (i / columns) as f32 * (size[1] + gap);
            (Rect::from_size(x, y, button_width, size[1]), *hit)
        })
        .collect();
    let rows = hits.len().div_ceil(columns);
    (entries, rows as f32 * (size[1] + gap) - gap)
}
pub fn layout(s: &State, area: Rect) -> Layout {
    if area.is_empty() {
        return Layout::default();
    }
    // 打开 AI 设置后，两个侧栏之间可能只剩 124 DIP。
    // 所有操作都要放在这段宽度内；超出的垂直内容可滚动查看。
    let padding = 24.0_f32.min((area.width() - 104.0).max(0.0) / 2.0);
    let left = area.left + padding;
    let right = area.right - padding;
    let width = right - left;
    let header_actions: &[Hit] = if s.form.is_none() {
        &[Hit::Options, Hit::Add]
    } else {
        &[Hit::Add]
    };
    let actions_width = header_actions.len() as f32 * 116.0 - 12.0;
    let inline_header = width >= 112.0 + actions_width;
    let actions_top = area.top + if inline_header { 20.0 } else { 56.0 };
    let actions_left = if inline_header {
        right - actions_width
    } else {
        left
    };
    let (entries, actions_height) = button_rows(
        header_actions,
        actions_left,
        actions_top,
        right - actions_left,
        [104.0, 36.0],
        12.0,
    );
    let title_height = TextStyle::Large.line_height().max(28.0);
    let mut l = Layout {
        entries,
        title: Rect::new(
            left,
            area.top + 16.0,
            if inline_header {
                actions_left - 12.0
            } else {
                right
            },
            area.top + 16.0 + title_height,
        ),
        body: Rect::new(
            area.left,
            actions_top + actions_height + 24.0,
            area.right,
            area.bottom,
        ),
        ..Default::default()
    };
    let mut y = l.body.top + 24.0 - s.scroll;
    l.entries.push((
        Rect::new(left, y, left + width.min(240.0), y + 36.0),
        Hit::ToggleInline,
    ));
    y += 56.0;
    let cols = if width > 460.0 {
        3
    } else if width >= 220.0 {
        2
    } else {
        1
    };
    let width = (right - left - 12.0 * (cols - 1) as f32) / cols as f32;
    for i in 0..6 {
        let x = left + (i % cols) as f32 * (width + 12.0);
        let top = y + (i / cols) as f32 * 40.0;
        l.entries
            .push((Rect::new(x, top, x + width, top + 32.0), Hit::Permission(i)));
    }
    y += 6_usize.div_ceil(cols) as f32 * 40.0 + 16.0;
    if s.form.is_some() {
        l.entries
            .push((Rect::new(left, y + 20.0, right, y + 56.0), Hit::Protocol));
        y += 88.0;
        l.entries
            .push((Rect::new(left, y + 20.0, right, y + 56.0), Hit::Preset));
        y += 72.0;
        for i in 0..4 {
            let field = Rect::new(left, y + 20.0, right, y + 56.0);
            l.entries.push((field, Hit::Field(i)));
            if i == 2 && field.width() > 0.0 {
                let button_width = field.width().clamp(54.0, 96.0);
                l.model_button = Some(Rect::new(
                    (field.right - button_width - 4.0).max(field.left + 4.0),
                    field.top + 4.0,
                    field.right - 4.0,
                    field.bottom - 4.0,
                ));
            }
            y += 72.0;
        }
        let (entries, height) = button_rows(
            &[Hit::Stream, Hit::Save, Hit::Cancel],
            left,
            y,
            right - left,
            [108.0, 36.0],
            12.0,
        );
        l.entries.extend(entries);
        y += height + 20.0;
    }
    if s.all.is_empty() && s.form.is_none() {
        l.empty = Some(Rect::new(left, y, right, y + 48.0));
        y += 72.0;
    }
    for (i, _) in s.all.iter().enumerate() {
        let inset = 12.0_f32.min((right - left) / 4.0);
        let (entries, height) = button_rows(
            &[Hit::Select(i), Hit::Test(i), Hit::Edit(i), Hit::Delete(i)],
            left + inset,
            y + 72.0,
            right - left - inset * 2.0,
            [80.0, 32.0],
            8.0,
        );
        let r = Rect::new(left, y, right, y + 86.0 + height);
        l.rows.push((r, i));
        l.entries.extend(entries);
        y = r.bottom + 12.0;
    }
    l.height = y - l.body.top + s.scroll + 40.0;
    l
}
pub fn paint(
    list: &mut DrawList,
    area: Rect,
    s: &mut State,
    l: &Layout,
    focused: bool,
    p: &Palette,
) {
    list.push_clip(area);
    list.rect(area, p.surface);
    list.text(l.title, "AI 配置", TextStyle::Large, p.foreground);
    list.hline(area.left, area.right, l.body.top - 1.0, p.border);
    for (r, h) in &l.entries {
        if *h == Hit::Options {
            s.interaction.button(list, *r, *h, "请求参数", false, p);
        }
        if *h == Hit::Add {
            s.interaction
                .button(list, *r, *h, "添加提供商", s.form.is_none(), p);
        }
    }
    list.push_clip(l.body);
    if let Some(r) = l.empty {
        list.text(
            Rect::new(r.left, r.top, r.right, r.top + 24.0),
            "还没有 AI 提供商",
            TextStyle::Caption,
            p.muted,
        );
        list.text(
            Rect::new(r.left, r.top + 24.0, r.right, r.bottom),
            "请添加配置。",
            TextStyle::Caption,
            p.muted,
        );
    }
    for (r, i) in &l.rows {
        let provider = &s.all[*i];
        list.rounded_rect(*r, 12.0, p.background);
        list.rounded_border(
            *r,
            12.0,
            if s.selected.as_deref() == Some(provider.id.as_str()) {
                p.accent
            } else {
                p.border
            },
        );
        list.text(
            Rect::new(r.left + 16.0, r.top + 12.0, r.right - 16.0, r.top + 34.0),
            format!(
                "{}{}",
                provider.name,
                if s.selected.as_deref() == Some(provider.id.as_str()) {
                    " · 当前"
                } else {
                    ""
                }
            ),
            TextStyle::Label,
            p.foreground,
        );
        list.text(
            Rect::new(r.left + 16.0, r.top + 40.0, r.right - 16.0, r.top + 60.0),
            text::ellipsize(
                &format!(
                    "{} · {} · {}",
                    protocol_label(&provider.protocol),
                    provider.model,
                    provider.base_url
                ),
                TextStyle::Caption,
                (r.width() - 32.0).max(0.0),
            ),
            TextStyle::Caption,
            p.muted,
        );
    }
    for (r, h) in &l.entries {
        if *h == Hit::Protocol {
            if let Some(form) = &s.form {
                list.text(
                    Rect::new(r.left, r.top - 20.0, r.right, r.top),
                    "API 协议",
                    TextStyle::Caption,
                    p.muted,
                );
                let label = format!("{}  ▾", protocol_label(&form.protocol));
                s.interaction.button(list, *r, *h, &label, false, p);
                list.text(
                    Rect::new(r.left, r.bottom + 4.0, r.right, r.bottom + 22.0),
                    "Anthropic Messages 使用 API Key；OAuth 与 AWS Bedrock 暂不支持。",
                    TextStyle::Tiny,
                    p.muted,
                );
            }
            continue;
        }
        if *h == Hit::Preset {
            if let Some(form) = &s.form {
                list.text(
                    Rect::new(r.left, r.top - 20.0, r.right, r.top),
                    "提供商预设",
                    TextStyle::Caption,
                    p.muted,
                );
                let label = text::ellipsize(
                    &form.preset_label(),
                    TextStyle::Label,
                    (r.width() - 24.0).max(0.0),
                );
                s.interaction.button(list, *r, *h, &label, false, p);
            }
            continue;
        }
        if let Hit::Permission(i) = h {
            let action = mochi_core::ai::permission::AiToolAction::ALL[*i];
            list.rounded_border(*r, 8.0, p.border);
            list.text(
                Rect::new(r.left + 10.0, r.top, r.right, r.bottom),
                format!(
                    "{} {}",
                    if s.actions.is_allowed(action) {
                        "☑"
                    } else {
                        "☐"
                    },
                    action.label()
                ),
                TextStyle::Caption,
                p.foreground,
            );
            continue;
        }
        let label = match h {
            Hit::ToggleInline => {
                if s.inline_enabled {
                    "☑ Tab 内联预测（Ctrl+Space 手动触发）"
                } else {
                    "☐ Tab 内联预测"
                }
            }
            Hit::Select(i) => {
                if s.all
                    .get(*i)
                    .is_some_and(|v| s.selected.as_deref() == Some(v.id.as_str()))
                {
                    "✓ 当前使用"
                } else {
                    "设为当前"
                }
            }
            Hit::Edit(_) => "编辑",
            Hit::Delete(_) => "删除",
            Hit::Test(i) => {
                if s.test_provider_id
                    .as_deref()
                    .is_some_and(|id| s.all.get(*i).is_some_and(|p| p.id == id))
                {
                    "取消测试"
                } else {
                    "测试连接"
                }
            }
            Hit::Save => "保存配置",
            Hit::Cancel => "取消",
            Hit::Models => continue,
            Hit::Stream => {
                if s.form.as_ref().is_some_and(|f| f.stream) {
                    "☑ 流式输出"
                } else {
                    "☐ 流式输出"
                }
            }
            _ => continue,
        };
        let busy = matches!(h, Hit::Test(i) if s.all.get(*i).is_some_and(|provider| s.test_provider_id.as_deref() == Some(provider.id.as_str())));
        s.interaction
            .button(list, *r, *h, label, *h == Hit::Save || busy, p);
    }
    let models_loading = s.models_provider_id.is_some();
    if let Some(form) = s.form.as_mut() {
        for (i, label) in LABELS.iter().enumerate().take(4) {
            if let Some(r) = l.field(i) {
                list.text(
                    Rect::new(r.left, r.top - 20.0, r.right, r.top),
                    *label,
                    TextStyle::Caption,
                    p.muted,
                );
                if i == 3 {
                    form.fields[i].paint_masked(
                        list,
                        r,
                        focused && form.focus == i,
                        p,
                        FieldLook::dialog(p),
                    );
                } else {
                    form.fields[i].paint(
                        list,
                        r,
                        focused && form.focus == i,
                        p,
                        FieldLook::dialog(p),
                    );
                }
            }
        }
        if let Some(button) = l.model_button {
            let label = if models_loading {
                "取消获取"
            } else {
                "获取模型"
            };
            s.interaction
                .button(list, button, Hit::Models, label, models_loading, p);
        }
    }
    list.pop_clip();
    if !s.status.is_empty() {
        list.rect(
            Rect::new(area.left, area.bottom - 32.0, area.right, area.bottom),
            p.surface,
        );
        list.text(
            Rect::new(
                area.left + 24.0,
                area.bottom - 32.0,
                area.right - 24.0,
                area.bottom,
            ),
            text::ellipsize(
                &s.status,
                TextStyle::Caption,
                (area.width() - 48.0).max(0.0),
            ),
            TextStyle::Caption,
            if s.status.contains("失败") || s.status.contains("错误") {
                p.danger
            } else if s.status.contains("已保存") || s.status.contains("成功") {
                p.accent
            } else {
                p.muted
            },
        );
    }
    list.pop_clip();
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_provider_hint_follows_permissions_without_overlap() {
        for width in [124.0, 800.0] {
            let l = layout(&State::default(), Rect::from_size(0.0, 0.0, width, 700.0));
            let empty = l.empty.unwrap();
            assert!(l.entries.iter().all(|(rect, _)| rect.bottom < empty.top));
            assert!(empty.bottom - l.body.top <= l.height);
        }
    }

    #[test]
    fn actions_stay_inside_narrow_settings_and_remain_reachable_after_scroll() {
        for width in [124.0, 180.0, 300.0, 380.0, 800.0] {
            for editing in [false, true] {
                let area = Rect::from_size(480.0, 69.0, width, 507.0);
                let mut s = State {
                    all: vec![AiProvider::default()],
                    form: editing.then(|| Form::new(AiProvider::default())),
                    ..State::default()
                };
                let l = layout(&s, area);
                for (index, (rect, hit)) in l.entries.iter().enumerate() {
                    assert!(
                        rect.width() > 0.0 && rect.left >= area.left && rect.right <= area.right,
                        "{hit:?} leaves {width} DIP settings area: {rect:?}"
                    );
                    for (other, _) in l.entries.iter().skip(index + 1) {
                        assert!(
                            rect.intersect(other).is_empty(),
                            "overlapping actions: {hit:?}"
                        );
                    }
                    if matches!(hit, Hit::Options | Hit::Add) {
                        assert!(
                            rect.intersect(&l.title).is_empty(),
                            "header action overlaps title"
                        );
                        assert!(rect.bottom < l.body.top);
                        assert_eq!(
                            l.hit(rect.left + rect.width() / 2.0, rect.top + 16.0),
                            Some(*hit)
                        );
                    } else {
                        s.scroll = (rect.top - l.body.top).clamp(0.0, l.max_scroll());
                        let scrolled = layout(&s, area);
                        let (visible, _) = scrolled.entries.iter().find(|(_, h)| h == hit).unwrap();
                        assert!(
                            visible.top >= scrolled.body.top
                                && visible.bottom <= scrolled.body.bottom,
                            "{hit:?} cannot scroll into view at {width} DIP"
                        );
                        let center_hit =
                            scrolled.hit(visible.left + visible.width() / 2.0, visible.top + 16.0);
                        let expected = if *hit == Hit::Field(2)
                            && scrolled.model_button.is_some_and(|button| {
                                button.contains(
                                    visible.left + visible.width() / 2.0,
                                    visible.top + 16.0,
                                )
                            }) {
                            Some(Hit::Models)
                        } else {
                            Some(*hit)
                        };
                        assert_eq!(center_hit, expected);
                    }
                }
            }
        }
    }

    #[test]
    fn default_sidebars_leave_save_and_edit_inside_editor_at_1024_dip() {
        use super::super::chrome::{Chrome, ChromeState, WorkspaceView};
        let chrome = Chrome::build(
            &ChromeState {
                view: WorkspaceView::Editor,
                ai_panel_open: true,
                ..ChromeState::default()
            },
            Rect::from_size(0.0, 0.0, 1024.0, 768.0),
        );
        let area = chrome.tree.rect(chrome.editor);
        assert_eq!(area.width(), 124.0);
        let mut s = State {
            all: vec![AiProvider::default()],
            form: Some(Form::new(AiProvider::default())),
            ..State::default()
        };
        for target in [Hit::Save, Hit::Cancel, Hit::Edit(0), Hit::Delete(0)] {
            let l = layout(&s, area);
            let (rect, _) = l.entries.iter().find(|(_, hit)| *hit == target).unwrap();
            s.scroll = (s.scroll + rect.top - l.body.top).clamp(0.0, l.max_scroll());
            let l = layout(&s, area);
            let (rect, _) = l.entries.iter().find(|(_, hit)| *hit == target).unwrap();
            let x = rect.left + rect.width() / 2.0;
            let y = rect.top + rect.height() / 2.0;
            assert_eq!(
                chrome.hit(x, y),
                Some(super::super::layout::NodeKey::Editor)
            );
            assert_eq!(l.hit(x, y), Some(target));
        }
    }

    #[test]
    fn options_button_is_clickable_above_scrollable_body() {
        let s = State::default();
        let l = layout(&s, Rect::new(0.0, 0.0, 800.0, 700.0));
        let (r, _) = l.entries.iter().find(|(_, h)| *h == Hit::Options).unwrap();
        assert_eq!(l.hit(r.left + 2.0, r.top + 2.0), Some(Hit::Options));
    }

    #[test]
    fn model_picker_is_a_native_subcontrol_of_the_model_field() {
        let s = State {
            form: Some(Form::new(AiProvider::default())),
            ..State::default()
        };
        let area = Rect::from_size(0.0, 0.0, 800.0, 700.0);
        let l = layout(&s, area);
        let button = l.model_button.expect("model picker button");
        assert_eq!(
            l.hit(button.left + 2.0, button.top + 2.0),
            Some(Hit::Models)
        );
        let field = l.field(2).unwrap();
        assert!(button.intersect(&field).width() > 0.0);
    }

    #[test]
    fn protocol_picker_is_visible_and_separate_from_the_preset_picker() {
        let s = State {
            form: Some(Form::new(AiProvider::default())),
            ..State::default()
        };
        let l = layout(&s, Rect::from_size(0.0, 0.0, 800.0, 700.0));
        let protocol = l
            .entries
            .iter()
            .find(|(_, hit)| *hit == Hit::Protocol)
            .map(|(rect, _)| *rect)
            .expect("protocol picker");
        let preset = l
            .entries
            .iter()
            .find(|(_, hit)| *hit == Hit::Preset)
            .map(|(rect, _)| *rect)
            .expect("preset picker");
        assert!(protocol.intersect(&preset).is_empty());
        assert_eq!(
            l.hit(protocol.left + 4.0, protocol.top + 4.0),
            Some(Hit::Protocol)
        );
    }

    #[test]
    fn password_never_enters_draw_list() {
        let mut s = State {
            form: Some(Form::new(AiProvider {
                id: "a".into(),
                api_key: "SECRET".into(),
                protocol: OPENAI_PROTOCOL.into(),
                ..Default::default()
            })),
            ..State::default()
        };
        let area = Rect::new(0.0, 0.0, 800.0, 900.0);
        let l = layout(&s, area);
        let mut d = DrawList::new();
        paint(
            &mut d,
            area,
            &mut s,
            &l,
            true,
            super::super::theme::tokens().palette(false),
        );
        assert!(!format!("{:?}", d.cmds()).contains("SECRET"));
        assert!(d.finish().is_ok());
    }
}
