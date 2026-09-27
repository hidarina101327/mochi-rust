//! 自动化界面的宿主逻辑：本地授权、后台规划和受保护的持久化。
use super::*;
use anyhow::Context;
use mochi_core::{base as model, base_automation as engine};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;

mod panel;
pub(super) use panel::TextEdit;
const GRANTS_KEY: &str = "native.baseAutomationGrants";

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Grant {
    path: PathBuf,
    table_id: String,
    view_id: String,
    rule_id: String,
    signature: String,
}
type Grants = BTreeMap<String, Grant>;
fn grants(settings: &SettingsService) -> Grants {
    settings
        .get(GRANTS_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
fn grant_key(path: &Path, table: &str, view: &str, rule: &str) -> String {
    engine::hash(&(path.to_string_lossy(), table, view, rule))
}
fn authorized(grants: &Grants, path: &Path) -> BTreeSet<String> {
    grants
        .values()
        .filter(|g| g.path == path)
        .map(|g| g.signature.clone())
        .collect()
}

#[derive(Default)]
pub(super) struct State {
    pub armed: bool,
    pub panel: Option<panel::Panel>,
    root: Option<PathBuf>,
    cache: BTreeMap<PathBuf, Cache>,
    job: Option<mpsc::Receiver<Batch>>,
    cursor: usize,
    last_error: String,
}
#[derive(Clone, Default)]
struct Cache {
    stamp: Option<(std::time::SystemTime, u64)>,
    next_due: u64,
    snapshot: engine::Snapshot,
    consent: BTreeSet<String>,
}
struct Plan {
    path: PathBuf,
    original: String,
    document: model::BaseDocument,
    content: Option<String>,
    cache: Cache,
}
struct Batch {
    root: PathBuf,
    plans: Vec<Plan>,
    errors: Vec<String>,
}

fn stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}
fn in_workspace(path: &Path, root: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mcb"))
        && path
            .canonicalize()
            .ok()
            .zip(root.canonicalize().ok())
            .is_some_and(|(p, r)| p.starts_with(r))
}

fn plan_file(
    path: &Path,
    cache: &Cache,
    consent: BTreeSet<String>,
    now: u64,
) -> anyhow::Result<Option<Plan>> {
    let current_stamp = stamp(path).context("无法读取自动化文件")?;
    anyhow::ensure!(
        current_stamp.1 <= engine::MAX_FILE_BYTES,
        "自动化文件超过 10 MiB"
    );
    if cache.stamp == Some(current_stamp) && cache.consent == consent && now < cache.next_due {
        return Ok(None);
    }
    let original = std::fs::read_to_string(path)?;
    let document = model::parse_base_document(&original)?;
    let (result, snapshot) = engine::evaluate(&document, &cache.snapshot, &consent, now)?;
    let content = if result != document {
        Some(model::serialize_base_document(&result)?)
    } else {
        None
    };
    let mut next_due = u64::MAX;
    for table in &result.tables {
        for view in &table.views {
            for rule in engine::rules(view)? {
                if !rule.enabled
                    || !consent.contains(&engine::signature(&result, table, view, &rule))
                {
                    continue;
                }
                if let engine::Trigger::Interval { minutes, start_at } = rule.trigger {
                    if minutes > 0 {
                        let interval = u64::from(minutes) * 60_000;
                        let next = if now < start_at {
                            start_at
                        } else {
                            start_at + ((now - start_at) / interval + 1) * interval
                        };
                        next_due = next_due.min(next);
                    }
                }
            }
        }
    }
    Ok(Some(Plan {
        path: path.into(),
        original,
        document: result,
        content,
        cache: Cache {
            stamp: Some(current_stamp),
            next_due,
            snapshot,
            consent,
        },
    }))
}

impl App {
    pub(super) fn automation_timer(&mut self) {
        self.automation.armed = false;
        let root = self.shell.workspace().map(|w| w.root.clone());
        if self.automation.root != root {
            self.automation.root = root.clone();
            self.automation.cache.clear();
            self.automation.job = None;
        }
        let Some(root) = root else {
            return;
        };
        let consent = grants(&self.settings);
        if let Some(job) = &self.automation.job {
            match job.try_recv() {
                Ok(batch) => {
                    self.automation.job = None;
                    if batch.root == root {
                        for mut plan in batch.plans {
                            if self.automation_file_busy(&plan.path)
                                || authorized(&consent, &plan.path) != plan.cache.consent
                                || !in_workspace(&plan.path, &root)
                            {
                                continue;
                            }
                            if let Some(content) = &plan.content {
                                match self.persist_automation(
                                    &plan.path,
                                    &plan.original,
                                    content,
                                    &plan.document,
                                ) {
                                    Ok(()) => {
                                        plan.cache.stamp = stamp(&plan.path);
                                        self.publish_notification(
                                            crate::ui::notifications::Category::Automation,
                                            "表格自动化已完成",
                                            &format!(
                                                "{} 的自动化修改已保存。",
                                                plan.path
                                                    .file_name()
                                                    .unwrap_or_default()
                                                    .to_string_lossy()
                                            ),
                                        );
                                        self.invalidate_main();
                                    }
                                    Err(error) => {
                                        self.automation_error(&error.to_string());
                                        continue;
                                    }
                                }
                            } else if stamp(&plan.path) != plan.cache.stamp {
                                continue;
                            }
                            self.automation.cache.insert(plan.path, plan.cache);
                        }
                        for error in batch.errors {
                            self.automation_error(&error);
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(_) => {
                    self.automation.job = None;
                    self.automation_error("自动化后台任务意外退出，下次检查会重试");
                }
            }
        }
        // 只监视已获授权的文件，不递归扫描用户的知识库。
        let paths: Vec<_> = consent
            .values()
            .map(|g| g.path.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|p| in_workspace(p, &root))
            .collect();
        if paths.is_empty() {
            self.automation.cache.clear();
            return;
        }
        self.automation.cache.retain(|p, _| paths.contains(p));
        let mut work = vec![];
        for offset in 0..paths.len().min(8) {
            let path = &paths[(self.automation.cursor + offset) % paths.len()];
            if !self.automation_file_busy(path) {
                work.push((
                    path.clone(),
                    self.automation.cache.get(path).cloned().unwrap_or_default(),
                    authorized(&consent, path),
                ));
            }
        }
        self.automation.cursor = (self.automation.cursor + paths.len().min(8)) % paths.len();
        if work.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.automation.job = Some(rx);
        std::thread::spawn(move || {
            let mut batch = Batch {
                root,
                plans: vec![],
                errors: vec![],
            };
            for (path, cache, consent) in work {
                match plan_file(&path, &cache, consent, engine::now_ms()) {
                    Ok(Some(plan)) => batch.plans.push(plan),
                    Ok(None) => {}
                    Err(error) => batch.errors.push(format!(
                        "{}：{error}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    )),
                }
            }
            let _ = tx.send(batch);
        });
    }

    fn automation_error(&mut self, error: &str) {
        if self.automation.last_error != error {
            self.automation.last_error = error.into();
            self.show_global_notice(&format!("自动化未写入：{error}"));
        }
    }
    fn automation_file_busy(&self, path: &Path) -> bool {
        self.automation.panel.as_ref().is_some_and(|p| p.path == path)
            || self.shell.tabs().iter().any(|tab| tab.path().is_some_and(|p| same_path(p,path)) && tab.dirty())
            || self.dialog.as_ref().is_some_and(|d| d.buttons.iter().any(|b| matches!(&b.action,DialogAction::BaseEdit { path: p, .. } if same_path(p,path))))
    }
    fn persist_automation(
        &mut self,
        path: &Path,
        original: &str,
        content: &str,
        document: &model::BaseDocument,
    ) -> anyhow::Result<()> {
        let _gate = engine::MUTATION_GATE
            .try_lock()
            .map_err(|_| anyhow::anyhow!("另一个表格操作正在保存，请稍后重试"))?;
        let _file_lock = engine::FileLock::acquire(path)?;
        anyhow::ensure!(
            self.shell
                .workspace()
                .is_some_and(|w| in_workspace(path, &w.root)),
            "文件不在当前工作区"
        );
        anyhow::ensure!(
            !self
                .shell
                .tabs()
                .iter()
                .any(|t| t.path().is_some_and(|p| same_path(p, path)) && t.dirty()),
            "文档有未保存修改，请先保存"
        );
        anyhow::ensure!(
            std::fs::read_to_string(path)? == original,
            "表格已被修改，请重新打开自动化面板后再试"
        );
        mochi_core::files::FileService::new().write_file_safe(path, content)?;
        for tab in self.shell.tabs_mut() {
            if let TabKind::Viewer {
                path: open,
                content: viewer::Content::Base(state),
            } = &mut tab.kind
            {
                if same_path(open, path) && !state.dirty {
                    // 保留视口位置、选中的视图、筛选和搜索条件，以及行详情的位置。
                    let active_table = state.document.active_table_id.clone();
                    let active_views: BTreeMap<_, _> = state
                        .document
                        .tables
                        .iter()
                        .map(|t| (t.id.clone(), t.active_view_id.clone()))
                        .collect();
                    state.document = document.clone();
                    state.document.active_table_id = active_table;
                    for table in &mut state.document.tables {
                        if let Some(view) = active_views.get(&table.id) {
                            table.active_view_id = view.clone();
                        }
                    }
                    state.saved_raw = content.into();
                    state.error.clear();
                }
            }
        }
        self.automation.last_error.clear();
        Ok(())
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b
        || a.canonicalize()
            .ok()
            .zip(b.canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
}
