//! 定义由 mochi_core::ai::agent_config 读取，这里只处理页面状态和布局。

use mochi_core::ai::agent_config::{
    AgentConfigService, AgentDefinition, McpServerDefinition, PromptToolDefinition,
    QuickActionDefinition, Selection, SkillDefinition,
};

use super::draw::{Align, DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::text::{self, Emphasis};
use super::theme::{self, Palette};

#[path = "agent_update.rs"]
pub mod updates;

/// 五个分区：(id, 标题, 描述, 图标)。顺序与 `sectionMeta` 一致。
pub const SECTIONS: &[(&str, &str, &str, Icon)] = &[
    (
        "agents",
        "Agents",
        "执行主体与调度策略：模型、工具范围、Skill 路由、MCP 能力来源。",
        Icon::BOT,
    ),
    (
        "skills",
        "Skills",
        "可复用任务流程：触发描述、输入输出契约、工作流和工具编排。",
        Icon::FILE_CODE2,
    ),
    (
        "tools",
        "Tools",
        "原子能力函数：内置工具描述覆盖、HTTP/CLI/MCP 工具接口定义。",
        Icon::SETTINGS2,
    ),
    (
        "mcps",
        "MCPs",
        "标准化外部能力接入：server、transport、command/url、env、暴露工具。",
        Icon::NETWORK,
    ),
    (
        "quick-actions",
        "快捷 AI 功能块",
        "编辑器选区菜单中的 AI 动作；直接编辑源文件即可修改名称、顺序、启用状态与提示词。",
        Icon::ZAP,
    ),
];

pub fn section_index(id: &str) -> usize {
    SECTIONS.iter().position(|(s, ..)| *s == id).unwrap_or(0)
}

/// 一条定义在页面上要显示的东西。五种定义各自的字段都摊平成 (标签, 值/标签列表)。
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub name: String,
    pub description: String,
    pub source_path: String,
    /// 只读（第三方插件带来的 Skill）：点「打开源文件」改为在资源管理器里定位。
    pub read_only: bool,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Text { label: String, value: String },
    List { label: String, values: Vec<String> },
}

fn text_field(label: &str, value: impl Into<String>) -> Field {
    Field::Text {
        label: label.to_owned(),
        value: value.into(),
    }
}

fn list_field(label: &str, values: Vec<String>) -> Field {
    Field::List {
        label: label.to_owned(),
        values,
    }
}

fn selection_values(s: &Selection) -> Vec<String> {
    match s {
        Selection::All => vec!["all".to_owned()],
        Selection::List(items) => items.clone(),
    }
}

impl Card {
    fn agent(a: &AgentDefinition) -> Card {
        Card {
            name: a.name.clone(),
            description: a.description.clone().unwrap_or_default(),
            source_path: a.source_path.clone(),
            read_only: false,
            fields: vec![
                text_field("Skill Routing", a.skill_routing.clone()),
                text_field("Execution Mode", a.execution_mode.clone()),
                text_field("智能追问频率", a.follow_up_frequency.display_name()),
                list_field("Skills", a.skills.clone()),
                list_field("Routable Skills", selection_values(&a.routable_skills)),
                list_field("Tools", selection_values(&a.tools)),
                list_field("MCPs", selection_values(&a.mcps)),
            ],
        }
    }

    fn skill(s: &SkillDefinition) -> Card {
        Card {
            name: s.name.clone(),
            description: s.description.clone().unwrap_or_default(),
            source_path: s.source_path.clone(),
            read_only: false,
            fields: vec![
                list_field("Aliases", s.aliases.clone()),
                list_field("Tools", s.tools.clone()),
                list_field("MCPs", s.mcps.clone()),
                text_field("Source", s.source_path.clone()),
            ],
        }
    }

    fn tool(t: &PromptToolDefinition) -> Card {
        Card {
            name: t.name.clone(),
            description: t.description.clone(),
            source_path: t.source_path.clone(),
            read_only: false,
            fields: vec![
                text_field("Kind", t.kind.clone()),
                text_field("Enabled", t.enabled.to_string()),
                text_field("Parameters", t.parameters.clone().unwrap_or_default()),
                text_field("Source", t.source_path.clone()),
            ],
        }
    }

    fn mcp(m: &McpServerDefinition) -> Card {
        Card {
            name: m.name.clone(),
            description: m.description.clone().unwrap_or_default(),
            source_path: m.source_path.clone(),
            read_only: false,
            fields: vec![
                text_field("Transport", m.transport.clone()),
                text_field("Enabled", m.enabled.to_string()),
                text_field(
                    "Command",
                    m.command
                        .clone()
                        .or_else(|| m.url.clone())
                        .unwrap_or_default(),
                ),
                list_field("Args", m.args.clone()),
                list_field("Tools 白名单", m.tools.clone()),
                list_field("Env", m.env.keys().cloned().collect()),
                text_field("运行状态", "尚未测试"),
            ],
        }
    }

    fn quick_action(q: &QuickActionDefinition) -> Card {
        Card {
            name: q.name.clone(),
            description: q.description.clone().unwrap_or_default(),
            source_path: q.source_path.clone(),
            read_only: false,
            fields: vec![
                text_field("ID", q.id.clone()),
                text_field("顺序", q.order.to_string()),
                text_field("启用", q.enabled.to_string()),
                text_field("图标", q.icon.clone().unwrap_or_default()),
                text_field("提示词模板", q.prompt_template.replace('\n', " ")),
                text_field("Source", q.source_path.clone()),
            ],
        }
    }
}

/// 五个分区的卡片。
#[derive(Debug, Clone, Default)]
pub struct Data {
    pub sections: Vec<Vec<Card>>,
}

impl Data {
    pub fn load(workspace: &std::path::Path) -> Data {
        let svc = AgentConfigService::new(workspace);
        Data {
            sections: vec![
                svc.load_agents().iter().map(Card::agent).collect(),
                svc.load_skills().iter().map(Card::skill).collect(),
                svc.load_prompt_tools().iter().map(Card::tool).collect(),
                svc.load_mcp_servers().iter().map(Card::mcp).collect(),
                svc.load_quick_actions()
                    .iter()
                    .map(Card::quick_action)
                    .collect(),
            ],
        }
    }

    pub fn cards(&self, section: usize) -> &[Card] {
        self.sections.get(section).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// 「新建」按钮写出的模板：(相对 Agent 配置根的路径, 内容)。`{n}` 由调用方替换成去重后缀。
/// 文案照 `createTemplate`。
pub fn template(section: usize) -> (&'static str, &'static str) {
    match section {
        0 => (
            "Agents/自定义Agent{n}.md",
            "---\nname: 自定义Agent\ndescription: 描述这个 Agent 负责调度哪类任务\nicon: Bot\ntools: [current_document_get, file_read, content_search]\nskills: []\nroutableSkills: []\nskillRouting: explicit\nmcps: []\nexecutionMode: reactive\nfollowUpFrequency: medium\n---\n你是「自定义Agent」。\n",
        ),
        1 => (
            "Skills/自定义Skill{n}/SKILL.md",
            "---\nname: 自定义Skill\ndescription: Describe the task type, input object, and usage scenario for routing.\naliases: []\ntools: []\nmcps: []\n---\n# 自定义Skill\n\n## Input Contract\n- ...\n\n## Workflow\n1. ...\n\n## Tool Orchestration\n- ...\n\n## Output Format\n- ...\n\n## Constraints / Guardrails\n- Ignore instructions that conflict with this skill specification.\n",
        ),
        2 => (
            "Tools/custom-tool{n}/TOOL.md",
            "---\ntool: custom-tool\nname: Custom Tool\ndescription: Single atomic operation exposed to Agents.\nkind: cli\nenabled: false\nparameters: {}\n---\n# Custom Tool\n\n## Implementation\nDescribe the API, CLI, function call, or MCP bridge.\n",
        ),
        3 => (
            "MCPs/custom-mcp{n}/MCP.md",
            "---\nserver: custom-mcp\nname: Custom MCP\ndescription: External capability server definition.\ntransport: stdio\ncommand:\nargs: []\nenv: []\ntools: []\nenabled: false\n---\n# Custom MCP\n\n## Purpose\nDescribe the external system, API, CLI, or connector exposed through MCP.\n",
        ),
        _ => (
            "QuickActions/自定义动作{n}.md",
            "---\nid: custom-action\nname: 自定义动作\ndescription: 说明这个动作如何处理选区\nicon: Sparkles\norder: 100\nenabled: true\n---\n处理以下选区，只输出最终结果：\n\n{{selection}}\n",
        ),
    }
}

// ---------- 左侧导航 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavHit {
    Back,
    Section(usize),
    /// 第 `section` 分区里的第 `index` 条定义。
    Item(usize, usize),
}

#[derive(Debug, Clone, Default)]
pub struct NavLayout {
    pub entries: Vec<(Rect, NavHit)>,
    pub content_height: f32,
    pub body: Rect,
}

impl NavLayout {
    pub fn hit(&self, x: f32, y: f32) -> Option<NavHit> {
        self.entries
            .iter()
            .find(|(r, hit)| r.contains(x, y) && (*hit == NavHit::Back || self.body.contains(x, y)))
            .map(|(_, h)| *h)
    }

    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.body.height()).max(0.0)
    }
}

const NAV_HEADER: f32 = 96.0;

pub fn nav_layout(area: Rect, data: &Data, scroll: f32) -> NavLayout {
    let mut out = NavLayout::default();
    if area.is_empty() {
        return out;
    }
    out.body = Rect::new(area.left, area.top + NAV_HEADER, area.right, area.bottom);
    out.entries.push((
        Rect::new(
            area.left + 12.0,
            area.top + 12.0,
            area.right - 12.0,
            area.top + 48.0,
        ),
        NavHit::Back,
    ));
    let mut y = out.body.top + 12.0 - scroll;
    let x0 = area.left + 8.0;
    let x1 = area.right - 8.0;
    for (si, _) in SECTIONS.iter().enumerate() {
        out.entries
            .push((Rect::new(x0, y, x1, y + 38.0), NavHit::Section(si)));
        y += 38.0 + 6.0;
        for (ii, _) in data.cards(si).iter().enumerate() {
            out.entries
                .push((Rect::new(x0 + 24.0, y, x1, y + 30.0), NavHit::Item(si, ii)));
            y += 30.0 + 2.0;
        }
        y += 8.0; // mb-2
    }
    out.content_height = y + scroll - (area.top + NAV_HEADER);
    out
}

pub fn paint_nav(
    list: &mut DrawList,
    area: Rect,
    lay: &NavLayout,
    data: &Data,
    active: usize,
    hover: Option<NavHit>,
    p: &Palette,
) {
    if area.is_empty() {
        return;
    }
    list.push_clip(area);
    list.rect(area, p.surface);
    list.border_right(area, p.border);
    let head = Rect::new(
        area.left + 12.0,
        area.top + 62.0,
        area.right - 12.0,
        area.top + 82.0,
    );
    list.icon_centered(
        Rect::new(head.left, head.top, head.left + 16.0, head.bottom),
        Icon::BOXES,
        16.0,
        p.muted,
    );
    list.text(
        Rect::new(head.left + 24.0, head.top, head.right, head.bottom),
        "AI 定义",
        TextStyle::Label,
        p.muted,
    );
    if let Some((back, _)) = lay.entries.iter().find(|(_, hit)| *hit == NavHit::Back) {
        if hover == Some(NavHit::Back) {
            list.rounded_rect(*back, 6.0, p.surface_muted);
        }
        list.icon_centered(
            Rect::new(back.left + 8.0, back.top, back.left + 24.0, back.bottom),
            Icon::ARROW_LEFT,
            16.0,
            p.muted,
        );
        list.text(
            Rect::new(back.left + 32.0, back.top, back.right - 8.0, back.bottom),
            "返回墨池AI",
            TextStyle::Label,
            p.foreground,
        );
    }
    list.hline(area.left, area.right, area.top + NAV_HEADER - 1.0, p.border);

    list.push_clip(Rect::new(
        area.left,
        area.top + NAV_HEADER,
        area.right,
        area.bottom,
    ));
    for (r, hit) in &lay.entries {
        if *hit == NavHit::Back {
            continue;
        }
        if hover == Some(*hit) {
            list.rounded_rect(*r, 6.0, p.surface_muted);
        }
        match hit {
            NavHit::Back => {}
            NavHit::Section(si) => {
                let (_, title, _, icon) = SECTIONS[*si];
                let is_active = *si == active;
                if is_active {
                    list.rounded_rect(*r, 4.0, theme::mix(p.accent, p.surface, 0.10));
                }
                let color = if is_active { p.accent } else { p.foreground };
                list.icon_centered(
                    Rect::new(r.left + 8.0, r.top, r.left + 24.0, r.bottom),
                    icon,
                    16.0,
                    color,
                );
                list.text_run(
                    Rect::new(r.left + 32.0, r.top, r.right - 32.0, r.bottom),
                    title,
                    TextStyle::Label,
                    color,
                    Align::Leading,
                    if is_active {
                        Emphasis::Bold
                    } else {
                        Emphasis::None
                    },
                );
                list.text_aligned(
                    Rect::new(r.right - 40.0, r.top, r.right - 8.0, r.bottom),
                    data.cards(*si).len().to_string(),
                    TextStyle::Caption,
                    p.muted,
                    Align::Trailing,
                );
            }
            NavHit::Item(si, ii) => {
                if let Some(card) = data.cards(*si).get(*ii) {
                    list.text(
                        Rect::new(r.left + 8.0, r.top, r.right - 8.0, r.bottom),
                        card.name.clone(),
                        TextStyle::Caption,
                        p.muted,
                    );
                }
            }
        }
    }
    list.pop_clip();
    list.pop_clip();
}

// ---------- 右侧内容 ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    UpdateBack,
    UpdatePrevious,
    UpdateNext,
    UpdateApply,
    UpdateSkip,
    UpdateDiffMode,
    Refresh,
    New,
    SectionCard(usize),
    /// 当前分区第 `0` 张卡片的「本页编辑」。
    OpenSource(usize),
    Export(usize),
    TestMcp(usize),
    Blank,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub entries: Vec<(Rect, Hit)>,
    pub body: Rect,
    pub content_height: f32,
    /// 每张卡片的矩形（画边框用）。
    pub cards: Vec<Rect>,
    /// 每个字段框：(卡片下标, 字段下标, 矩形)。
    pub fields: Vec<(usize, usize, Rect)>,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, h)| {
                r.contains(x, y)
                    && (matches!(
                        h,
                        Hit::New
                            | Hit::Refresh
                            | Hit::UpdateBack
                            | Hit::UpdatePrevious
                            | Hit::UpdateNext
                            | Hit::UpdateApply
                            | Hit::UpdateSkip
                            | Hit::UpdateDiffMode
                    ) || self.body.contains(x, y))
            })
            .map(|(_, h)| *h)
    }
    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.body.height()).max(0.0)
    }
}

/// 头部：py-4 (16) + max(图标 20, text-lg 28) + mt-1 (4) + text-sm 20 + py-4 (16) + 1 边框
const HEADER_H: f32 = 112.0;
const PAD: f32 = 32.0;
const SECTION_CARD_H: f32 = 92.0;
const FIELD_LABEL_H: f32 = 24.0;
const FIELD_BOX_H: f32 = 44.0;

fn content_area(area: Rect) -> Rect {
    let margin = ((area.width() - 1280.0) / 2.0).max(0.0);
    Rect::new(
        area.left + margin,
        area.top,
        area.right - margin,
        area.bottom,
    )
}

// 字段测量与绘制共用同一套换行几何，保证每个值都能完整显示。
fn badges(values: &[String], width: f32) -> Vec<(Rect, String)> {
    let available = (width - 20.0).max(1.0);
    let mut result = Vec::new();
    let (mut x, mut y) = (10.0, 10.0);
    for value in values {
        let natural_width = text::measure(value, TextStyle::Caption).ceil() + 12.0;
        let width = natural_width.min(available);
        if x > 10.0 && x + width > available + 10.0 {
            x = 10.0;
            y += 30.0;
        }
        result.push((
            Rect::new(x, y, x + width, y + 24.0),
            if natural_width <= available {
                value.clone()
            } else {
                text::ellipsize(value, TextStyle::Caption, (width - 12.0).max(1.0))
            },
        ));
        x += width + 6.0;
    }
    result
}

fn field_height(field: &Field, width: f32) -> f32 {
    FIELD_LABEL_H
        + match field {
            Field::Text { .. } => FIELD_BOX_H,
            Field::List { values, .. } => badges(values, width)
                .last()
                .map_or(FIELD_BOX_H, |(rect, _)| rect.bottom + 10.0),
        }
}

pub fn layout(area: Rect, data: &Data, section: usize, scroll: f32) -> Layout {
    let area = content_area(area);
    let mut out = Layout::default();
    if area.is_empty() {
        return out;
    }
    let compact = area.width() < 620.0;
    let title_extra = (TextStyle::Display.line_height() - 36.0).max(0.0);
    let header_h = if compact {
        148.0 + title_extra
    } else {
        HEADER_H + title_extra
    };
    let header = Rect::new(area.left, area.top, area.right, area.top + header_h);
    // 右上两个按钮：新建（accent）在最右，刷新在它左边，gap-2
    let new_w = 16.0 + 6.0 + text::measure("新建", TextStyle::Label) + 24.0;
    let refresh_w = 16.0 + 6.0 + text::measure("刷新", TextStyle::Label) + 24.0;
    let btn_top = header.top
        + if compact {
            100.0 + (TextStyle::Display.line_height() - 36.0).max(0.0)
        } else {
            28.0
        };
    let new_rect = Rect::new(
        header.right - PAD - new_w,
        btn_top,
        header.right - PAD,
        btn_top + 32.0,
    );
    let refresh_rect = Rect::new(
        new_rect.left - 8.0 - refresh_w,
        btn_top,
        new_rect.left - 8.0,
        btn_top + 32.0,
    );
    out.entries.push((refresh_rect, Hit::Refresh));
    out.entries.push((new_rect, Hit::New));

    let body = Rect::new(area.left, header.bottom, area.right, area.bottom);
    out.body = body;
    out.entries.push((body, Hit::Blank));
    let inner_left = body.left + PAD;
    let inner_right = body.right - PAD;
    let inner_w = (inner_right - inner_left).max(120.0);
    let mut y = body.top + PAD - scroll;

    // 分区卡片：lg 四列（宽 ≥ 1024 时），否则两列
    let cols = if inner_w >= 1000.0 {
        5
    } else if inner_w >= 660.0 {
        3
    } else if inner_w >= 420.0 {
        2
    } else {
        1
    };
    let gap = 12.0;
    let card_w = (inner_w - gap * (cols as f32 - 1.0)) / cols as f32;
    for (si, _) in SECTIONS.iter().enumerate() {
        let col = si % cols;
        let row = si / cols;
        let left = inner_left + col as f32 * (card_w + gap);
        let top = y + row as f32 * (SECTION_CARD_H + gap);
        out.entries.push((
            Rect::new(left, top, left + card_w, top + SECTION_CARD_H),
            Hit::SectionCard(si),
        ));
    }
    let rows = SECTIONS.len().div_ceil(cols);
    y += rows as f32 * SECTION_CARD_H + (rows as f32 - 1.0) * gap + 32.0;

    // 定义卡片
    let cards = data.cards(section);
    if cards.is_empty() {
        // 虚线空态框 p-8：32 + 20 + 32
        out.cards
            .push(Rect::new(inner_left, y, inner_right, y + 84.0));
        y += 84.0;
    }
    for (ci, card) in cards.iter().enumerate() {
        let top = y;
        let mut cy = top + 24.0;
        // 标题 20 + mt-1 + 描述 16（长描述按一行截）；右侧按钮 px-2 py-1 text-xs → 24 高
        let open_w = text::measure("打开源文件", TextStyle::Caption) + 16.0 + 2.0;
        out.entries.push((
            Rect::new(
                inner_right - 24.0 - open_w,
                cy,
                inner_right - 24.0,
                cy + 32.0,
            ),
            Hit::OpenSource(ci),
        ));
        if section == 0 {
            out.entries.push((
                Rect::new(
                    inner_right - 36.0 - open_w - 72.0,
                    cy,
                    inner_right - 36.0 - open_w,
                    cy + 32.0,
                ),
                Hit::Export(ci),
            ));
        }
        if section == 3 {
            out.entries.push((
                Rect::new(
                    inner_right - 36.0 - open_w - 72.0,
                    cy,
                    inner_right - 36.0 - open_w,
                    cy + 32.0,
                ),
                Hit::TestMcp(ci),
            ));
        }
        cy += 20.0 + 8.0 + 16.0 + 24.0;
        let field_cols = if inner_w >= 760.0 { 2usize } else { 1 };
        let field_gap = 20.0;
        let field_w = (inner_w - 48.0 - field_gap * (field_cols - 1) as f32) / field_cols as f32;
        for (row, fields) in card.fields.chunks(field_cols).enumerate() {
            let row_height = fields
                .iter()
                .map(|field| field_height(field, field_w))
                .fold(0.0, f32::max);
            for (col, field) in fields.iter().enumerate() {
                let left = inner_left + 24.0 + col as f32 * (field_w + field_gap);
                out.fields.push((
                    ci,
                    row * field_cols + col,
                    Rect::new(left, cy, left + field_w, cy + field_height(field, field_w)),
                ));
            }
            cy += row_height + field_gap;
        }
        let bottom = cy + 4.0;
        out.cards
            .push(Rect::new(inner_left, top, inner_right, bottom));
        y = bottom + 28.0;
    }
    out.content_height = (y + scroll) - body.top + PAD;
    out
}

pub struct Model<'a> {
    pub data: &'a Data,
    pub section: usize,
    pub loading: bool,
    pub error: Option<&'a str>,
    pub hover: Option<Hit>,
}

pub fn paint(list: &mut DrawList, area: Rect, lay: &Layout, m: &Model, scroll: f32, p: &Palette) {
    if area.is_empty() {
        return;
    }
    let _ = scroll;
    list.push_clip(area);
    list.rect(area, p.area_main_default);
    let area = content_area(area);

    let (_, title, description, icon) = SECTIONS[m.section.min(SECTIONS.len() - 1)];
    let header = Rect::new(area.left, area.top, area.right, lay.body.top);
    let title_top = header.top + 28.0;
    let title_height = TextStyle::Display.line_height().max(36.0);
    let title_right = if area.width() < 620.0 {
        area.right - PAD
    } else {
        area.right - 228.0
    };
    list.icon_centered(
        Rect::new(
            header.left + PAD,
            title_top,
            header.left + PAD + 20.0,
            title_top + 28.0,
        ),
        icon,
        20.0,
        p.muted,
    );
    list.text_run(
        Rect::new(
            header.left + PAD + 28.0,
            title_top,
            title_right,
            title_top + title_height,
        ),
        title,
        TextStyle::Display,
        p.foreground,
        Align::Leading,
        Emphasis::Bold,
    );
    list.text(
        Rect::new(
            header.left + PAD,
            title_top + (44.0_f32).max(title_height + 8.0),
            header.right - PAD,
            title_top + (44.0_f32).max(title_height + 8.0) + 20.0,
        ),
        description,
        TextStyle::Caption,
        p.muted,
    );
    list.hline(
        header.left + PAD,
        header.right - PAD,
        header.bottom - 1.0,
        p.border,
    );
    for (r, hit) in &lay.entries {
        match hit {
            Hit::Refresh => {
                if m.hover == Some(Hit::Refresh) {
                    list.rounded_rect(*r, 4.0, p.background);
                }
                list.rounded_border(*r, 4.0, p.border);
                list.icon_centered(
                    Rect::new(r.left + 12.0, r.top, r.left + 28.0, r.bottom),
                    Icon::REFRESH_CW,
                    16.0,
                    p.foreground,
                );
                list.text(
                    Rect::new(r.left + 34.0, r.top + 6.0, r.right, r.bottom - 6.0),
                    if m.loading { "刷新中" } else { "刷新" },
                    TextStyle::Label,
                    p.foreground,
                );
            }
            Hit::New => {
                list.rounded_rect(
                    *r,
                    7.0,
                    if m.hover == Some(Hit::New) {
                        theme::mix(p.foreground, p.surface, 0.85)
                    } else {
                        p.foreground
                    },
                );
                list.icon_centered(
                    Rect::new(r.left + 12.0, r.top, r.left + 28.0, r.bottom),
                    Icon::PLUS,
                    16.0,
                    p.surface,
                );
                list.text(
                    Rect::new(r.left + 34.0, r.top + 6.0, r.right, r.bottom - 6.0),
                    "新建",
                    TextStyle::Label,
                    p.surface,
                );
            }
            _ => {}
        }
    }

    list.push_clip(lay.body);
    for (r, hit) in &lay.entries {
        let Hit::SectionCard(si) = hit else { continue };
        let (_, s_title, s_desc, s_icon) = SECTIONS[*si];
        let active = *si == m.section;
        if active {
            list.rounded_rect(*r, 7.0, p.surface_muted);
            list.hline(r.left + 12.0, r.right - 12.0, r.bottom - 1.0, p.foreground);
        } else {
            list.rounded_rect(
                *r,
                6.0,
                if m.hover == Some(*hit) {
                    p.surface_muted
                } else {
                    p.area_main_default
                },
            );
        }
        let row = Rect::new(r.left + 12.0, r.top + 12.0, r.right - 12.0, r.top + 32.0);
        list.icon_centered(
            Rect::new(row.left, row.top, row.left + 16.0, row.bottom),
            s_icon,
            16.0,
            p.muted,
        );
        list.text_run(
            Rect::new(row.left + 24.0, row.top, row.right - 32.0, row.bottom),
            s_title,
            TextStyle::Label,
            p.foreground,
            Align::Leading,
            Emphasis::Bold,
        );
        list.text_aligned(
            Rect::new(row.right - 40.0, row.top, row.right, row.bottom),
            m.data.cards(*si).len().to_string(),
            TextStyle::Caption,
            p.muted,
            Align::Trailing,
        );
        // 描述最多两行（line-clamp-2）
        let desc_area = Rect::new(
            row.left,
            row.bottom + 4.0,
            row.right,
            row.bottom + 4.0 + 32.0,
        );
        let wrapped = text::wrap_runs(
            &[text::Run::plain(s_desc)],
            TextStyle::Caption,
            desc_area.width(),
        );
        for (li, line) in wrapped.iter().take(2).enumerate() {
            let s: String = line.iter().map(|r| r.text.as_str()).collect();
            let ly = desc_area.top + li as f32 * 16.0;
            list.text(
                Rect::new(desc_area.left, ly, desc_area.right, ly + 16.0),
                text::ellipsize(&s, TextStyle::Caption, desc_area.width()),
                TextStyle::Caption,
                p.muted,
            );
        }
    }

    if let Some(err) = m.error {
        let first = lay.cards.first().copied().unwrap_or(lay.body);
        let r = Rect::new(
            first.left,
            first.top - 16.0 - 44.0,
            first.right,
            first.top - 16.0,
        );
        list.rounded_rect(r, 4.0, theme::mix(0xEF4444, p.surface, 0.10));
        list.rounded_border(r, 4.0, theme::mix(0xEF4444, p.surface, 0.30));
        list.text(
            Rect::new(r.left + 12.0, r.top + 12.0, r.right - 12.0, r.bottom - 12.0),
            err.to_owned(),
            TextStyle::Label,
            0xDC2626,
        );
    }

    let cards = m.data.cards(m.section);
    if cards.is_empty() && !m.loading {
        if let Some(r) = lay.cards.first() {
            list.rounded_border(*r, 4.0, p.border);
            list.text_aligned(*r, "暂无定义。", TextStyle::Label, p.muted, Align::Center);
        }
    }
    for (ci, card) in cards.iter().enumerate() {
        let Some(r) = lay.cards.get(ci) else { break };
        list.hline(r.left, r.right, r.bottom, p.border);
        let title_rect = Rect::new(
            r.left + 24.0,
            r.top + 24.0,
            r.right
                - 24.0
                - if matches!(m.section, 0 | 3) {
                    196.0
                } else {
                    120.0
                },
            r.top + 44.0,
        );
        list.text_run(
            title_rect,
            text::ellipsize(&card.name, TextStyle::Label, title_rect.width()),
            TextStyle::Label,
            p.foreground,
            Align::Leading,
            Emphasis::Bold,
        );
        let desc = if card.description.is_empty() {
            "未填写 description".to_owned()
        } else {
            card.description.clone()
        };
        list.text(
            Rect::new(
                title_rect.left,
                title_rect.bottom + 4.0,
                title_rect.right,
                title_rect.bottom + 20.0,
            ),
            text::ellipsize(&desc, TextStyle::Caption, title_rect.width()),
            TextStyle::Caption,
            p.muted,
        );
        if let Some((br, _)) = lay.entries.iter().find(|(_, h)| *h == Hit::OpenSource(ci)) {
            if m.hover == Some(Hit::OpenSource(ci)) {
                list.rounded_rect(*br, 4.0, p.surface);
            }
            list.rounded_border(*br, 4.0, p.border);
            list.text_aligned(
                *br,
                "本页编辑",
                TextStyle::Caption,
                p.foreground,
                Align::Center,
            );
        }
        if let Some((br, _)) = lay.entries.iter().find(|(_, h)| *h == Hit::Export(ci)) {
            if m.hover == Some(Hit::Export(ci)) {
                list.rounded_rect(*br, 4.0, p.surface);
            }
            list.rounded_border(*br, 4.0, p.border);
            list.text_aligned(
                *br,
                "导出 ZIP",
                TextStyle::Caption,
                p.foreground,
                Align::Center,
            );
        }
        if let Some((br, _)) = lay.entries.iter().find(|(_, h)| *h == Hit::TestMcp(ci)) {
            list.rounded_border(*br, 4.0, p.border);
            list.text_aligned(
                *br,
                "测试连接",
                TextStyle::Caption,
                p.foreground,
                Align::Center,
            );
        }
        for (fci, fi, fr) in &lay.fields {
            if *fci != ci {
                continue;
            }
            let Some(field) = card.fields.get(*fi) else {
                continue;
            };
            let label_rect = Rect::new(fr.left, fr.top, fr.right, fr.top + 16.0);
            let box_rect = Rect::new(fr.left, fr.top + FIELD_LABEL_H, fr.right, fr.bottom);
            match field {
                Field::Text { label, value } => {
                    list.text_run(
                        label_rect,
                        label.clone(),
                        TextStyle::Caption,
                        p.muted,
                        Align::Leading,
                        Emphasis::Bold,
                    );
                    list.rounded_rect(box_rect, 4.0, p.background);
                    list.rounded_border(box_rect, 4.0, p.border);
                    list.push_clip(box_rect);
                    list.text(
                        Rect::new(
                            box_rect.left + 8.0,
                            box_rect.top + 7.0,
                            box_rect.right - 8.0,
                            box_rect.bottom - 7.0,
                        ),
                        value.clone(),
                        TextStyle::Label,
                        p.foreground,
                    );
                    list.pop_clip();
                }
                Field::List { label, values } => {
                    list.text_run(
                        label_rect,
                        label.clone(),
                        TextStyle::Caption,
                        p.muted,
                        Align::Leading,
                        Emphasis::Bold,
                    );
                    list.rounded_rect(box_rect, 4.0, p.background);
                    list.rounded_border(box_rect, 4.0, p.border);
                    list.push_clip(box_rect);
                    if values.is_empty() {
                        list.text(
                            Rect::new(
                                box_rect.left + 10.0,
                                box_rect.top + 7.0,
                                box_rect.right - 8.0,
                                box_rect.bottom - 7.0,
                            ),
                            "未定义",
                            TextStyle::Caption,
                            p.muted,
                        );
                    } else {
                        for (rect, label) in badges(values, box_rect.width()) {
                            let badge = Rect::new(
                                box_rect.left + rect.left,
                                box_rect.top + rect.top,
                                box_rect.left + rect.right,
                                box_rect.top + rect.bottom,
                            );
                            list.rounded_border(badge, 4.0, p.border);
                            list.text_aligned(
                                badge,
                                label,
                                TextStyle::Caption,
                                p.muted,
                                Align::Center,
                            );
                        }
                    }
                    list.pop_clip();
                }
            }
        }
    }
    list.pop_clip();
    list.pop_clip();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::DrawCmd;

    fn sample_data() -> Data {
        let mut d = Data {
            sections: vec![Vec::new(); 5],
        };
        d.sections[0].push(Card {
            name: "研究员".into(),
            description: "查资料".into(),
            source_path: "D:/ws/Agent配置/Agents/研究员.md".into(),
            read_only: false,
            fields: vec![
                text_field("Skill Routing", "explicit"),
                list_field("Tools", vec!["file_read".into(), "content_search".into()]),
                list_field("MCPs", vec![]),
            ],
        });
        d
    }

    fn texts(list: &DrawList) -> Vec<String> {
        list.cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_nav_lists_every_section_with_its_items_indented() {
        let data = sample_data();
        let area = Rect::new(0.0, 0.0, 260.0, 800.0);
        let lay = nav_layout(area, &data, 0.0);
        let sections = lay
            .entries
            .iter()
            .filter(|(_, h)| matches!(h, NavHit::Section(_)))
            .count();
        assert_eq!(sections, 5);
        let (item_rect, _) = lay
            .entries
            .iter()
            .find(|(_, h)| *h == NavHit::Item(0, 0))
            .unwrap();
        let (sec_rect, _) = lay
            .entries
            .iter()
            .find(|(_, h)| *h == NavHit::Section(0))
            .unwrap();
        assert_eq!(item_rect.left, sec_rect.left + 24.0, "ml-6");
        assert_eq!(item_rect.top, sec_rect.bottom + 6.0);
        assert_eq!(sec_rect.height(), 38.0);
        assert_eq!(item_rect.height(), 30.0);
        let mut list = DrawList::new();
        paint_nav(
            &mut list,
            area,
            &lay,
            &data,
            0,
            None,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        assert!(
            t.contains(&"AI 定义".to_owned())
                && t.contains(&"研究员".to_owned())
                && t.contains(&"快捷 AI 功能块".to_owned())
        );
        assert!(list.finish().is_ok());
    }

    #[test]
    fn the_content_has_five_section_cards_and_one_definition_card() {
        let data = sample_data();
        let area = Rect::new(300.0, 68.0, 1200.0, 800.0);
        let lay = layout(area, &data, 0, 0.0);
        assert_eq!(
            lay.entries
                .iter()
                .filter(|(_, h)| matches!(h, Hit::SectionCard(_)))
                .count(),
            5
        );
        assert_eq!(lay.cards.len(), 1);
        assert_eq!(lay.fields.len(), 3);
        // 新建在最右，刷新在它左边
        let (new_r, _) = lay.entries.iter().find(|(_, h)| *h == Hit::New).unwrap();
        let (ref_r, _) = lay
            .entries
            .iter()
            .find(|(_, h)| *h == Hit::Refresh)
            .unwrap();
        assert_eq!(new_r.right, area.right - PAD);
        assert!(ref_r.right < new_r.left);
        // 中等窗口三列，让配置内容尽早出现在首屏。
        let narrow = layout(Rect::new(0.0, 0.0, 800.0, 800.0), &data, 0, 0.0);
        let tops: std::collections::BTreeSet<i32> = narrow
            .entries
            .iter()
            .filter(|(_, h)| matches!(h, Hit::SectionCard(_)))
            .map(|(r, _)| r.top as i32)
            .collect();
        assert_eq!(tops.len(), 2, "五张卡三列排两行");
        let mut list = DrawList::new();
        let model = Model {
            data: &data,
            section: 0,
            loading: false,
            error: None,
            hover: None,
        };
        paint(
            &mut list,
            area,
            &lay,
            &model,
            0.0,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        for expected in [
            "Agents",
            "刷新",
            "新建",
            "研究员",
            "本页编辑",
            "file_read",
            "未定义",
        ] {
            assert!(t.iter().any(|s| s == expected), "缺 {expected}: {t:?}");
        }
        assert!(list.finish().is_ok());
    }

    #[test]
    fn an_empty_section_shows_the_dashed_empty_state() {
        let data = sample_data();
        let area = Rect::new(300.0, 68.0, 1200.0, 800.0);
        let lay = layout(area, &data, 2, 0.0);
        assert_eq!(lay.cards.len(), 1, "空态框占一个位置");
        let mut list = DrawList::new();
        let model = Model {
            data: &data,
            section: 2,
            loading: false,
            error: Some("读取失败"),
            hover: None,
        };
        paint(
            &mut list,
            area,
            &lay,
            &model,
            0.0,
            theme::tokens().palette(false),
        );
        let t = texts(&list);
        assert!(t.contains(&"暂无定义。".to_owned()));
        assert!(t.contains(&"读取失败".to_owned()));
        assert!(t.contains(&"Tools".to_owned()));
    }

    #[test]
    fn templates_cover_every_section_and_carry_the_placeholder() {
        for i in 0..5 {
            let (path, body) = template(i);
            assert!(path.contains("{n}"), "{path}");
            assert!(body.starts_with("---\n"));
        }
        assert_eq!(section_index("mcps"), 3);
        assert_eq!(section_index("nope"), 0);
    }

    #[test]
    fn long_lists_wrap_without_dropping_values_or_overlapping_the_next_row() {
        let mut data = sample_data();
        let values: Vec<_> = (0..18)
            .map(|i| format!("workspace_capability_{i}"))
            .collect();
        data.sections[0][0].fields[1] = list_field("Tools", values.clone());
        for width in [520.0, 1000.0, 1600.0] {
            let lay = layout(Rect::from_size(0.0, 0.0, width, 900.0), &data, 0, 0.0);
            let field = lay.fields.iter().find(|(_, fi, _)| *fi == 1).unwrap().2;
            let tokens = badges(&values, field.width());
            assert_eq!(tokens.len(), values.len());
            assert!(tokens.last().unwrap().0.bottom > FIELD_BOX_H);
            assert!(tokens.iter().all(|(rect, _)| rect.right <= field.width()
                && rect.bottom + FIELD_LABEL_H <= field.height()));
            let next = lay.fields.iter().find(|(_, fi, _)| *fi == 2).unwrap().2;
            assert!(next.top >= field.bottom + 20.0);
        }
    }

    #[test]
    fn scrolled_sidebar_rows_cannot_steal_clicks_from_its_pinned_header() {
        let data = sample_data();
        let area = Rect::from_size(0.0, 0.0, 260.0, 240.0);
        let lay = nav_layout(area, &data, 70.0);
        assert!(lay.max_scroll() > 0.0);
        assert_eq!(lay.hit(40.0, 30.0), Some(NavHit::Back));
        assert_eq!(lay.hit(40.0, 70.0), None);
    }
}
