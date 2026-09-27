//! 这里只判定到期任务并保存运行状态，定时器和模型调用由宿主负责。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ai::agent_config::{parse_frontmatter, FmValue, Frontmatter};
use crate::ai::models::AiToolDefinition;
use crate::{json2, jstime, paths};

/// 任务定义目录，工作区根下。
pub const TASKS_DIR: &str = "Tasks";
/// 上次运行时间的状态文件。
pub const STATE_FILE: &str = "ai-agent/task-state.json";
/// 运行日志（JSONL 追加）。
pub const RUNS_FILE: &str = "ai-agent/task-runs.jsonl";

/// 自治级别，决定后台任务的写操作怎么处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskAutonomy {
    /// 只给只读工具，不可能产生写入。
    ReadOnly,
    /// 白名单内的写操作直接执行。
    Whitelisted,
    /// 写操作全部降级成提案，进审批收件箱等用户确认。
    ProposeOnly,
}

impl TaskAutonomy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Whitelisted => "whitelisted",
            Self::ProposeOnly => "propose-only",
        }
    }

    /// 认不出来的值一律当 `propose-only`——**默认落在最安全的一侧**。
    /// 用户在文档头部中拼错 `autonomy` 时，后果应是“多确认一次”，而不是“随意写入”。
    pub fn parse(value: &str) -> Self {
        match value {
            "read-only" => Self::ReadOnly,
            "whitelisted" => Self::Whitelisted,
            _ => Self::ProposeOnly,
        }
    }
}

/// 后台任务只读模式下允许的工具，含原生工作流与卡片查询。
const READ_ONLY_TOOLS: &[&str] = &[
    "workflow_catalog",
    "workflow_list",
    "workflow_get",
    "workflow_validate",
    "workflow_history",
    "desktop_cards_get",
    "desktop_cards_templates",
    "workspace_get_state",
    "current_document_get",
    "kb_list",
    "folder_list",
    "file_read",
    "content_search",
    "agenda_overview",
    "agenda_list",
    "agenda_get",
    "agenda_find_free_time",
    "agenda_history",
    "memory_search",
    "base_get_schema",
    "base_automation_list",
    "base_automation_test",
    "script_environment",
    "base_query_records",
    "http_request",
    "web_search",
];

#[derive(Debug, Clone, PartialEq)]
pub struct AgentTask {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    /// 简化的调度表达式：`interval:30m` / `daily:09:00` / `hourly` / `manual`
    pub schedule: String,
    pub agent_id: Option<String>,
    pub autonomy: TaskAutonomy,
    /// 已解析成绝对路径（相对项按工作区根拼接）。
    pub write_whitelist: Vec<String>,
    pub prompt: String,
    pub source_path: String,
}

/// 运行状态。字段形状即磁盘契约——Electron 版读同一个文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskState {
    pub version: u8,
    #[serde(default)]
    pub last_run: BTreeMap<String, i64>,
}

impl Default for TaskState {
    fn default() -> Self {
        Self {
            version: 1,
            last_run: BTreeMap::new(),
        }
    }
}

/// 一条运行日志。`summary` / `error` 为 `None` 时**整个键省略**——
/// TS 的 `JSON.stringify` 就是这么处理 `undefined` 的。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRunRecord {
    pub task_id: String,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------- 文档头部取值 ----------
//
// 不要复用 `agent_config` 的 `fm_*` 辅助函数：调度器对文档头部的处理规则**不同**。
// 最要命的是 `enabled`——TS 写的是 `data.enabled !== 'false'`，只有字面量 `false`
// 才关得掉；`enabled: no` 在调度器下是**开着的**，而 `agent_config::fm_bool` 会把它读成关。
// 两边随手统一，用户就会遇到「同样的写法在两处含义相反」。

fn fm<'a>(data: &'a Frontmatter, key: &str) -> Option<&'a FmValue> {
    data.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// 标量取值。空字符串视为不存在（与 TS 的 `data.x || fallback` 一致）。
fn fm_str(data: &Frontmatter, key: &str) -> Option<String> {
    match fm(data, key) {
        Some(FmValue::Scalar(s)) if !s.is_empty() => Some(s.clone()),
        // 列表形式出现在标量位置时还原成原始写法，交给调用方处理
        Some(FmValue::List(items)) if !items.is_empty() => Some(items.join(", ")),
        _ => None,
    }
}

/// 列表取值。对齐 TS 的 `parseList`：`[a, b]` / `a, b` / `a` 都收。
fn fm_list(data: &Frontmatter, key: &str) -> Vec<String> {
    match fm(data, key) {
        Some(FmValue::List(items)) => items.clone(),
        // 标量也按逗号分隔——TS 不会把文档头部解析为数组，因此 `writeWhitelist: a, b`
        // 到 parseList 手里才被拆开
        Some(FmValue::Scalar(s)) => s
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|item| item.trim().trim_matches(['\'', '"']).to_owned())
            .filter(|item| !item.is_empty())
            .collect(),
        None => Vec::new(),
    }
}

fn tasks_dir(workspace: &Path) -> PathBuf {
    workspace.join(paths::AGENT_CONFIG_DIR).join(TASKS_DIR)
}

/// 读出全部任务定义。目录不存在或单个文件损坏都不是错误——
/// 一个写坏的任务文件不该让其余任务全部停摆。
pub fn load_tasks(workspace: &Path) -> Vec<AgentTask> {
    let dir = tasks_dir(workspace);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
        .collect();
    // `fs::read_dir` 的顺序随文件系统而变；排序让「先跑哪个任务」可复现
    files.sort();

    files
        .iter()
        .filter_map(|path| {
            let raw = std::fs::read_to_string(path).ok()?;
            let id = path.file_stem()?.to_string_lossy().into_owned();
            let (data, body) = parse_frontmatter(&raw);

            Some(AgentTask {
                name: fm_str(&data, "name").unwrap_or_else(|| id.clone()),
                description: fm_str(&data, "description"),
                // 只有字面量 "false" 关得掉，见上面的说明
                enabled: fm_str(&data, "enabled").as_deref() != Some("false"),
                schedule: fm_str(&data, "schedule").unwrap_or_else(|| "manual".into()),
                agent_id: fm_str(&data, "agent"),
                autonomy: fm_str(&data, "autonomy")
                    .map(|v| TaskAutonomy::parse(&v))
                    .unwrap_or(TaskAutonomy::ProposeOnly),
                write_whitelist: fm_list(&data, "writeWhitelist")
                    .into_iter()
                    .map(|item| absolutize(workspace, &item))
                    .collect(),
                prompt: body.trim().to_owned(),
                source_path: paths::to_forward_slashes(&path.to_string_lossy()),
                id,
            })
        })
        .collect()
}

fn absolutize(workspace: &Path, item: &str) -> String {
    let candidate = Path::new(item);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace.join(candidate)
    };
    paths::to_forward_slashes(&joined.to_string_lossy())
}

/// 这个任务此刻该不该跑。
///
/// 支持 `interval:<n>[m|h|d]`、`hourly`、`daily:<HH:MM>`；`manual` 永不自动触发。
/// `now` 显式传入而不是内部取，否则时间相关的分支根本没法测。
pub fn is_task_due(
    task: &AgentTask,
    last_run: Option<i64>,
    now: chrono::DateTime<chrono::Local>,
) -> bool {
    if !task.enabled {
        return false;
    }
    let schedule = task.schedule.trim().to_lowercase();
    if schedule.is_empty() || schedule == "manual" {
        return false;
    }
    let now_ms = now.timestamp_millis();

    if let Some(raw) = schedule.strip_prefix("interval:") {
        let raw = raw.trim();
        let digits: String = raw.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return false;
        }
        let unit = raw[digits.len()..].trim();
        let factor = match unit {
            "" | "m" => 60_000i64,
            "h" => 3_600_000,
            "d" => 86_400_000,
            // TS 的正则是 ^(\d+)\s*([mhd]?)$，别的后缀整体不匹配
            _ => return false,
        };
        let Ok(amount) = digits.parse::<i64>() else {
            return false;
        };
        let interval = amount.saturating_mul(factor);
        if interval <= 0 {
            return false;
        }
        return last_run.is_none_or(|last| now_ms - last >= interval);
    }

    if schedule == "hourly" {
        return last_run.is_none_or(|last| now_ms - last >= 3_600_000);
    }

    if let Some(rest) = schedule.strip_prefix("daily:") {
        let mut parts = rest.split(':');
        let Ok(hour) = parts.next().unwrap_or("").trim().parse::<u32>() else {
            return false;
        };
        let minute = match parts.next() {
            None => 0,
            Some(text) => match text.trim().parse::<u32>() {
                Ok(value) => value,
                Err(_) => return false,
            },
        };

        // 今天的这个时刻。**本地时区**——用户写 daily:09:00 指的是自己的九点。
        use chrono::TimeZone;
        let Some(target) = now
            .date_naive()
            .and_hms_opt(hour, minute, 0)
            .and_then(|naive| chrono::Local.from_local_datetime(&naive).earliest())
        else {
            return false;
        };
        let target_ms = target.timestamp_millis();
        if now_ms < target_ms {
            return false;
        }
        // 今天这个点已经跑过就不再重复
        return last_run.is_none_or(|last| last < target_ms);
    }

    false
}

// ---------- 状态与日志 ----------

fn state_path(workspace: &Path) -> PathBuf {
    workspace.join(paths::MOCHI_DIR_NAME).join(STATE_FILE)
}

fn runs_path(workspace: &Path) -> PathBuf {
    workspace.join(paths::MOCHI_DIR_NAME).join(RUNS_FILE)
}

/// 读运行状态。文件缺失或损坏都退回空状态——状态丢了最多是任务多跑一次，
/// 而在这里报错会让调度整个停摆。
pub fn read_state(workspace: &Path) -> TaskState {
    std::fs::read_to_string(state_path(workspace))
        .ok()
        .and_then(|raw| serde_json::from_str::<TaskState>(&raw).ok())
        .map(|state| TaskState {
            version: 1,
            last_run: state.last_run,
        })
        .unwrap_or_default()
}

/// 写运行状态。**两空格缩进 + 尾换行**，与 TS 的
/// `JSON.stringify(state, null, 2) + '\n'` 字节一致。
///
/// 注意尾换行：`json2::write` 不加（`.mochi/*.json` 那批本来就没有），
/// 这个文件是 TS 侧显式加了 `+ '\n'` 的那一类，见 docs 里的落盘约定对照表。
pub fn write_state(workspace: &Path, state: &TaskState) -> std::io::Result<()> {
    let body = json2::serialize(state).map_err(std::io::Error::other)?;
    json2::write_to_file(state_path(workspace), &format!("{body}\n")).map_err(std::io::Error::other)
}

/// 追加一条运行日志。紧凑 JSON + 换行（JSONL）。
pub fn append_run(workspace: &Path, record: &TaskRunRecord) -> std::io::Result<()> {
    use std::io::Write;
    let target = runs_path(workspace);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(record).map_err(std::io::Error::other)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(target)?;
    writeln!(file, "{line}")
}

/// 取出此刻该跑的任务，**并立即把 `lastRun` 落盘**。
///
/// 先记后跑是有意的：任务真正执行要几十秒到几分钟，这期间如果按周期又检查一次，
/// 同一个任务会被重复触发。宁可任务崩了这轮不补跑，也不能让它并发跑两份。
pub fn take_due_tasks(workspace: &Path, now: chrono::DateTime<chrono::Local>) -> Vec<AgentTask> {
    let tasks = load_tasks(workspace);
    if tasks.is_empty() {
        return Vec::new();
    }

    let mut state = read_state(workspace);
    let due: Vec<AgentTask> = tasks
        .into_iter()
        .filter(|task| is_task_due(task, state.last_run.get(&task.id).copied(), now))
        .collect();
    if due.is_empty() {
        return due;
    }

    let stamp = now.timestamp_millis();
    for task in &due {
        state.last_run.insert(task.id.clone(), stamp);
    }
    // 落盘失败就让任务下轮再跑一次，总好过不跑
    let _ = write_state(workspace, &state);
    due
}

/// 按自治级别裁剪工具集。
///
/// 只在只读模式（`read-only`）下裁剪：另两级要保留全部工具，写操作由权限检查与提案机制处理。
/// `mcp__` 前缀的外部工具一律放行——核心层无从判断它们的读写性质，
/// 这与 TS 一致（那边同样只按前缀放行）。
pub fn filter_tools_for_autonomy(
    tools: &[AiToolDefinition],
    autonomy: TaskAutonomy,
) -> Vec<AiToolDefinition> {
    if autonomy != TaskAutonomy::ReadOnly {
        return tools.to_vec();
    }
    tools
        .iter()
        .filter(|tool| {
            let name = tool.function.name.as_str();
            READ_ONLY_TOOLS.contains(&name) || name.starts_with("mcp__")
        })
        .cloned()
        .collect()
}

/// 无人值守运行时追加到系统提示词后面的说明。
///
/// 必须告诉模型「用户不在场」：否则它会像在交互对话里那样反问一句然后停下来，
/// 而没有人会回答，这轮任务就白跑了。
pub fn unattended_prompt_suffix(task_name: &str, autonomy: TaskAutonomy) -> String {
    let mode = if autonomy == TaskAutonomy::ReadOnly {
        "本次运行为只读模式，你只能读取信息，不要尝试任何写操作。"
    } else {
        "需要批准的写操作会进入审批收件箱，届时不要声称已经完成。"
    };
    format!("当前是一次无人值守的后台任务「{task_name}」。用户不在场，无法回答追问。\n\n{mode}")
}

/// 时间戳工具，供上层记录运行日志。
pub fn now_millis() -> i64 {
    jstime::now_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct Ws(PathBuf);
    impl Drop for Ws {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    impl Ws {
        fn task(&self, name: &str, body: &str) -> &Self {
            let dir = tasks_dir(&self.0);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{name}.md")), body).unwrap();
            self
        }
        fn tasks(&self) -> Vec<AgentTask> {
            load_tasks(&self.0)
        }
    }

    fn ws(tag: &str) -> Ws {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-tasks-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Ws(root)
    }

    fn at(h: u32, m: u32) -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(2026, 8, 29, h, m, 0).unwrap()
    }

    fn task(schedule: &str) -> AgentTask {
        AgentTask {
            id: "t".into(),
            name: "任务".into(),
            description: None,
            enabled: true,
            schedule: schedule.into(),
            agent_id: None,
            autonomy: TaskAutonomy::ProposeOnly,
            write_whitelist: Vec::new(),
            prompt: "干活".into(),
            source_path: String::new(),
        }
    }

    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 3_600_000;

    // ---------- 定义加载 ----------

    #[test]
    fn a_task_file_is_parsed_into_a_definition() {
        let w = ws("load");
        w.task(
            "每日总结",
            "---\nname: 每日总结\ndescription: 汇总今天写了什么\nschedule: daily:09:00\nagent: 写作助手\nautonomy: read-only\n---\n请汇总我今天的笔记。\n",
        );

        let tasks = w.tasks();
        assert_eq!(tasks.len(), 1);
        let t = &tasks[0];
        assert_eq!(t.id, "每日总结", "id 取文件名");
        assert_eq!(t.name, "每日总结");
        assert_eq!(t.description.as_deref(), Some("汇总今天写了什么"));
        assert_eq!(t.schedule, "daily:09:00");
        assert_eq!(t.agent_id.as_deref(), Some("写作助手"));
        assert_eq!(t.autonomy, TaskAutonomy::ReadOnly);
        assert_eq!(t.prompt, "请汇总我今天的笔记。");
        assert!(t.enabled);
        assert!(
            t.source_path.ends_with("Agent配置/Tasks/每日总结.md"),
            "{}",
            t.source_path
        );
    }

    #[test]
    fn missing_fields_fall_back_to_safe_defaults() {
        let w = ws("defaults");
        w.task("裸任务", "没有 frontmatter，整篇都是提示词。");

        let t = &w.tasks()[0];
        assert_eq!(t.name, "裸任务", "没写 name 就用文件名");
        assert_eq!(t.schedule, "manual", "没写 schedule 就不自动跑");
        assert_eq!(
            t.autonomy,
            TaskAutonomy::ProposeOnly,
            "缺省要落在最安全的一侧"
        );
        assert_eq!(t.prompt, "没有 frontmatter，整篇都是提示词。");
        assert!(t.agent_id.is_none());
    }

    /// **只有字面量 `false` 关得掉**。这与 `agent_config::fm_bool` 的语义不同，
    /// 是照抄 TS 的 `data.enabled !== 'false'`，不是疏忽。
    #[test]
    fn only_the_literal_false_disables_a_task() {
        let w = ws("enabled");
        w.task("关", "---\nenabled: false\n---\nx");
        w.task("开1", "---\nenabled: true\n---\nx");
        w.task("开2", "---\nenabled: no\n---\nx");
        w.task("开3", "---\nenabled: 0\n---\nx");

        let by_id: BTreeMap<_, _> = w.tasks().into_iter().map(|t| (t.id.clone(), t)).collect();
        assert!(!by_id["关"].enabled);
        assert!(by_id["开1"].enabled);
        assert!(by_id["开2"].enabled, "no 在调度器语义下是开着的");
        assert!(by_id["开3"].enabled, "0 在调度器语义下是开着的");
    }

    #[test]
    fn unknown_autonomy_values_degrade_to_propose_only() {
        let w = ws("autonomy");
        w.task("拼错了", "---\nautonomy: readonly\n---\nx");
        assert_eq!(
            w.tasks()[0].autonomy,
            TaskAutonomy::ProposeOnly,
            "拼错的后果应该是多问一次，而不是随便写"
        );
    }

    #[test]
    fn the_write_whitelist_is_resolved_to_absolute_paths() {
        let w = ws("whitelist");
        w.task(
            "白名单",
            "---\nwriteWhitelist: [知识库/草稿, D:/别处/固定]\n---\nx",
        );

        let list = &w.tasks()[0].write_whitelist;
        assert_eq!(list.len(), 2);
        assert!(list[0].ends_with("/知识库/草稿"), "{}", list[0]);
        assert!(list[0].starts_with(&paths::to_forward_slashes(&w.0.to_string_lossy())));
        assert_eq!(list[1], "D:/别处/固定", "绝对路径原样保留");
    }

    /// 一个写坏的任务文件不该让其余任务全部停摆。
    #[test]
    fn a_broken_file_does_not_take_down_the_others() {
        let w = ws("broken");
        w.task("好的", "---\nschedule: hourly\n---\n干活");
        std::fs::write(tasks_dir(&w.0).join("不是md.txt"), "忽略我").unwrap();
        w.task("残缺", "---\nname: 只有开头没有结尾\n还是没结尾");

        let tasks = w.tasks();
        assert_eq!(tasks.len(), 2, "只收 .md，且损坏的那个仍作为任务存在");
        assert!(tasks.iter().any(|t| t.id == "好的"));
    }

    #[test]
    fn tasks_are_returned_in_a_stable_order() {
        let w = ws("order");
        for name in ["c", "a", "b"] {
            w.task(name, "---\nschedule: hourly\n---\nx");
        }
        let ids: Vec<String> = w.tasks().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["a", "b", "c"], "顺序要可复现，不能随文件系统而变");
    }

    #[test]
    fn a_missing_tasks_directory_is_not_an_error() {
        let w = ws("nodir");
        assert!(w.tasks().is_empty());
    }

    // ---------- 到点判定 ----------

    #[test]
    fn manual_and_disabled_tasks_never_fire() {
        assert!(!is_task_due(&task("manual"), None, at(9, 0)));
        assert!(!is_task_due(&task(""), None, at(9, 0)));
        assert!(!is_task_due(
            &AgentTask {
                enabled: false,
                ..task("hourly")
            },
            None,
            at(9, 0)
        ));
    }

    #[test]
    fn interval_schedules_respect_their_unit() {
        let now = at(12, 0);
        let now_ms = now.timestamp_millis();

        assert!(
            is_task_due(&task("interval:30m"), None, now),
            "从没跑过就该跑"
        );
        assert!(!is_task_due(
            &task("interval:30m"),
            Some(now_ms - 29 * MINUTE),
            now
        ));
        assert!(is_task_due(
            &task("interval:30m"),
            Some(now_ms - 30 * MINUTE),
            now
        ));

        // 不带单位默认分钟
        assert!(is_task_due(
            &task("interval:30"),
            Some(now_ms - 30 * MINUTE),
            now
        ));
        assert!(!is_task_due(&task("interval:2h"), Some(now_ms - HOUR), now));
        assert!(is_task_due(
            &task("interval:2h"),
            Some(now_ms - 2 * HOUR),
            now
        ));
        assert!(is_task_due(
            &task("interval:1d"),
            Some(now_ms - 24 * HOUR),
            now
        ));
    }

    #[test]
    fn malformed_intervals_never_fire() {
        let now = at(12, 0);
        for schedule in [
            "interval:",
            "interval:abc",
            "interval:0",
            "interval:5x",
            "interval:-5",
        ] {
            assert!(
                !is_task_due(&task(schedule), None, now),
                "{schedule} 不该触发"
            );
        }
    }

    #[test]
    fn hourly_fires_once_an_hour() {
        let now = at(12, 0);
        let now_ms = now.timestamp_millis();
        assert!(is_task_due(&task("hourly"), None, now));
        assert!(!is_task_due(
            &task("hourly"),
            Some(now_ms - 59 * MINUTE),
            now
        ));
        assert!(is_task_due(&task("hourly"), Some(now_ms - HOUR), now));
    }

    /// daily 的语义不是「距上次满 24 小时」而是「今天这个点过了且今天还没跑过」。
    #[test]
    fn daily_fires_once_after_its_time_each_day() {
        let before = at(8, 59);
        let after = at(9, 1);
        let target_ms = at(9, 0).timestamp_millis();

        assert!(!is_task_due(&task("daily:09:00"), None, before), "还没到点");
        assert!(
            is_task_due(&task("daily:09:00"), None, after),
            "到点且没跑过"
        );
        assert!(
            !is_task_due(&task("daily:09:00"), Some(target_ms + 60_000), after),
            "今天这个点已经跑过就不再重复"
        );
        assert!(
            is_task_due(&task("daily:09:00"), Some(target_ms - 24 * HOUR), after),
            "昨天跑过不影响今天"
        );
    }

    #[test]
    fn daily_accepts_an_hour_without_minutes_and_rejects_garbage() {
        assert!(
            is_task_due(&task("daily:9"), None, at(9, 30)),
            "省略分钟按 0 分"
        );
        for schedule in ["daily:", "daily:abc", "daily:9:xx"] {
            assert!(
                !is_task_due(&task(schedule), None, at(23, 59)),
                "{schedule} 不该触发"
            );
        }
    }

    #[test]
    fn schedule_matching_is_case_and_space_insensitive() {
        let now = at(12, 0);
        assert!(is_task_due(&task("  HOURLY  "), None, now));
        assert!(is_task_due(&task("Interval:30M"), None, now));
        assert!(!is_task_due(&task(" Manual "), None, now));
    }

    #[test]
    fn an_unknown_schedule_never_fires() {
        assert!(!is_task_due(&task("每天早上"), None, at(9, 0)));
        assert!(!is_task_due(&task("cron:0 9 * * *"), None, at(9, 0)));
    }

    // ---------- 状态与日志 ----------

    /// 状态文件是磁盘契约：两空格缩进 + 尾换行，Electron 版读同一个文件。
    #[test]
    fn the_state_file_matches_the_upstream_byte_shape() {
        let w = ws("state");
        let mut state = TaskState::default();
        state.last_run.insert("每日总结".into(), 1_756_000_000_000);
        write_state(&w.0, &state).unwrap();

        let raw = std::fs::read_to_string(state_path(&w.0)).unwrap();
        assert_eq!(
            raw,
            "{\n  \"version\": 1,\n  \"lastRun\": {\n    \"每日总结\": 1756000000000\n  }\n}\n"
        );
    }

    #[test]
    fn state_round_trips_and_tolerates_a_corrupt_file() {
        let w = ws("state-rt");
        let mut state = TaskState::default();
        state.last_run.insert("a".into(), 123);
        write_state(&w.0, &state).unwrap();
        assert_eq!(read_state(&w.0).last_run.get("a"), Some(&123));

        std::fs::write(state_path(&w.0), "{损坏的").unwrap();
        assert!(
            read_state(&w.0).last_run.is_empty(),
            "损坏就退回空状态，不能让调度停摆"
        );
        assert!(read_state(&ws("state-missing").0).last_run.is_empty());
    }

    /// 先记 lastRun 再跑：任务要跑几十秒，期间再检查一次不能重复触发同一个。
    #[test]
    fn taking_due_tasks_stamps_them_so_they_do_not_double_fire() {
        let w = ws("take");
        w.task("每小时", "---\nschedule: hourly\n---\n干活");
        w.task("手动", "---\nschedule: manual\n---\n干活");

        let now = at(12, 0);
        let first = take_due_tasks(&w.0, now);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].id, "每小时");

        // 同一时刻再问一次，不该再给出来
        assert!(
            take_due_tasks(&w.0, now).is_empty(),
            "同一任务不能被重复触发"
        );
        // 一小时后又该跑了
        assert_eq!(take_due_tasks(&w.0, at(13, 0)).len(), 1);
    }

    #[test]
    fn run_records_are_appended_as_jsonl_omitting_empty_fields() {
        let w = ws("runs");
        append_run(
            &w.0,
            &TaskRunRecord {
                task_id: "t1".into(),
                started_at: 100,
                finished_at: Some(200),
                ok: Some(true),
                summary: Some("干完了".into()),
                error: None,
            },
        )
        .unwrap();
        append_run(
            &w.0,
            &TaskRunRecord {
                task_id: "t2".into(),
                started_at: 300,
                finished_at: Some(400),
                ok: Some(false),
                summary: None,
                error: Some("炸了".into()),
            },
        )
        .unwrap();

        let raw = std::fs::read_to_string(runs_path(&w.0)).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 2, "追加而不是覆盖");
        assert_eq!(
            lines[0],
            r#"{"taskId":"t1","startedAt":100,"finishedAt":200,"ok":true,"summary":"干完了"}"#,
            "error 为空时整个键省略"
        );
        assert!(lines[1].contains(r#""error":"炸了""#));
        assert!(!lines[1].contains("summary"));
    }

    // ---------- 自治级别 ----------

    #[test]
    fn read_only_tasks_only_get_read_only_tools() {
        let all = crate::ai::tools::definitions::all().to_vec();
        let filtered = filter_tools_for_autonomy(&all, TaskAutonomy::ReadOnly);

        let names: Vec<&str> = filtered.iter().map(|t| t.function.name.as_str()).collect();
        assert!(names.contains(&"file_read"));
        assert!(names.contains(&"content_search"));
        assert!(names.contains(&"memory_search"));
        assert!(names.contains(&"workflow_get"));
        assert!(names.contains(&"desktop_cards_get"));
        for writer in [
            "file_write",
            "file_delete",
            "path_rename",
            "git_commit",
            "settings_update",
            "shell_run",
            "workflow_batch",
            "workflow_set_schedule",
            "workflow_delete",
            "workflow_folders",
            "desktop_cards_batch",
        ] {
            assert!(!names.contains(&writer), "只读模式下不该有 {writer}");
        }
        assert!(filtered.len() < all.len());
    }

    #[test]
    fn the_other_autonomy_levels_keep_every_tool() {
        let all = crate::ai::tools::definitions::all().to_vec();
        assert_eq!(
            filter_tools_for_autonomy(&all, TaskAutonomy::Whitelisted).len(),
            all.len()
        );
        assert_eq!(
            filter_tools_for_autonomy(&all, TaskAutonomy::ProposeOnly).len(),
            all.len()
        );
    }

    /// 外部 MCP 工具一律放行——核心层无从判断它们的读写性质，与 TS 一致。
    #[test]
    fn external_mcp_tools_pass_the_read_only_filter() {
        let mcp = AiToolDefinition::function(
            "mcp__github__list_issues",
            "列 issue",
            serde_json::json!({}),
        );
        let other =
            AiToolDefinition::function("some_write_tool", "写点什么", serde_json::json!({}));
        let filtered = filter_tools_for_autonomy(&[mcp, other], TaskAutonomy::ReadOnly);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].function.name, "mcp__github__list_issues");
    }

    /// 只读清单里的名字必须都真实存在，否则那条限制形同虚设。
    #[test]
    fn every_read_only_tool_name_exists() {
        for name in READ_ONLY_TOOLS {
            assert!(
                crate::ai::tools::definitions::find(name).is_some(),
                "只读清单里的 {name} 在工具定义表里不存在"
            );
        }
    }

    // ---------- 无人值守提示 ----------

    #[test]
    fn the_unattended_suffix_states_nobody_is_watching() {
        let read_only = unattended_prompt_suffix("每日总结", TaskAutonomy::ReadOnly);
        assert!(read_only.contains("每日总结"));
        assert!(
            read_only.contains("用户不在场"),
            "不说这句模型会反问然后干等"
        );
        assert!(read_only.contains("只读模式"));

        let proposing = unattended_prompt_suffix("整理", TaskAutonomy::ProposeOnly);
        assert!(proposing.contains("审批收件箱"));
        assert!(!proposing.contains("只读模式"));
    }
}
