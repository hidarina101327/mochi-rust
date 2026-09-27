//! 配置语义以 aiPrompts.ts 为准。空工具列表表示全部；* / all 只对 mcps 和 routableSkills 有效。
//! 通用助手的能力上限跟随内置工具目录；运行时通过 Skills 渐进披露。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::files::locale_compare;
use crate::paths;

#[path = "agent_consolidation.rs"]
mod consolidation;
#[path = "skill_catalog.rs"]
pub mod skill_catalog;
#[path = "agent_updates.rs"]
pub mod updates;

pub const GENERAL_ASSISTANT_ID: &str = "通用助手";
pub const WEB_RESEARCH_ASSISTANT_ID: &str = "联网研究助手";

/// 通用助手首轮必备的发现工具；领域工具在 Skill 加载后才披露。
pub const REQUIRED_GENERAL_ASSISTANT_TOOLS: &[&str] = super::tools::skill_tools::INITIAL;

/// 工具/MCP/可路由技能的选择：要么全部，要么一份显式清单。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    All,
    List(Vec<String>),
}

impl Selection {
    pub fn is_all(&self) -> bool {
        matches!(self, Self::All)
    }
    pub fn contains(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::List(items) => items.iter().any(|i| i == name),
        }
    }
    pub fn as_list(&self) -> &[String] {
        match self {
            Self::All => &[],
            Self::List(items) => items,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub tools: Selection,
    pub skills: Vec<String>,
    pub routable_skills: Selection,
    /// `none` | `explicit` | `light` | `progressive`
    pub skill_routing: String,
    pub mcps: Selection,
    pub model: Option<String>,
    /// `reactive` | `plan-and-execute`
    pub execution_mode: String,
    pub max_turns: Option<i64>,
    pub follow_up_frequency: super::follow_up::FollowUpFrequency,
    pub system_prompt: String,
    pub source_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub tools: Vec<String>,
    pub mcps: Vec<String>,
    pub body: String,
    pub source_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptToolDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `builtin` | `http` | `cli` | `mcp`
    pub kind: String,
    pub enabled: bool,
    pub parameters: Option<String>,
    pub implementation: String,
    pub source_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpServerDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// `stdio` | `http` | `sse`
    pub transport: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub env: BTreeMap<String, String>,
    pub tools: Vec<String>,
    pub enabled: bool,
    pub source_path: String,
}

/// 种子元数据用来表示「适配器已在 Mochi 中注册」的保留命令。
/// 它不是可执行的 MCP 进程。
pub const BUILTIN_MCP_COMMAND: &str = "mochi-internal";

impl McpServerDefinition {
    pub fn is_builtin(&self) -> bool {
        self.command
            .as_deref()
            .is_some_and(|command| command.trim() == BUILTIN_MCP_COMMAND)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuickActionDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub order: i64,
    pub enabled: bool,
    /// `replace` | `append`
    pub apply: String,
    pub system_prompt: Option<String>,
    pub prompt_template: String,
    pub source_path: String,
}

// ---------- 文档头部 ----------

/// 一个文档头部字段的值：可以是单个值或列表。使用有序映射以保留原有顺序（写回时需要）。
#[derive(Debug, Clone, PartialEq)]
pub enum FmValue {
    Scalar(String),
    List(Vec<String>),
}

pub type Frontmatter = Vec<(String, FmValue)>;

fn fm_get<'a>(data: &'a Frontmatter, key: &str) -> Option<&'a FmValue> {
    data.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// 标量取值。空串视为不存在（与 TS 的 `typeof value === 'string' && value` 一致）。
fn fm_string(data: &Frontmatter, key: &str) -> Option<String> {
    match fm_get(data, key) {
        Some(FmValue::Scalar(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// 列表取值。标量非空时包装成单元素列表（TS 行为）。
fn fm_array(data: &Frontmatter, key: &str) -> Vec<String> {
    match fm_get(data, key) {
        Some(FmValue::List(items)) => items.clone(),
        Some(FmValue::Scalar(s)) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn fm_bool(data: &Frontmatter, key: &str, fallback: bool) -> bool {
    match fm_string(data, key).map(|s| s.to_lowercase()) {
        Some(v) if v == "true" || v == "yes" || v == "1" => true,
        Some(v) if v == "false" || v == "no" || v == "0" => false,
        _ => fallback,
    }
}

fn fm_number(data: &Frontmatter, key: &str) -> Option<i64> {
    let raw = fm_string(data, key)?;
    // TS 用 Number()，"12.7" 会得到 12.7；这里的调用点都只取整数语义
    raw.trim()
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .map(|n| n as i64)
}

fn fm_choice(data: &Frontmatter, key: &str, choices: &[&str], fallback: &str) -> String {
    match fm_string(data, key) {
        Some(v) if choices.contains(&v.as_str()) => v,
        _ => fallback.to_owned(),
    }
}

/// `key: [a=1, b=2]` → map。非 `k=v` 形式的项跳过。
fn fm_key_value(data: &Frontmatter, key: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for item in fm_array(data, key) {
        if let Some(idx) = item.find('=') {
            if idx > 0 {
                out.insert(
                    item[..idx].trim().to_owned(),
                    item[idx + 1..].trim().to_owned(),
                );
            }
        }
    }
    out
}

/// 简易的文档头部解析：支持 `key: value` 和 `key: [a, b]`。
///
/// 没有文档头部时，返回空数据和**原始全文**（不是去掉头部后的正文），与 TS 保持一致。
pub fn parse_frontmatter(raw: &str) -> (Frontmatter, String) {
    // 去掉可能的 BOM
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw);

    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (Vec::new(), raw.to_owned());
    };
    // 找结束分隔符：行首的 `---`
    let Some((head, body)) = split_at_closing_fence(rest) else {
        return (Vec::new(), raw.to_owned());
    };

    let mut data: Frontmatter = Vec::new();
    for line in head.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)) {
        let Some(idx) = line.find(':') else { continue };
        let key = line[..idx].trim();
        if key.is_empty() {
            continue;
        }
        let value = line[idx + 1..].trim();
        let parsed = if value.starts_with('[') && value.ends_with(']') {
            FmValue::List(
                value[1..value.len() - 1]
                    .split(',')
                    .map(|s| s.trim().trim_matches(['\'', '"']).to_owned())
                    .filter(|s| !s.is_empty())
                    .collect(),
            )
        } else {
            FmValue::Scalar(value.trim_matches(['\'', '"']).to_owned())
        };
        data.push((key.to_owned(), parsed));
    }
    (data, body)
}

/// 从文档头部的起始分隔线后查找结束的 `---` 行，并返回（头部、正文）。
fn split_at_closing_fence(rest: &str) -> Option<(String, String)> {
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = trimmed.strip_suffix('\r').unwrap_or(trimmed);
        if trimmed == "---" {
            let head = rest[..offset].trim_end_matches(['\n', '\r']).to_owned();
            let body = rest[offset + line.len()..].to_owned();
            return Some((head, body));
        }
        offset += line.len();
    }
    // 文件以 `---` 结尾且不带换行
    let trimmed = rest.trim_end();
    if trimmed.ends_with("---") {
        let head_end = trimmed.len() - 3;
        return Some((
            rest[..head_end].trim_end_matches(['\n', '\r']).to_owned(),
            String::new(),
        ));
    }
    None
}

// ---------- 目录读取 ----------

struct MarkdownDoc {
    name: String,
    path: PathBuf,
    raw: String,
}

/// 读一个目录下的所有 `.md`（不递归）。读不了的条目跳过，不让一个坏文件毁掉整个列表。
fn read_markdown_dir(dir: &Path) -> Vec<MarkdownDoc> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<MarkdownDoc> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .to_lowercase()
                .ends_with(".md")
        })
        .filter_map(|e| {
            let path = e.path();
            let raw = std::fs::read_to_string(&path).ok()?;
            Some(MarkdownDoc {
                name: e.file_name().to_string_lossy().into_owned(),
                path,
                raw,
            })
        })
        .collect();
    out.sort_by(|a, b| locale_compare(&a.name, &b.name));
    out
}

/// 读 `<dir>/<子目录>/<file_name>` 形式的定义（Skills/Tools/MCPs 都是这个布局）。
fn read_subdir_docs(dir: &Path, file_name: &str) -> Vec<MarkdownDoc> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    dirs.sort_by(|a, b| locale_compare(&a.0, &b.0));

    dirs.into_iter()
        .filter_map(|(name, path)| {
            let doc_path = path.join(file_name);
            let raw = std::fs::read_to_string(&doc_path).ok()?;
            Some(MarkdownDoc {
                name,
                path: doc_path,
                raw,
            })
        })
        .collect()
}

fn forward(path: &Path) -> String {
    paths::to_forward_slashes(&path.to_string_lossy())
}

fn stem(file_name: &str) -> String {
    match file_name.to_lowercase().strip_suffix(".md") {
        Some(_) => file_name[..file_name.len() - 3].to_owned(),
        None => file_name.to_owned(),
    }
}

// ---------- 加载器 ----------

pub struct AgentConfigService {
    workspace_path: PathBuf,
}

impl AgentConfigService {
    pub fn new(workspace_path: impl AsRef<Path>) -> Self {
        Self {
            workspace_path: workspace_path.as_ref().to_path_buf(),
        }
    }

    /// Agent 配置根目录。旧目录名 `AI提示词` 的迁移由 `workspace::WorkspaceService` 负责，
    /// 这里只在新目录缺失、旧目录存在时做一次读取回退。
    pub fn root(&self) -> PathBuf {
        let current = self.workspace_path.join(paths::AGENT_CONFIG_DIR);
        if current.is_dir() {
            return current;
        }
        let legacy = self.workspace_path.join(paths::LEGACY_AGENT_CONFIG_DIR);
        if legacy.is_dir() {
            return legacy;
        }
        current
    }

    /// 只播种缺失的文件，内容一字不差来自 Electron 生成的初始数据。
    /// 与所有读取 API 分离：查看 Agent 必须始终是只读的。
    pub fn ensure_seeds(&self) -> anyhow::Result<usize> {
        use std::io::Write;
        let seeds = Self::bundled_seeds();
        let workspace = self.workspace_path.canonicalize()?;
        let config = self.root();
        let mut written = 0;
        for (relative, body) in seeds.iter() {
            let relative = Path::new(relative);
            anyhow::ensure!(
                relative
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))),
                "invalid seed path"
            );
            let target = config.join(relative);
            let parent = target.parent().unwrap();
            let mut existing = parent;
            while !existing.exists() {
                existing = existing
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("invalid seed directory"))?;
            }
            anyhow::ensure!(
                existing.canonicalize()?.starts_with(&workspace),
                "Agent seed directory is outside workspace"
            );
            std::fs::create_dir_all(parent)?;
            anyhow::ensure!(
                parent.canonicalize()?.starts_with(&workspace),
                "Agent seed directory is outside workspace"
            );
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
            {
                Ok(mut file) => {
                    file.write_all(
                        body.as_str()
                            .ok_or_else(|| anyhow::anyhow!("invalid seed content"))?
                            .as_bytes(),
                    )?;
                    written += 1;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.consolidate_agents()?;
        self.track_agent_versions()?;
        Ok(written)
    }

    fn bundled_seeds() -> &'static serde_json::Map<String, serde_json::Value> {
        static SEEDS: std::sync::LazyLock<serde_json::Map<String, serde_json::Value>> =
            std::sync::LazyLock::new(|| {
                let mut seeds: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_str(include_str!("../../assets/agent-seeds.json"))
                        .expect("invalid generated agent seeds");
                // 原生专属工具也要出现在 Agent配置/Tools 里。复用可执行
                // 结构定义；下方的 create_new 会保留用户已有的所有覆盖项。
                for tool in super::tools::definitions::all() {
                    let name = &tool.function.name;
                    let guide = if name.starts_with("desktop_cards_") {
                        super::tools::desktop_tools::GUIDE
                    } else if name.starts_with("workflow_") {
                        super::tools::workflow_tools::GUIDE
                    } else if name.starts_with("script_") {
                        crate::script::GUIDE
                    } else if name.starts_with("base_automation_") {
                        super::tools::base_automation_tools::GUIDE
                    } else {
                        ""
                    };
                    let body=format!("---\nname: {name}\ndescription: {}\n---\n\n# {name}\n\n{}\n\n## 编写流程\n\n{}\n\n## 参数契约\n\n```json\n{}\n```\n",tool.function.description,tool.function.description,guide,serde_json::to_string_pretty(&tool.function.parameters).unwrap());
                    seeds.insert(
                        format!("Tools/{name}/TOOL.md"),
                        serde_json::Value::String(body),
                    );
                }
                seeds.insert(
                    "Skills/桌面卡片/SKILL.md".into(),
                    super::tools::desktop_tools::GUIDE.into(),
                );
                seeds.insert(
                    "Skills/工作流/SKILL.md".into(),
                    super::tools::workflow_tools::GUIDE.into(),
                );
                seeds
                    .retain(|path, _| !path.starts_with("Agents/") || path == "Agents/通用助手.md");
                seeds.insert("Agents/通用助手.md".into(), "---\nname: 通用助手\ndescription: 按需加载 Skills，管理墨池与工作区\nicon: Sparkles\nskillRouting: progressive\nroutableSkills: [*]\nmcps: [*]\n---\n你是墨池通用助手。根据用户目标完成工作，先查证再修改，保留无关内容。能力目录只介绍用途；需要执行时先调用 skill_load 获取对应指引和工具，再根据结果继续。只在工具确认成功后报告完成，待批准或排队不代表完成。".into());
                for skill in skill_catalog::bundled_skills() {
                    seeds
                        .entry(format!("Skills/{}/SKILL.md", skill.id))
                        .or_insert_with(|| skill_catalog::document(&skill).into());
                }
                seeds
            });
        &SEEDS
    }

    fn section(&self, name: &str) -> PathBuf {
        self.root().join(name)
    }

    pub fn load_agents(&self) -> Vec<AgentDefinition> {
        let mut agents: Vec<AgentDefinition> = read_markdown_dir(&self.section("Agents"))
            .into_iter()
            .map(|doc| {
                let (data, body) = parse_frontmatter(&doc.raw);
                let id = stem(&doc.name);
                let skills: Vec<String> = fm_array(&data, "skills")
                    .into_iter()
                    .filter(|s| s != "auto" && s != "*")
                    .collect();
                let mut routable = fm_array(&data, "routableSkills");
                if id == GENERAL_ASSISTANT_ID && !routable.is_empty() {
                    for name in ["多维表格", "工作流", "桌面卡片"] {
                        if !routable.iter().any(|existing| existing == name) {
                            routable.push(name.into());
                        }
                    }
                }

                AgentDefinition {
                    name: fm_string(&data, "name").unwrap_or_else(|| id.clone()),
                    description: fm_string(&data, "description"),
                    icon: fm_string(&data, "icon"),
                    tools: normalize_agent_tools(&id, fm_array(&data, "tools")),
                    skills,
                    routable_skills: if contains_wildcard(&routable) {
                        Selection::All
                    } else {
                        Selection::List(routable)
                    },
                    skill_routing: fm_choice(
                        &data,
                        "skillRouting",
                        &["none", "explicit", "light", "progressive"],
                        if id == GENERAL_ASSISTANT_ID {
                            "progressive"
                        } else {
                            "explicit"
                        },
                    ),
                    mcps: {
                        let mcps = fm_array(&data, "mcps");
                        if contains_wildcard(&mcps) {
                            Selection::All
                        } else {
                            Selection::List(mcps)
                        }
                    },
                    model: fm_string(&data, "model"),
                    execution_mode: fm_choice(
                        &data,
                        "executionMode",
                        &["reactive", "plan-and-execute"],
                        "reactive",
                    ),
                    max_turns: fm_number(&data, "maxTurns"),
                    follow_up_frequency: fm_string(&data, "followUpFrequency")
                        .and_then(|s| super::follow_up::FollowUpFrequency::from_str_loose(&s))
                        .unwrap_or(super::follow_up::FollowUpFrequency::Medium),
                    system_prompt: body.trim().to_owned(),
                    source_path: forward(&doc.path),
                    id,
                }
            })
            .collect();

        // 通用助手 永远排第一
        agents.sort_by(|a, b| {
            if a.id == GENERAL_ASSISTANT_ID {
                std::cmp::Ordering::Less
            } else if b.id == GENERAL_ASSISTANT_ID {
                std::cmp::Ordering::Greater
            } else {
                locale_compare(&a.name, &b.name)
            }
        });
        agents
    }

    /// 按 id 找 Agent；找不到回退到通用助手；再找不到返回 `None`。
    pub fn find_agent(&self, agent_id: Option<&str>) -> Option<AgentDefinition> {
        let agents = self.load_agents();
        agent_id
            .and_then(|id| agents.iter().find(|a| a.id == id).cloned())
            .or_else(|| {
                agents
                    .iter()
                    .find(|a| a.id == GENERAL_ASSISTANT_ID)
                    .cloned()
            })
    }

    pub fn load_skills(&self) -> Vec<SkillDefinition> {
        let mut skills: Vec<SkillDefinition> =
            read_subdir_docs(&self.section("Skills"), "SKILL.md")
                .into_iter()
                .map(|doc| {
                    let (data, body) = parse_frontmatter(&doc.raw);
                    SkillDefinition {
                        name: fm_string(&data, "name").unwrap_or_else(|| doc.name.clone()),
                        description: fm_string(&data, "description"),
                        aliases: fm_array(&data, "aliases"),
                        tools: fm_array(&data, "tools"),
                        mcps: fm_array(&data, "mcps"),
                        body: body.trim().to_owned(),
                        source_path: forward(&doc.path),
                        id: doc.name,
                    }
                })
                .collect();
        skills.sort_by(|a, b| locale_compare(&a.name, &b.name));
        skills
    }

    pub fn load_available_skills(&self) -> Vec<SkillDefinition> {
        let mut skills = self.load_skills();
        // 老工作区保留自己的指令，同时当前能力仍可被发现，
        // 且不覆盖仅允许新建文件的 Skill 快照。
        for mut builtin in skill_catalog::bundled_skills() {
            if let Some(existing) = skills.iter_mut().find(|s| s.id == builtin.id) {
                for name in builtin.tools {
                    if !existing.tools.contains(&name) {
                        existing.tools.push(name);
                    }
                }
            } else {
                builtin.source_path =
                    forward(&self.section("Skills").join(&builtin.id).join("SKILL.md"));
                skills.push(builtin);
            }
        }
        skills.sort_by(|a, b| locale_compare(&a.name, &b.name));
        skills
    }

    pub fn load_prompt_tools(&self) -> Vec<PromptToolDefinition> {
        read_subdir_docs(&self.section("Tools"), "TOOL.md")
            .into_iter()
            .map(|doc| {
                let (data, body) = parse_frontmatter(&doc.raw);
                let body = body.trim().to_owned();
                let tool = fm_string(&data, "tool");
                PromptToolDefinition {
                    id: tool.clone().unwrap_or_else(|| doc.name.clone()),
                    name: fm_string(&data, "name")
                        .or(tool)
                        .unwrap_or_else(|| doc.name.clone()),
                    description: fm_string(&data, "description").unwrap_or_else(|| body.clone()),
                    kind: fm_choice(&data, "kind", &["builtin", "http", "cli", "mcp"], "builtin"),
                    enabled: fm_bool(&data, "enabled", true),
                    parameters: fm_string(&data, "parameters"),
                    implementation: body,
                    source_path: forward(&doc.path),
                }
            })
            .collect()
    }

    pub fn load_mcp_servers(&self) -> Vec<McpServerDefinition> {
        read_subdir_docs(&self.section("MCPs"), "MCP.md")
            .into_iter()
            .map(|doc| {
                let (data, _) = parse_frontmatter(&doc.raw);
                let server = fm_string(&data, "server");
                McpServerDefinition {
                    id: server.clone().unwrap_or_else(|| doc.name.clone()),
                    name: fm_string(&data, "name")
                        .or(server)
                        .unwrap_or_else(|| doc.name.clone()),
                    description: fm_string(&data, "description"),
                    transport: fm_choice(&data, "transport", &["stdio", "http", "sse"], "stdio"),
                    command: fm_string(&data, "command"),
                    args: fm_array(&data, "args"),
                    url: fm_string(&data, "url"),
                    env: fm_key_value(&data, "env"),
                    tools: fm_array(&data, "tools"),
                    enabled: fm_bool(&data, "enabled", true),
                    source_path: forward(&doc.path),
                }
            })
            .collect()
    }

    pub fn load_quick_actions(&self) -> Vec<QuickActionDefinition> {
        let mut actions: Vec<QuickActionDefinition> =
            read_markdown_dir(&self.section("QuickActions"))
                .into_iter()
                .map(|doc| {
                    let (data, body) = parse_frontmatter(&doc.raw);
                    let id = fm_string(&data, "id").unwrap_or_else(|| stem(&doc.name));
                    QuickActionDefinition {
                        name: fm_string(&data, "name").unwrap_or_else(|| id.clone()),
                        description: fm_string(&data, "description"),
                        icon: fm_string(&data, "icon"),
                        // TS 是 `frontmatterNumber(data,'order',100) || 100`——
                        // JS 的 `|| ` 会把 0 也吃掉，order: 0 实际得到 100。照搬这个怪癖。
                        order: match fm_number(&data, "order") {
                            Some(0) | None => 100,
                            Some(n) => n,
                        },
                        enabled: fm_bool(&data, "enabled", true),
                        apply: fm_choice(&data, "apply", &["replace", "append"], "replace"),
                        system_prompt: fm_string(&data, "systemPrompt"),
                        prompt_template: body.trim().to_owned(),
                        source_path: forward(&doc.path),
                        id,
                    }
                })
                .filter(|a| a.enabled)
                .collect();
        actions.sort_by(|a, b| {
            a.order
                .cmp(&b.order)
                .then_with(|| locale_compare(&a.name, &b.name))
        });
        actions
    }

    /// 内置工具的描述覆盖：用户改了 `Tools/<name>/TOOL.md` 的 description 就以它为准。
    pub fn load_tool_overrides(&self) -> BTreeMap<String, String> {
        self.load_prompt_tools()
            .into_iter()
            .filter(|t| t.kind == "builtin" && t.enabled && !t.description.is_empty())
            .map(|t| (t.id, t.description))
            .collect()
    }

    pub fn update_agent_frequency(
        &self,
        agent_id: &str,
        frequency: super::follow_up::FollowUpFrequency,
    ) -> anyhow::Result<()> {
        let path = self.section("Agents").join(format!("{agent_id}.md"));
        anyhow::ensure!(path.is_file(), "Agent 配置文件不存在: {:?}", path);
        let raw = std::fs::read_to_string(&path)?;
        let (mut data, body) = parse_frontmatter(&raw);
        let mut found = false;
        for (k, v) in &mut data {
            if k == "followUpFrequency" {
                *v = FmValue::Scalar(frequency.as_str().to_owned());
                found = true;
                break;
            }
        }
        if !found {
            data.push((
                "followUpFrequency".into(),
                FmValue::Scalar(frequency.as_str().to_owned()),
            ));
        }
        let serialized = serialize_frontmatter(&data, &body);
        std::fs::write(&path, serialized)?;
        Ok(())
    }
}

pub fn serialize_frontmatter(data: &Frontmatter, body: &str) -> String {
    let mut out = String::from("---\n");
    for (k, v) in data {
        match v {
            FmValue::Scalar(s) => out.push_str(&format!("{k}: {s}\n")),
            FmValue::List(l) => {
                let items = l.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
                out.push_str(&format!("{k}: [{items}]\n"));
            }
        }
    }
    out.push_str("---\n");
    let trimmed_body = body.trim();
    if !trimmed_body.is_empty() {
        out.push_str(trimmed_body);
        out.push('\n');
    }
    out
}

fn contains_wildcard(items: &[String]) -> bool {
    items.iter().any(|s| s == "*" || s == "all")
}

/// 自定义 Agent 空列表 → 全部工具。通用助手可按需发现现有内置能力，
/// 排除由审批流程使用的内部 apply 接口；这不是首轮模型工具列表。
///
/// 注意 `*` / `all` 在**工具**列表里不是通配符（那是 mcps/routableSkills 的语义）——
/// C# 版把它们当普通项过滤掉了，导致写了 `tools: [*]` 的 Agent 反而变成"无工具"。
pub fn normalize_agent_tools(agent_id: &str, tools: Vec<String>) -> Selection {
    if agent_id == GENERAL_ASSISTANT_ID {
        return Selection::List(
            super::tools::definitions::names()
                .into_iter()
                .filter(|name| *name != "agenda_apply_change")
                .map(str::to_owned)
                .collect(),
        );
    }
    if tools.is_empty() {
        return Selection::All;
    }
    if agent_id == WEB_RESEARCH_ASSISTANT_ID {
        let mut next = tools;
        for required in ["web_search", "http_request"] {
            if !next.iter().any(|tool| tool == required) {
                next.push(required.to_owned());
            }
        }
        return Selection::List(next);
    }
    Selection::List(tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct TempWs(PathBuf);
    impl TempWs {
        fn new(tag: &str) -> Self {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir()
                .join(format!("mochi-agentcfg-{}-{tag}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn write(&self, rel: &str, content: &str) {
            let p = self.0.join(paths::AGENT_CONFIG_DIR).join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }
        fn svc(&self) -> AgentConfigService {
            AgentConfigService::new(&self.0)
        }
    }
    impl Drop for TempWs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn frontmatter_scalar_list_and_body() {
        let (data, body) = parse_frontmatter(
            "---\nname: 通用助手\ntools: [a, \"b\", 'c']\nempty: []\n---\n正文第一行\n正文第二行",
        );
        assert_eq!(fm_string(&data, "name").as_deref(), Some("通用助手"));
        assert_eq!(fm_array(&data, "tools"), ["a", "b", "c"], "引号应被剥掉");
        assert!(fm_array(&data, "empty").is_empty());
        assert_eq!(body.trim(), "正文第一行\n正文第二行");
    }

    #[test]
    fn frontmatter_strips_bom_and_handles_crlf() {
        let (data, body) = parse_frontmatter("\u{feff}---\r\nname: X\r\n---\r\n正文");
        assert_eq!(fm_string(&data, "name").as_deref(), Some("X"));
        assert_eq!(body.trim(), "正文");
    }

    /// 没有文档头部时，返回**原始全文**作为正文，而不是空字符串。
    #[test]
    fn missing_frontmatter_returns_raw_body() {
        let raw = "# 只有正文\n没有分隔符";
        let (data, body) = parse_frontmatter(raw);
        assert!(data.is_empty());
        assert_eq!(body, raw);
    }

    #[test]
    fn frontmatter_accessors() {
        let (data, _) = parse_frontmatter(
            "---\nflag: yes\noff: 0\nnum: 12\nbad: 乱写\nchoice: light\nenv: [A=1, B=2, 坏项]\n---\n",
        );
        assert!(fm_bool(&data, "flag", false));
        assert!(!fm_bool(&data, "off", true));
        assert!(fm_bool(&data, "缺失", true), "缺失时用 fallback");
        assert_eq!(fm_number(&data, "num"), Some(12));
        assert_eq!(fm_number(&data, "bad"), None);
        assert_eq!(
            fm_choice(&data, "choice", &["none", "light"], "none"),
            "light"
        );
        assert_eq!(
            fm_choice(&data, "bad", &["none", "light"], "none"),
            "none",
            "非法值应回退"
        );
        let env = fm_key_value(&data, "env");
        assert_eq!(env.get("A").map(String::as_str), Some("1"));
        assert_eq!(env.len(), 2, "非 k=v 的项应跳过");
    }

    /// C# 版没有这段：存量工作区的通用助手会失去设置控制与产品知识能力。
    #[test]
    fn general_assistant_gets_required_tools_injected() {
        let ws = TempWs::new("required-tools");
        ws.write(
            "Agents/通用助手.md",
            "---\nname: 通用助手\ntools: [file_read, file_write]\n---\n提示词",
        );
        let agents = ws.svc().load_agents();
        let tools = agents[0].tools.as_list();

        assert!(tools.contains(&"file_read".to_string()), "原有工具应保留");
        for required in REQUIRED_GENERAL_ASSISTANT_TOOLS {
            assert!(
                tools.iter().any(|t| t == required),
                "通用助手缺内置工具 {required}: {tools:?}"
            );
        }
    }

    #[test]
    fn other_agents_do_not_get_injected_tools() {
        let ws = TempWs::new("no-inject");
        ws.write(
            "Agents/日程助手.md",
            "---\nname: 日程助手\ntools: [agenda_list]\n---\nX",
        );
        let agents = ws.svc().load_agents();
        assert_eq!(agents[0].tools, Selection::List(vec!["agenda_list".into()]));
    }

    #[test]
    fn legacy_web_research_agent_gets_executable_search_tools() {
        let tools = normalize_agent_tools(
            WEB_RESEARCH_ASSISTANT_ID,
            vec!["file_read".into(), "agent_plan".into()],
        );
        let tools = tools.as_list();
        assert!(tools.contains(&"web_search".into()));
        assert!(tools.contains(&"http_request".into()));
    }

    #[test]
    fn empty_tool_list_means_all_tools() {
        let ws = TempWs::new("all-tools");
        ws.write("Agents/自由助手.md", "---\nname: 自由助手\n---\nX");
        assert!(
            ws.svc().load_agents()[0].tools.is_all(),
            "没写 tools 应表示全部"
        );
    }

    #[test]
    fn general_assistant_sorts_first() {
        let ws = TempWs::new("sort");
        ws.write("Agents/阿助手.md", "---\nname: 阿助手\n---\nX");
        ws.write("Agents/通用助手.md", "---\nname: 通用助手\n---\nX");
        ws.write("Agents/写作助手.md", "---\nname: 写作助手\n---\nX");

        let ids: Vec<String> = ws.svc().load_agents().into_iter().map(|a| a.id).collect();
        assert_eq!(ids[0], GENERAL_ASSISTANT_ID, "通用助手必须排第一: {ids:?}");
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn skill_routing_defaults_differ_for_general_assistant() {
        let ws = TempWs::new("routing");
        ws.write("Agents/通用助手.md", "---\nname: 通用助手\n---\nX");
        ws.write("Agents/别的.md", "---\nname: 别的\n---\nX");
        let agents = ws.svc().load_agents();
        let by_id = |id: &str| agents.iter().find(|a| a.id == id).unwrap();
        assert_eq!(by_id(GENERAL_ASSISTANT_ID).skill_routing, "progressive");
        assert_eq!(by_id("别的").skill_routing, "explicit");
    }

    #[test]
    fn wildcards_mean_all_for_mcps_and_routable_skills() {
        let ws = TempWs::new("wildcard");
        ws.write(
            "Agents/A.md",
            "---\nname: A\nmcps: [*]\nroutableSkills: [all]\ntools: [x]\n---\nX",
        );
        let a = &ws.svc().load_agents()[0];
        assert!(a.mcps.is_all());
        assert!(a.routable_skills.is_all());
        assert_eq!(
            a.tools,
            Selection::List(vec!["x".into()]),
            "工具列表不认通配符"
        );
    }

    #[test]
    fn skills_filter_out_auto_and_star() {
        let ws = TempWs::new("skills-filter");
        ws.write(
            "Agents/A.md",
            "---\nname: A\nskills: [出题规范, auto, *]\n---\nX",
        );
        assert_eq!(ws.svc().load_agents()[0].skills, ["出题规范"]);
    }

    #[test]
    fn find_agent_falls_back_to_general_assistant() {
        let ws = TempWs::new("find");
        ws.write("Agents/通用助手.md", "---\nname: 通用助手\n---\nX");
        ws.write("Agents/日程助手.md", "---\nname: 日程助手\n---\nX");
        let svc = ws.svc();

        assert_eq!(svc.find_agent(Some("日程助手")).unwrap().id, "日程助手");
        assert_eq!(
            svc.find_agent(Some("不存在")).unwrap().id,
            GENERAL_ASSISTANT_ID
        );
        assert_eq!(svc.find_agent(None).unwrap().id, GENERAL_ASSISTANT_ID);
    }

    #[test]
    fn find_agent_on_empty_workspace_returns_none() {
        let ws = TempWs::new("find-empty");
        assert!(ws.svc().find_agent(Some("任意")).is_none());
    }

    #[test]
    fn skills_are_read_from_subdirectories() {
        let ws = TempWs::new("skills");
        ws.write(
            "Skills/出题规范/SKILL.md",
            "---\nname: 出题规范\ndescription: 从文档生成练习题\naliases: [出题, 命题]\n---\n# 工作流\n1. 读材料",
        );
        // 没有 SKILL.md 的目录应被忽略
        fs::create_dir_all(
            ws.0.join(paths::AGENT_CONFIG_DIR)
                .join("Skills")
                .join("空目录"),
        )
        .unwrap();

        let skills = ws.svc().load_skills();
        assert_eq!(skills.len(), 1, "没有 SKILL.md 的目录不该入列");
        assert_eq!(skills[0].id, "出题规范");
        assert_eq!(skills[0].aliases, ["出题", "命题"]);
        assert!(skills[0].body.starts_with("# 工作流"));
    }

    #[test]
    fn prompt_tools_and_overrides() {
        let ws = TempWs::new("tools");
        ws.write(
            "Tools/file_read/TOOL.md",
            "---\ntool: file_read\nkind: builtin\nenabled: true\ndescription: 读取文件（用户改过的说明）\n---\n实现说明",
        );
        ws.write(
            "Tools/disabled_tool/TOOL.md",
            "---\ntool: disabled_tool\nenabled: false\ndescription: X\n---\n",
        );
        ws.write(
            "Tools/http_tool/TOOL.md",
            "---\ntool: http_tool\nkind: http\ndescription: Y\n---\n",
        );

        let svc = ws.svc();
        assert_eq!(svc.load_prompt_tools().len(), 3);

        let overrides = svc.load_tool_overrides();
        assert_eq!(
            overrides.get("file_read").map(String::as_str),
            Some("读取文件（用户改过的说明）")
        );
        assert!(
            !overrides.contains_key("disabled_tool"),
            "禁用的工具不该产生覆盖"
        );
        assert!(
            !overrides.contains_key("http_tool"),
            "非 builtin 不该产生覆盖"
        );
    }

    #[test]
    fn tool_description_falls_back_to_body() {
        let ws = TempWs::new("tool-body");
        ws.write("Tools/t/TOOL.md", "---\ntool: t\n---\n这是正文当描述");
        assert_eq!(
            ws.svc().load_prompt_tools()[0].description,
            "这是正文当描述"
        );
    }

    #[test]
    fn mcp_servers_parse_transport_and_env() {
        let ws = TempWs::new("mcp");
        ws.write(
            "MCPs/web-search/MCP.md",
            "---\nserver: web-search\nname: Web 搜索\ntransport: sse\nurl: https://x/y\nenv: [KEY=abc, MODE=fast]\nenabled: true\n---\n",
        );
        let servers = ws.svc().load_mcp_servers();
        assert_eq!(servers[0].id, "web-search");
        assert_eq!(servers[0].transport, "sse");
        assert_eq!(servers[0].url.as_deref(), Some("https://x/y"));
        assert_eq!(servers[0].env.get("KEY").map(String::as_str), Some("abc"));
    }

    #[test]
    fn mcp_transport_falls_back_to_stdio() {
        let ws = TempWs::new("mcp-transport");
        ws.write("MCPs/s/MCP.md", "---\nserver: s\ntransport: 乱写\n---\n");
        assert_eq!(ws.svc().load_mcp_servers()[0].transport, "stdio");
    }

    #[test]
    fn quick_actions_filter_disabled_and_sort_by_order() {
        let ws = TempWs::new("quick");
        ws.write(
            "QuickActions/乙.md",
            "---\nname: 乙\norder: 10\n---\n模板乙",
        );
        ws.write(
            "QuickActions/甲.md",
            "---\nname: 甲\norder: 20\n---\n模板甲",
        );
        ws.write(
            "QuickActions/关闭的.md",
            "---\nname: 关闭的\nenabled: false\n---\nX",
        );

        let actions = ws.svc().load_quick_actions();
        let names: Vec<&str> = actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["乙", "甲"], "应按 order 升序，且禁用项被过滤");
        assert_eq!(actions[0].prompt_template, "模板乙");
    }

    /// TS 用 `frontmatterNumber(...) || 100`，JS 的 `||` 会把 0 也吃掉。照搬这个怪癖。
    #[test]
    fn quick_action_order_zero_becomes_100_like_js() {
        let ws = TempWs::new("quick-zero");
        ws.write("QuickActions/A.md", "---\nname: A\norder: 0\n---\nX");
        assert_eq!(ws.svc().load_quick_actions()[0].order, 100);
    }

    #[test]
    fn missing_directories_yield_empty_lists() {
        let ws = TempWs::new("empty");
        let svc = ws.svc();
        assert!(svc.load_agents().is_empty());
        assert!(svc.load_skills().is_empty());
        assert!(svc.load_prompt_tools().is_empty());
        assert!(svc.load_mcp_servers().is_empty());
        assert!(svc.load_quick_actions().is_empty());
    }

    /// 新目录不存在但旧目录在时，读取应回退到旧目录（迁移由 workspace 层负责，
    /// 但读取路径不能因为还没迁移就一片空白）。
    #[test]
    fn falls_back_to_legacy_directory_for_reading() {
        let ws = TempWs::new("legacy");
        let legacy = ws.0.join(paths::LEGACY_AGENT_CONFIG_DIR).join("Agents");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("旧助手.md"), "---\nname: 旧助手\n---\nX").unwrap();

        let agents = ws.svc().load_agents();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].id, "旧助手");
    }

    #[test]
    fn source_paths_use_forward_slashes() {
        let ws = TempWs::new("paths");
        ws.write("Agents/A.md", "---\nname: A\n---\nX");
        assert!(!ws.svc().load_agents()[0].source_path.contains('\\'));
    }

    #[test]
    fn broken_file_does_not_kill_the_whole_list() {
        let ws = TempWs::new("broken");
        ws.write("Agents/好的.md", "---\nname: 好的\n---\nX");
        ws.write(
            "Agents/坏的.md",
            "这文件没有 frontmatter，但也不该让整个列表失败",
        );
        let agents = ws.svc().load_agents();
        assert_eq!(agents.len(), 2, "无 frontmatter 的文件应按 id=文件名 收下");
        let broken = agents.iter().find(|a| a.id == "坏的").unwrap();
        assert_eq!(broken.name, "坏的", "缺 name 时用文件名兜底");
    }

    #[test]
    fn follow_up_frequency_default_and_update() {
        use crate::ai::FollowUpFrequency;
        let ws = TempWs::new("fu-freq");
        // 旧 Agent 缺省 followUpFrequency 应为 medium
        ws.write(
            "Agents/测试助手.md",
            "---\nname: 测试助手\n---\n系统提示词正文",
        );
        let svc = ws.svc();
        let agent = svc.find_agent(Some("测试助手")).unwrap();
        assert_eq!(agent.follow_up_frequency, FollowUpFrequency::Medium);

        // 更新为 never
        svc.update_agent_frequency("测试助手", FollowUpFrequency::Never)
            .unwrap();
        let updated = svc.find_agent(Some("测试助手")).unwrap();
        assert_eq!(updated.follow_up_frequency, FollowUpFrequency::Never);

        svc.update_agent_frequency("测试助手", FollowUpFrequency::Aggressive)
            .unwrap();
        let updated2 = svc.find_agent(Some("测试助手")).unwrap();
        assert_eq!(updated2.follow_up_frequency, FollowUpFrequency::Aggressive);
        assert_eq!(updated2.system_prompt, "系统提示词正文");
    }
}
