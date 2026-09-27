//! 内置 Agent 的修订按内容寻址，与 app 版本无关。
//! 记住最近一次接受的上游文本：本地自己改过不算更新。
use super::AgentConfigService;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

static GATE: Mutex<()> = Mutex::new(());
const STATE: &str = ".bundled-agents/state.json";

#[derive(Default, Serialize, Deserialize, PartialEq)]
struct Record {
    baseline: Option<String>,
    skipped: Option<String>,
    notified: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    const GENERAL: &str = "Agents/通用助手.md";
    struct Workspace(PathBuf);
    impl Workspace {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "mochi-agent-updates-{}",
                crate::paths::random_base36(16)
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn service(&self) -> AgentConfigService {
            AgentConfigService::new(&self.0)
        }
        fn seed(&self) -> AgentConfigService {
            let svc = self.service();
            svc.ensure_seeds().unwrap();
            svc
        }
        fn old_version(&self, baseline: &str, local: &str) -> AgentConfigService {
            let svc = self.seed();
            fs::write(svc.root().join(GENERAL), local).unwrap();
            let mut manifest = svc.read_manifest().unwrap();
            manifest.agents.get_mut(GENERAL).unwrap().baseline = Some(baseline.into());
            svc.save_manifest(&manifest).unwrap();
            svc
        }
    }
    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn initial_install_and_local_edits_do_not_offer_updates() {
        let ws = Workspace::new();
        let svc = ws.seed();
        assert!(svc.agent_updates(false).unwrap().is_empty());
        let path = svc.root().join(GENERAL);
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("{text}\n个人偏好：简短回答。\n")).unwrap();
        assert_eq!(svc.ensure_seeds().unwrap(), 0);
        assert!(svc.agent_updates(false).unwrap().is_empty());
        assert!(svc.take_agent_update_notices().unwrap().is_empty());
    }

    #[test]
    fn legacy_unknown_baseline_diff_and_backup_are_preserved() {
        let ws = Workspace::new();
        let svc = ws.service();
        fs::create_dir_all(svc.root().join("Agents")).unwrap();
        let local = "---\nname: 通用助手\n---\n我的定制提示词\n";
        fs::write(svc.root().join(GENERAL), local).unwrap();
        svc.ensure_seeds().unwrap();
        let updates = svc.agent_updates(false).unwrap();
        assert_eq!(updates.len(), 1);
        let update = &updates[0];
        assert!(update.baseline.is_none());
        assert!(update.diff().unwrap().contains("-我的定制提示词"));
        assert_eq!(fs::read_to_string(&update.source_path).unwrap(), local);
        let backup = svc.apply_agent_update(update).unwrap();
        assert_eq!(fs::read_to_string(backup).unwrap(), local);
        assert_eq!(
            fs::read_to_string(&update.source_path).unwrap(),
            update.incoming
        );
        assert!(svc.agent_updates(false).unwrap().is_empty());
    }

    #[test]
    fn upstream_and_replacement_diffs_distinguish_personal_edits() {
        let ws = Workspace::new();
        let svc = ws.old_version("旧官方提示词\n", "旧官方提示词\n个人偏好\n");
        let updates = svc.agent_updates(false).unwrap();
        assert_eq!(updates.len(), 1);
        assert!(updates[0].locally_modified());
        assert!(updates[0].diff().unwrap().contains("-个人偏好"));
        assert!(!updates[0]
            .upstream_diff()
            .unwrap()
            .unwrap()
            .contains("个人偏好"));
        assert_eq!(svc.take_agent_update_notices().unwrap(), vec![GENERAL]);
        assert!(svc.take_agent_update_notices().unwrap().is_empty());
        assert!(ws.service().take_agent_update_notices().unwrap().is_empty());
    }

    #[test]
    fn skip_is_persistent_and_only_applies_to_that_revision() {
        let ws = Workspace::new();
        let svc = ws.old_version("旧官方\n", "旧官方\n");
        let update = svc.agent_updates(false).unwrap().remove(0);
        svc.skip_agent_update(&update).unwrap();
        assert!(ws.service().agent_updates(false).unwrap().is_empty());
        assert!(svc.take_agent_update_notices().unwrap().is_empty());
        assert!(svc.agent_updates(true).unwrap()[0].skipped);
        assert_eq!(
            fs::read_to_string(&update.source_path).unwrap(),
            update.current
        );
        // 升级之后，记录在案的 skipped 哈希属于旧的内置包。
        let mut manifest = svc.read_manifest().unwrap();
        manifest.agents.get_mut(GENERAL).unwrap().skipped = Some(revision("previous bundle"));
        manifest.agents.get_mut(GENERAL).unwrap().notified = Some(revision("previous bundle"));
        svc.save_manifest(&manifest).unwrap();
        assert_eq!(svc.agent_updates(false).unwrap().len(), 1);
        assert_eq!(svc.take_agent_update_notices().unwrap(), vec![GENERAL]);
    }

    #[test]
    fn stale_preview_or_forged_definition_never_overwrites_local_file() {
        let ws = Workspace::new();
        let svc = ws.old_version("旧官方\n", "旧官方\n");
        let mut update = svc.agent_updates(false).unwrap().remove(0);
        fs::write(&update.source_path, "预览后的新编辑").unwrap();
        assert!(svc
            .apply_agent_update(&update)
            .unwrap_err()
            .to_string()
            .contains("预览后"));
        assert_eq!(
            fs::read_to_string(&update.source_path).unwrap(),
            "预览后的新编辑"
        );
        update.current = "预览后的新编辑".into();
        update.incoming = "伪造新版".into();
        assert!(svc.apply_agent_update(&update).is_err());
        update.relative_path = "../../outside.md".into();
        assert!(svc.apply_agent_update(&update).is_err());
    }

    #[test]
    fn corrupt_manifest_is_reported_without_replacing_it_or_agent() {
        let ws = Workspace::new();
        let svc = ws.old_version("旧官方\n", "旧官方\n");
        let update = svc.agent_updates(false).unwrap().remove(0);
        fs::write(svc.root().join(STATE), "invalid json").unwrap();
        assert!(svc.apply_agent_update(&update).is_err());
        assert!(svc.ensure_seeds().is_err());
        assert_eq!(
            fs::read_to_string(&update.source_path).unwrap(),
            update.current
        );
        assert_eq!(
            fs::read_to_string(svc.root().join(STATE)).unwrap(),
            "invalid json"
        );
    }

    #[test]
    fn versions_ignore_windows_line_endings_and_bom_but_detect_prompt_changes() {
        assert_eq!(revision("a\nb\n"), revision("\u{feff}a\r\nb\r\n"));
        assert_ne!(revision("a\nb\n"), revision("a\nc\n"));
    }

    #[test]
    fn custom_agents_are_not_considered_bundled_updates() {
        let ws = Workspace::new();
        let svc = ws.seed();
        fs::write(svc.root().join("Agents/自己的助手.md"), "自定义").unwrap();
        assert!(svc.agent_updates(true).unwrap().is_empty());
    }
}

#[derive(Serialize, Deserialize, PartialEq)]
struct Manifest {
    schema_version: u32,
    agents: BTreeMap<String, Record>,
}
impl Default for Manifest {
    fn default() -> Self {
        Self {
            schema_version: 1,
            agents: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AgentUpdate {
    pub relative_path: String,
    pub source_path: PathBuf,
    pub current: String,
    pub incoming: String,
    pub baseline: Option<String>,
    pub revision: String,
    pub skipped: bool,
}
impl AgentUpdate {
    pub fn locally_modified(&self) -> bool {
        self.baseline
            .as_ref()
            .is_some_and(|base| revision(base) != revision(&self.current))
    }
    pub fn previous_revision(&self) -> Option<String> {
        self.baseline.as_deref().map(revision)
    }
    /// 精确的替换预览，包括将被移除的用户自己的修改。
    pub fn diff(&self) -> Result<String> {
        diff(&self.current, &self.incoming)
    }
    pub fn upstream_diff(&self) -> Result<Option<String>> {
        self.baseline
            .as_deref()
            .map(|base| diff(base, &self.incoming))
            .transpose()
    }
}

fn normalized(text: &str) -> String {
    text.trim_start_matches('\u{feff}').replace("\r\n", "\n")
}
pub fn revision(text: &str) -> String {
    Sha256::digest(normalized(text).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn diff(before: &str, after: &str) -> Result<String> {
    let before = normalized(before);
    let after = normalized(after);
    let mut options = git2::DiffOptions::new();
    options.context_lines(3).force_text(true);
    let mut patch = git2::Patch::from_buffers(
        before.as_bytes(),
        Some(Path::new("current.md")),
        after.as_bytes(),
        Some(Path::new("bundled.md")),
        Some(&mut options),
    )?;
    Ok(String::from_utf8(patch.to_buf()?.to_vec())?)
}

impl AgentConfigService {
    fn managed_path(&self, relative: &str) -> Result<PathBuf> {
        let relative = Path::new(relative);
        ensure!(
            relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "无效的 Agent 更新路径"
        );
        let path = self.root().join(relative);
        let workspace = self.workspace_path.canonicalize()?;
        let mut existing = path.as_path();
        while !existing.exists() {
            existing = existing.parent().context("无效的 Agent 更新目录")?;
        }
        ensure!(
            existing.canonicalize()?.starts_with(&workspace),
            "Agent 更新路径位于工作区之外"
        );
        Ok(path)
    }

    fn read_manifest(&self) -> Result<Manifest> {
        let path = self.managed_path(STATE)?;
        match fs::read_to_string(path) {
            Ok(raw) => {
                let manifest: Manifest =
                    serde_json::from_str(&raw).context("Agent 版本记录损坏，原文件已保留")?;
                ensure!(
                    manifest.schema_version == 1,
                    "无法读取此版本的 Agent 版本记录"
                );
                Ok(manifest)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
            Err(e) => Err(e.into()),
        }
    }
    fn save_manifest(&self, manifest: &Manifest) -> Result<()> {
        crate::files::FileService::new().write_file_safe(
            self.managed_path(STATE)?,
            &serde_json::to_string_pretty(manifest)?,
        )
    }
    fn bundled_agents() -> impl Iterator<Item = (&'static str, &'static str)> {
        Self::bundled_seeds().iter().filter_map(|(path, body)| {
            (path.starts_with("Agents/") && path.ends_with(".md"))
                .then(|| (path.as_str(), body.as_str().expect("agent seed text")))
        })
    }

    /// 迁移匹配的旧文件，但内容有差异的文件绝不猜它的基线。
    pub(super) fn track_agent_versions(&self) -> Result<()> {
        let _guard = GATE.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::settings::file::Lock::acquire(&self.managed_path(STATE)?)?;
        let mut manifest = self.read_manifest()?;
        let before = serde_json::to_string(&manifest)?;
        for (path, incoming) in Self::bundled_agents() {
            let current = fs::read_to_string(self.managed_path(path)?)?;
            let record = manifest.agents.entry(path.into()).or_default();
            if revision(&current) == revision(incoming) {
                record.baseline = Some(incoming.into());
                record.skipped = None;
            }
        }
        if before != serde_json::to_string(&manifest)? {
            self.save_manifest(&manifest)?;
        }
        Ok(())
    }

    pub fn agent_updates(&self, include_skipped: bool) -> Result<Vec<AgentUpdate>> {
        let manifest = self.read_manifest()?;
        self.updates_with(&manifest, include_skipped)
    }
    fn updates_with(&self, manifest: &Manifest, include_skipped: bool) -> Result<Vec<AgentUpdate>> {
        let mut updates = Vec::new();
        for (relative, incoming) in Self::bundled_agents() {
            let source_path = self.managed_path(relative)?;
            let current = match fs::read_to_string(&source_path) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e).with_context(|| format!("无法检查 {relative}")),
            };
            let record = manifest.agents.get(relative);
            let baseline = record.and_then(|r| r.baseline.clone());
            let revision = revision(incoming);
            let skipped = record.and_then(|r| r.skipped.as_ref()) == Some(&revision);
            if self::revision(&current) == revision
                || baseline
                    .as_ref()
                    .is_some_and(|base| self::revision(base) == revision)
                || (skipped && !include_skipped)
            {
                continue;
            }
            updates.push(AgentUpdate {
                relative_path: relative.into(),
                source_path,
                current,
                incoming: incoming.into(),
                baseline,
                revision,
                skipped,
            });
        }
        Ok(updates)
    }

    /// 启动通知只返回新发现的修订。
    pub fn take_agent_update_notices(&self) -> Result<Vec<String>> {
        let _guard = GATE.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::settings::file::Lock::acquire(&self.managed_path(STATE)?)?;
        let mut manifest = self.read_manifest()?;
        let updates = self.updates_with(&manifest, false)?;
        let mut names = Vec::new();
        for update in updates {
            let record = manifest
                .agents
                .entry(update.relative_path.clone())
                .or_default();
            if record.notified.as_ref() != Some(&update.revision) {
                names.push(update.relative_path);
                record.notified = Some(update.revision);
            }
        }
        if !names.is_empty() {
            self.save_manifest(&manifest)?;
        }
        Ok(names)
    }

    fn validate_review(&self, update: &AgentUpdate) -> Result<PathBuf> {
        let incoming = Self::bundled_agents()
            .find(|(path, _)| *path == update.relative_path)
            .context("该定义不属于安装包")?
            .1;
        ensure!(
            revision(incoming) == update.revision && incoming == update.incoming,
            "安装包定义已变化，请重新查看差异"
        );
        let target = self.managed_path(&update.relative_path)?;
        ensure!(
            fs::read_to_string(&target)? == update.current,
            "本地定义在预览后已变化，请重新查看差异"
        );
        Ok(target)
    }

    pub fn skip_agent_update(&self, update: &AgentUpdate) -> Result<()> {
        let _guard = GATE.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::settings::file::Lock::acquire(&self.managed_path(STATE)?)?;
        self.validate_review(update)?;
        let mut manifest = self.read_manifest()?;
        manifest
            .agents
            .entry(update.relative_path.clone())
            .or_default()
            .skipped = Some(update.revision.clone());
        self.save_manifest(&manifest)
    }

    /// 用户显式接受的替换。原子写入之前先做永久备份。
    pub fn apply_agent_update(&self, update: &AgentUpdate) -> Result<PathBuf> {
        let _guard = GATE.lock().unwrap_or_else(|e| e.into_inner());
        let _lock = crate::settings::file::Lock::acquire(&self.managed_path(STATE)?)?;
        let target = self.validate_review(update)?;
        let mut manifest = self.read_manifest()?;
        let backup = self.managed_path(&format!(
            ".bundled-agents/backups/{}-{}.md",
            crate::jstime::now_millis(),
            crate::paths::random_base36(12)
        ))?;
        let files = crate::files::FileService::new();
        files
            .write_file_safe(&backup, &update.current)
            .context("更新前备份失败，未修改 Agent")?;
        self.validate_review(update)?;
        files.write_file_safe(target, &update.incoming)?;
        let record = manifest
            .agents
            .entry(update.relative_path.clone())
            .or_default();
        record.baseline = Some(update.incoming.clone());
        record.skipped = None;
        self.save_manifest(&manifest).with_context(|| {
            format!(
                "Agent 已更新，但版本记录保存失败；备份：{}",
                backup.display()
            )
        })?;
        Ok(backup)
    }
}
