//! 为数据存放在工作区之外的模块提供有界的读取。
//!
//! 自动化存放在工作流 SQLite 库里，番茄钟历史记录在分析 JSONL 流里，
//! 考试则是带 `:::exam` JSON 块的 Markdown 文件。这些提供方刻意只读：
//! 桌面卡片每次刷新都不会创建资料库、迁移数据结构或改动考试文件。

use super::{Action, Module, Page, PageSnapshot, Row, MAX_ROWS_PER_PAGE};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, NaiveTime, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use std::cmp::Reverse;
use std::fs::{self, DirEntry, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const HARD_MAX_READ_BYTES: usize = 8 * 1024 * 1024;
const MAX_EXAM_DEPTH: usize = 5;
const MAX_SCAN_VISITS: usize = 4096;
const MAX_ROW_CHARS: usize = 240;

/// 为自动化、番茄钟或考试构建快照。
///
/// 只处理那些「普通提供方无法用通用目录列表暴露真实来源」的模块。
/// 每次文件读取、SQL 查询和目录遍历都有调用方给的界限外加硬上限。
pub fn extra_snapshot(
    root: &Path,
    page: &Page,
    max_read_bytes: usize,
    max_entries: usize,
) -> Result<PageSnapshot> {
    if page.options.is_empty() {
        return Ok(PageSnapshot::empty(page));
    }
    anyhow::ensure!(max_read_bytes > 0, "来源读取上限必须大于 0");
    anyhow::ensure!(max_entries > 0, "来源条目上限必须大于 0");

    match page.module {
        Module::Automations => automations_snapshot(root, page, max_read_bytes, max_entries),
        Module::Pomodoro => pomodoro_snapshot(root, page, max_read_bytes, max_entries),
        Module::Exam => exam_snapshot(root, page, max_read_bytes, max_entries),
        module => bail!("{} 模块不支持扩展来源快照", module.label()),
    }
}

fn automations_snapshot(
    root: &Path,
    page: &Page,
    max_read_bytes: usize,
    max_entries: usize,
) -> Result<PageSnapshot> {
    let mut snapshot = PageSnapshot::empty(page);
    let row_limit = max_entries.min(MAX_ROWS_PER_PAGE);
    let db_path = root.join(".mochi/workflows/workflows.sqlite3");

    // 新工作区还没有工作流数据库。这是货真价实的空状态；
    // 而数据库存在但损坏，就必须暴露出来。
    if !db_path.exists() {
        snapshot.empty_message = "还没有配置自动化".into();
        return Ok(snapshot);
    }
    let db_path = canonical_internal_file(root, &db_path, "自动化数据库")?;
    check_storage_size(root, &db_path, max_read_bytes, "自动化数据库")?;
    let db = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("无法只读打开自动化数据库: {}", db_path.display()))?;
    db.busy_timeout(std::time::Duration::from_millis(250))
        .context("无法设置自动化数据库读取超时")?;

    let mut workflows = read_workflows(&db, row_limit)?;
    let mut rows = Vec::new();
    let now = crate::jstime::now_millis();

    let show_enabled = page.selected("enabled");
    let show_next = page.selected("nextRun");
    let show_recent = page.selected("recent");
    // 卡片常常三个默认分区全开。给运行历史留一半行数，
    // 免得一条很长的自动化列表把选中的 `recent` 区挤没。
    // 没选自动化分区时，历史独占整张卡。
    let recent_reservation = if show_recent && (show_enabled || show_next) {
        row_limit.div_ceil(2)
    } else {
        0
    };
    let workflow_limit = row_limit.saturating_sub(recent_reservation);

    if show_enabled || show_next {
        let mut workflow_rows = 0;
        for workflow in workflows
            .iter()
            .filter(|workflow| show_enabled || (show_next && workflow.enabled))
        {
            if workflow_rows >= workflow_limit {
                break;
            }
            let status = if workflow.enabled {
                "已启用"
            } else if workflow.approved {
                "已停用"
            } else {
                "待授权"
            };
            let mut details = Vec::new();
            if show_enabled {
                details.push(status.to_owned());
            }
            details.push(format!("{} 个步骤", workflow.definition.nodes.len()));
            if show_next && workflow.enabled {
                details.push(format!(
                    "下次运行：{}",
                    next_run_label(&workflow.definition.trigger, now)
                ));
            }
            if show_enabled {
                details.push(updated_detail(workflow.updated_at));
            }
            rows.push(Row {
                id: format!("automation:workflow:{}", workflow.definition.id),
                title: nonempty_or(&workflow.definition.name, "未命名自动化").into(),
                detail: clip(&details.join(" · "), MAX_ROW_CHARS),
                action: Some(Action::OpenModule(Module::Automations.wire_name().into())),
                checked: None,
                meta: Default::default(),
            });
            workflow_rows += 1;
        }
    }

    if show_recent && rows.len() < row_limit {
        let names = workflows
            .drain(..)
            .map(|workflow| (workflow.definition.id, workflow.definition.name))
            .collect::<std::collections::BTreeMap<_, _>>();
        let query_limit = row_limit.saturating_sub(rows.len());
        let mut statement = db
            .prepare(
                "SELECT workflow_id,status,started_at,finished_at,source
                 FROM runs ORDER BY started_at DESC LIMIT ?1",
            )
            .context("无法读取自动化运行历史")?;
        let recent = statement
            .query_map([query_limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .context("无法读取自动化运行记录")?;
        for item in recent {
            let (workflow_id, status, started_at, finished_at, source) =
                item.context("自动化运行记录格式无效")?;
            let name = names
                .get(&workflow_id)
                .map(String::as_str)
                .filter(|name| !name.trim().is_empty())
                .unwrap_or("未命名自动化");
            let elapsed = finished_at
                .map(|end| format!(" · {} 秒", (end - started_at).max(0) / 1000))
                .unwrap_or_default();
            rows.push(Row {
                id: format!("automation:run:{workflow_id}:{started_at}"),
                title: name.into(),
                detail: clip(
                    &format!("{} · 来源 {}{}", run_status_label(&status), source, elapsed),
                    MAX_ROW_CHARS,
                ),
                action: Some(Action::OpenModule(Module::Automations.wire_name().into())),
                checked: None,
                meta: Default::default(),
            });
        }
    }

    snapshot.rows = rows;
    if snapshot.rows.is_empty() {
        snapshot.empty_message = "没有符合条件的自动化内容".into();
    }
    Ok(snapshot)
}

#[derive(Debug)]
struct StoredWorkflow {
    definition: crate::workflows::Workflow,
    approved: bool,
    enabled: bool,
    updated_at: i64,
}

fn read_workflows(db: &Connection, limit: usize) -> Result<Vec<StoredWorkflow>> {
    let mut statement = db
        .prepare(
            "SELECT definition,revision,approved_hash,enabled,updated_at
             FROM workflows ORDER BY updated_at DESC LIMIT ?1",
        )
        .context("无法读取自动化配置")?;
    let rows = statement
        .query_map([limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .context("无法读取自动化配置记录")?;
    let mut workflows = Vec::new();
    for row in rows {
        let (definition, approved_hash, enabled, updated_at) =
            row.context("自动化配置记录格式无效")?;
        let definition: crate::workflows::Workflow =
            serde_json::from_str(&definition).context("自动化配置中的工作流 JSON 无效")?;
        let approved =
            approved_hash.as_deref() == Some(crate::workflows::fingerprint(&definition).as_str());
        workflows.push(StoredWorkflow {
            definition,
            approved,
            enabled: enabled && approved,
            updated_at,
        });
    }
    Ok(workflows)
}

fn next_run_label(trigger: &crate::workflows::Trigger, now_millis: i64) -> String {
    match trigger {
        crate::workflows::Trigger::Manual => "手动触发".into(),
        crate::workflows::Trigger::Interval { minutes } => {
            let interval = i64::from(*minutes).saturating_mul(60_000);
            if interval == 0 {
                return "时间配置无效".into();
            }
            let next = now_millis
                .div_euclid(interval)
                .saturating_add(1)
                .saturating_mul(interval);
            crate::jstime::from_millis(next).unwrap_or_else(|| "时间配置无效".into())
        }
        crate::workflows::Trigger::Daily {
            time,
            utc_offset_minutes,
        } => next_calendar_run(now_millis, time, *utc_offset_minutes, None),
        crate::workflows::Trigger::Weekly {
            time,
            weekdays,
            utc_offset_minutes,
        } => next_calendar_run(
            now_millis,
            time,
            *utc_offset_minutes,
            Some(weekdays.as_slice()),
        ),
    }
}

fn next_calendar_run(
    now_millis: i64,
    time: &str,
    utc_offset_minutes: i32,
    weekdays: Option<&[u32]>,
) -> String {
    let Some(parsed_time) = NaiveTime::parse_from_str(time, "%H:%M").ok() else {
        return "时间配置无效".into();
    };
    let Some(offset) = FixedOffset::east_opt(utc_offset_minutes.saturating_mul(60)) else {
        return "时区配置无效".into();
    };
    let Some(now_utc) = DateTime::<Utc>::from_timestamp_millis(now_millis) else {
        return "时间配置无效".into();
    };
    let local = now_utc.with_timezone(&offset);
    for days in 0..=7 {
        let date = local.date_naive() + Duration::days(days);
        let weekday = date.weekday().number_from_monday();
        if weekdays.is_some_and(|allowed| !allowed.contains(&weekday)) {
            continue;
        }
        let candidate = date.and_time(parsed_time);
        if days == 0 && candidate <= local.naive_local() {
            continue;
        }
        let Some(candidate) = offset.from_local_datetime(&candidate).single() else {
            continue;
        };
        return crate::jstime::from_millis(candidate.with_timezone(&Utc).timestamp_millis())
            .unwrap_or_else(|| "时间配置无效".into());
    }
    "暂无下次运行".into()
}

fn run_status_label(status: &str) -> &'static str {
    match status {
        "queued" => "排队中",
        "running" => "运行中",
        "success" | "succeeded" => "已成功",
        "failed" => "失败",
        "partial" => "部分成功",
        "cancelled" => "已取消",
        "interrupted" => "已中断",
        _ => "状态未知",
    }
}

fn updated_detail(millis: i64) -> String {
    crate::jstime::from_millis(millis).unwrap_or_else(|| "修改时间未知".into())
}

fn pomodoro_snapshot(
    root: &Path,
    page: &Page,
    max_read_bytes: usize,
    max_entries: usize,
) -> Result<PageSnapshot> {
    let mut snapshot = PageSnapshot::empty(page);
    let row_limit = max_entries.min(MAX_ROWS_PER_PAGE);
    let now = Local::now();
    let start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Local.from_local_datetime(&value).single())
        .unwrap_or(now);
    let activity_path = root.join(".mochi").join("activity").join(format!(
        "{}.jsonl",
        crate::analytics::events::month_key(now)
    ));
    let events = if activity_path.exists() {
        let activity_path = canonical_internal_file(root, &activity_path, "专注活动记录")?;
        read_activity_file(&activity_path, max_read_bytes)?
            .into_iter()
            .filter(|event| {
                event.timestamp_millis().is_some_and(|millis| {
                    millis >= start.timestamp_millis() && millis <= now.timestamp_millis()
                }) && event.kind == crate::analytics::events::ActivityEventType::FocusSession
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let sessions = events.len();
    let minutes: f64 = events
        .iter()
        .map(|event| event.duration_minutes.unwrap_or(0.0).max(0.0))
        .sum();
    let latest = events
        .iter()
        .filter_map(|event| event.timestamp_millis())
        .max()
        .and_then(crate::jstime::from_millis);
    let action = Some(Action::OpenModule(Module::Pomodoro.wire_name().into()));
    let mut rows = Vec::new();

    if page.selected("status") && rows.len() < row_limit {
        let detail = match latest {
            Some(time) => format!("空闲 · 最近完成于 {time}；今日 {sessions} 轮"),
            None => "空闲 · 今日尚未完成专注".into(),
        };
        rows.push(Row {
            id: "pomodoro:status".into(),
            title: "空闲".into(),
            detail,
            action: action.clone(),
            checked: None,
            meta: Default::default(),
        });
    }
    if page.selected("today") && rows.len() < row_limit {
        rows.push(Row {
            id: "pomodoro:today".into(),
            title: "今日次数".into(),
            detail: format!("{sessions} 轮 · {} 分钟", format_number(minutes)),
            action,
            checked: None,
            meta: Default::default(),
        });
    }

    snapshot.rows = rows;
    if snapshot.rows.is_empty() {
        snapshot.empty_message = "没有启用的番茄钟内容".into();
    }
    Ok(snapshot)
}

fn read_activity_file(
    path: &Path,
    max_read_bytes: usize,
) -> Result<Vec<crate::analytics::events::ActivityEvent>> {
    let text = read_text(path, max_read_bytes)?;
    let mut events = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        events.push(serde_json::from_str(line).with_context(|| {
            format!(
                "活动记录第 {} 行格式无效: {}",
                line_number + 1,
                path.display()
            )
        })?);
    }
    Ok(events)
}

#[derive(Debug)]
struct ExamSummary {
    relative: String,
    modified: Option<SystemTime>,
    title: String,
    question_count: usize,
    attempts: usize,
    latest_score: Option<u64>,
    due: bool,
}

fn exam_snapshot(
    root: &Path,
    page: &Page,
    max_read_bytes: usize,
    max_entries: usize,
) -> Result<PageSnapshot> {
    let mut snapshot = PageSnapshot::empty(page);
    let row_limit = max_entries.min(MAX_ROWS_PER_PAGE);
    let paths = exam_paths(root, page, max_entries)?;
    let mut exams = Vec::new();
    for path in paths {
        exams.push(read_exam_summary(root, &path, max_read_bytes)?);
    }
    exams.sort_by_key(|exam| Reverse(exam.modified));
    let recent = page.selected("recent");
    let due = page.selected("due");
    let mut selected = exams
        .into_iter()
        .filter(|exam| recent || (due && exam.due))
        .collect::<Vec<_>>();
    if due && recent {
        // 小卡片里未完成的工作保持可见，同时组内按修改时间保持确定顺序。
        selected.sort_by_key(|exam| (Reverse(exam.due), Reverse(exam.modified)));
    } else if due {
        selected.retain(|exam| exam.due);
    }

    let mut rows = Vec::new();
    for exam in selected.into_iter().take(row_limit) {
        let mut detail = format!("{} 题 · {} 次作答", exam.question_count, exam.attempts);
        if let Some(score) = exam.latest_score {
            detail.push_str(&format!(" · 最近 {} 分", score));
        } else {
            detail.push_str(" · 未作答");
        }
        if exam.due {
            detail.push_str(" · 待完成");
        }
        rows.push(Row {
            id: format!("exam:{}", exam.relative),
            title: exam.title,
            detail: clip(&detail, MAX_ROW_CHARS),
            action: Some(Action::OpenFile(exam.relative)),
            checked: Some(!exam.due),
            meta: Default::default(),
        });
    }
    snapshot.rows = rows;
    if snapshot.rows.is_empty() {
        snapshot.empty_message = if due {
            "没有待完成的试卷".into()
        } else {
            "没有可展示的试卷".into()
        };
    }
    Ok(snapshot)
}

fn exam_paths(root: &Path, page: &Page, max_entries: usize) -> Result<Vec<PathBuf>> {
    let root_canonical = root
        .canonicalize()
        .with_context(|| format!("工作区不存在或无法读取: {}", root.display()))?;
    let source = if let Some(source) = page.source.as_deref() {
        super::safe_source_path(root, source)?
    } else {
        root_canonical.clone()
    };
    let metadata = fs::metadata(&source)
        .with_context(|| format!("试卷来源不存在或无法读取: {}", source.display()))?;
    if metadata.is_file() {
        anyhow::ensure!(is_exam_file(&source), "试卷来源必须是 .exam 文件");
        return Ok(vec![source]);
    }
    anyhow::ensure!(metadata.is_dir(), "试卷来源不是目录: {}", source.display());

    let max_files = max_entries.saturating_mul(16).clamp(32, 512);
    let mut budget = ScanBudget::new(MAX_SCAN_VISITS.min(max_files.saturating_mul(8)));
    let mut files = Vec::new();
    collect_exam_files(
        &source,
        &root_canonical,
        0,
        max_files,
        &mut budget,
        &mut files,
    )?;
    files.sort();
    Ok(files)
}

fn collect_exam_files(
    directory: &Path,
    root: &Path,
    depth: usize,
    max_files: usize,
    budget: &mut ScanBudget,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    if depth > MAX_EXAM_DEPTH || files.len() >= max_files || !budget.spend() {
        return Ok(());
    }
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("无法读取试卷目录: {}", directory.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("读取试卷目录失败: {}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if files.len() >= max_files || !budget.spend() {
            break;
        }
        let canonical = match canonical_entry(&entry, root) {
            Ok(path) => path,
            Err(_) => continue,
        };
        let metadata = match fs::metadata(&canonical) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.is_dir() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            collect_exam_files(&canonical, root, depth + 1, max_files, budget, files)?;
        } else if metadata.is_file() && is_exam_file(&canonical) {
            files.push(canonical);
        }
    }
    Ok(())
}

fn read_exam_summary(root: &Path, path: &Path, max_read_bytes: usize) -> Result<ExamSummary> {
    let text = read_text(path, max_read_bytes)?;
    let blocks = crate::exam::parse(&text);
    anyhow::ensure!(
        !blocks.is_empty(),
        "试卷没有有效的 :::exam 区块: {}",
        path.display()
    );
    let mut title = None;
    let mut question_count = 0;
    let mut attempts = 0;
    let mut latest_score = None;
    let mut latest_submission = i64::MIN;
    let mut due = false;
    for block in blocks {
        let model = &block.model;
        if title.is_none() {
            title = model["title"].as_str().map(str::to_owned);
        }
        question_count += model["questions"].as_array().map_or(0, Vec::len);
        let history = model["history"].as_array().cloned().unwrap_or_default();
        attempts += history.len();
        if history.is_empty() {
            due = true;
        }
        for entry in history {
            let submitted = entry["submitted_at"].as_i64().unwrap_or(0);
            if submitted >= latest_submission {
                latest_submission = submitted;
                latest_score = entry["score"].as_u64();
            }
        }
        if latest_score.is_some_and(|score| score < 100) {
            due = true;
        }
    }
    let relative = relative_path(root, path)?;
    let metadata =
        fs::metadata(path).with_context(|| format!("无法读取试卷元数据: {}", path.display()))?;
    Ok(ExamSummary {
        relative,
        modified: metadata.modified().ok(),
        title: title
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                path.file_stem()
                    .and_then(|value| value.to_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "试卷".into()),
        question_count,
        attempts,
        latest_score,
        due,
    })
}

fn read_text(path: &Path, max_read_bytes: usize) -> Result<String> {
    let limit = max_read_bytes.min(HARD_MAX_READ_BYTES);
    anyhow::ensure!(limit > 0, "来源读取上限必须大于 0");
    let metadata =
        fs::metadata(path).with_context(|| format!("来源不存在或无法读取: {}", path.display()))?;
    anyhow::ensure!(metadata.is_file(), "来源不是文件: {}", path.display());
    anyhow::ensure!(
        metadata.len() <= limit as u64,
        "来源文件超过读取上限（{} 字节）: {}",
        limit,
        path.display()
    );
    let file = File::open(path).with_context(|| format!("无法打开来源文件: {}", path.display()))?;
    let mut bytes = Vec::with_capacity(metadata.len().min(limit as u64) as usize);
    file.take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("无法读取来源文件: {}", path.display()))?;
    anyhow::ensure!(
        bytes.len() <= limit,
        "来源文件在读取期间超过上限: {}",
        path.display()
    );
    String::from_utf8(bytes)
        .map(|text| text.trim_start_matches('\u{feff}').to_owned())
        .with_context(|| format!("来源文件不是 UTF-8 文本: {}", path.display()))
}

fn canonical_internal_file(root: &Path, path: &Path, label: &str) -> Result<PathBuf> {
    let root = root
        .canonicalize()
        .with_context(|| format!("工作区不存在或无法读取: {}", root.display()))?;
    let canonical = path
        .canonicalize()
        .with_context(|| format!("无法读取{label}: {}", path.display()))?;
    anyhow::ensure!(
        crate::paths::path_is_within(&root, &canonical),
        "{label}路径越出工作区: {}",
        path.display()
    );
    anyhow::ensure!(canonical.is_file(), "{label}不是文件: {}", path.display());
    Ok(canonical)
}

fn check_storage_size(root: &Path, path: &Path, max_read_bytes: usize, label: &str) -> Result<()> {
    let limit = max_read_bytes.min(HARD_MAX_READ_BYTES);
    let root = root
        .canonicalize()
        .with_context(|| format!("工作区不存在或无法读取: {}", root.display()))?;
    let metadata =
        fs::metadata(path).with_context(|| format!("无法读取{label}: {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= limit as u64,
        "{label}超过读取上限（{} 字节）: {}",
        limit,
        path.display()
    );
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{}", path.display(), suffix));
        if let Ok(meta) = fs::metadata(&sidecar) {
            let canonical_sidecar = sidecar
                .canonicalize()
                .with_context(|| format!("无法读取{label}辅助文件: {}", sidecar.display()))?;
            anyhow::ensure!(
                crate::paths::path_is_within(&root, &canonical_sidecar),
                "{label}辅助文件路径越出工作区: {}",
                sidecar.display()
            );
            anyhow::ensure!(
                meta.len() <= limit as u64,
                "{label}辅助文件超过读取上限（{} 字节）: {}",
                limit,
                sidecar.display()
            );
        }
    }
    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    let root = root
        .canonicalize()
        .with_context(|| format!("工作区不存在或无法读取: {}", root.display()))?;
    let path = path
        .canonicalize()
        .with_context(|| format!("来源不存在或无法读取: {}", path.display()))?;
    anyhow::ensure!(
        crate::paths::path_is_within(&root, &path),
        "来源路径越出工作区"
    );
    let relative = path
        .strip_prefix(&root)
        .context("来源路径无法转换为工作区相对路径")?
        .to_string_lossy()
        .replace('\\', "/");
    anyhow::ensure!(!relative.is_empty(), "来源不能是工作区根目录");
    Ok(relative)
}

fn canonical_entry(entry: &DirEntry, root: &Path) -> Result<PathBuf> {
    let path = entry.path().canonicalize()?;
    anyhow::ensure!(
        crate::paths::path_is_within(root, &path),
        "目录条目越出工作区"
    );
    Ok(path)
}

fn is_exam_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exam"))
}

fn nonempty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

fn clip(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let clipped = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{clipped}…")
    } else {
        clipped
    }
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value:.1}")
    }
}

struct ScanBudget {
    remaining: usize,
}

impl ScanBudget {
    fn new(remaining: usize) -> Self {
        Self { remaining }
    }

    fn spend(&mut self) -> bool {
        if self.remaining == 0 {
            false
        } else {
            self.remaining -= 1;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_cards::Module;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    fn workspace(tag: &str) -> PathBuf {
        let id = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("mochi-extra-provider-{tag}-{id}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn page(module: Module, options: &[&str]) -> Page {
        let mut page = Page::with_id(module, "test-page");
        page.options = options.iter().map(|value| (*value).into()).collect();
        page
    }

    #[test]
    fn automations_reads_the_workflow_sqlite_schema_without_writing() {
        let root = workspace("automations");
        let store = crate::workflows::Store::open(&root).unwrap();
        let mut workflow = crate::workflows::Workflow::blank();
        workflow.name = "每日整理".into();
        workflow.trigger = crate::workflows::Trigger::Interval { minutes: 5 };
        let saved = store.save(&workflow, None).unwrap();
        store.approve(&workflow.id, saved.revision, true).unwrap();
        store
            .enqueue(&workflow.id, serde_json::json!({}), "test", None)
            .unwrap();
        let db_path = root.join(".mochi/workflows/workflows.sqlite3");
        let before = fs::metadata(&db_path).unwrap().len();
        let snapshot = extra_snapshot(
            &root,
            &page(Module::Automations, &["enabled", "nextRun", "recent"]),
            512 * 1024,
            12,
        )
        .unwrap();
        assert!(snapshot.rows.iter().any(|row| row.title == "每日整理"));
        assert!(snapshot
            .rows
            .iter()
            .any(|row| row.detail.contains("下次运行")));
        assert!(snapshot
            .rows
            .iter()
            .any(|row| row.detail.contains("排队中")));
        assert_eq!(fs::metadata(&db_path).unwrap().len(), before);
        drop(store);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pomodoro_uses_focus_sessions_from_today_activity_log() {
        let root = workspace("pomodoro");
        let now = crate::jstime::now();
        let dir = root.join(".mochi/activity");
        fs::create_dir_all(&dir).unwrap();
        let month = crate::analytics::events::month_key(Local::now());
        fs::write(
            dir.join(format!("{month}.jsonl")),
            format!(
                "{{\"version\":1,\"id\":\"focus-1\",\"type\":\"focus_session\",\"timestamp\":\"{now}\",\"durationMinutes\":25}}\n"
            ),
        )
        .unwrap();
        let snapshot = extra_snapshot(
            &root,
            &page(Module::Pomodoro, &["status", "today"]),
            4096,
            12,
        )
        .unwrap();
        assert!(snapshot.rows.iter().any(|row| row.title == "空闲"));
        assert!(snapshot.rows.iter().any(|row| row.detail.contains("1 轮")));
        assert!(snapshot
            .rows
            .iter()
            .any(|row| row.detail.contains("25 分钟")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn exam_provider_summarizes_real_exam_blocks_and_due_state() {
        let root = workspace("exam");
        fs::write(
            root.join("待复习.exam"),
            ":::exam\n{\"version\":\"1.0\",\"title\":\"算法复习\",\"questions\":[{\"id\":\"q1\",\"type\":\"single_choice\",\"stem\":\"复杂度\",\"options\":[\"O(1)\",\"O(n)\"],\"answer\":0}],\"history\":[]}\n:::\n",
        )
        .unwrap();
        let snapshot =
            extra_snapshot(&root, &page(Module::Exam, &["recent", "due"]), 4096, 12).unwrap();
        assert_eq!(snapshot.rows.len(), 1);
        assert_eq!(snapshot.rows[0].title, "算法复习");
        assert!(snapshot.rows[0].detail.contains("待完成"));
        assert_eq!(
            snapshot.rows[0].action,
            Some(Action::OpenFile("待复习.exam".into()))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn oversized_activity_and_invalid_exam_are_errors() {
        let root = workspace("errors");
        let dir = root.join(".mochi/activity");
        fs::create_dir_all(&dir).unwrap();
        let month = crate::analytics::events::month_key(Local::now());
        fs::write(dir.join(format!("{month}.jsonl")), "0123456789").unwrap();
        assert!(extra_snapshot(&root, &page(Module::Pomodoro, &["today"]), 4, 12).is_err());
        fs::write(root.join("bad.exam"), ":::exam\n{bad}\n:::\n").unwrap();
        assert!(extra_snapshot(&root, &page(Module::Exam, &["recent"]), 4096, 12,).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
