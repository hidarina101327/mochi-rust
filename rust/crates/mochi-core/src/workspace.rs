//! 工作区初始化可能迁移目录并写入种子文件；只读校验不要调用 open。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::domain::{
    AiFolderPermission, AiPermissionsConfig, Library, LibrarySidebarConfig, LibraryType,
    MochiConfig, SidebarConfig, SidebarSection, WorkspaceInfo,
};
use crate::{json2, jstime, paths};

/// 根目录迁移时跳过的目录（TS 版清单 + schedule/收件箱 两个后置功能目录）。
const MIGRATION_SKIP_DIRS: &[&str] = &[
    paths::MOCHI_DIR_NAME,
    paths::KNOWLEDGE_BASE_TYPE_NAME,
    paths::AGENT_CONFIG_DIR,
    paths::LEGACY_AGENT_CONFIG_DIR,
    paths::SCHEDULE_DIR_NAME,
    crate::agenda::store::DIR_NAME,
    paths::INBOX_DIR_NAME,
    // 首页与 Electron 版共用 `日记/YYYY-MM-DD.md` 路径。
    "日记",
    "node_modules",
    "dist",
    "dist-electron",
    "release",
    "assets",
];

/// 用行数组 join 生成 LF，避免 Rust 原始字符串保留检出时的 CRLF。
const GITIGNORE_LINES: &[&str] = &[
    "# Mochi cache directories",
    ".mochi/",
    ".mochi/cache/",
    ".mochi/thumbnails/",
    ".mochi/link-cache/",
    ".mochi/temp/",
    "",
    "# System files",
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
    "",
    "# Editor files",
    ".vscode/",
    ".idea/",
    "*.swp",
    "*.swo",
    "*~",
];

/// 与 TS 版一致：内容以单个换行收尾。
pub fn default_gitignore() -> String {
    format!("{}\n", GITIGNORE_LINES.join("\n"))
}

pub struct WorkspaceService {
    root: PathBuf,
    name: String,
}

impl WorkspaceService {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root
            .as_ref()
            .canonicalize()
            .unwrap_or_else(|_| root.as_ref().to_path_buf());
        // canonicalize 在 Windows 会加 \\?\ 前缀，去掉以免污染落盘路径
        let root = strip_unc_prefix(root);
        let name = root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Self { root, name })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn open(&self) -> Result<WorkspaceInfo> {
        if !self.root.is_dir() {
            bail!("工作区不存在: {}", self.root.display());
        }

        self.ensure_mochi_structure()?;
        let migrated = self.migrate_legacy_structure()?;
        self.ensure_agent_config_rename()?;
        self.ensure_gitignore()?;
        let library_types = self.ensure_library_types()?;
        crate::bundled_guides::install(&self.root, false)?;
        let libraries = self.load_or_create_libraries()?;
        crate::git::GitService::new(&self.root).ensure_repository()?;

        Ok(WorkspaceInfo {
            root_path: self.root.clone(),
            name: self.name.clone(),
            library_types,
            libraries,
            migrated_this_open: migrated,
        })
    }

    // ---------- 结构初始化 ----------

    fn ensure_mochi_structure(&self) -> Result<()> {
        let mochi = paths::mochi_dir(&self.root);
        fs::create_dir_all(&mochi)
            .with_context(|| format!("创建 .mochi 失败: {}", mochi.display()))?;
        for sub in ["cache", "comments", "thumbnails", "link-cache"] {
            fs::create_dir_all(mochi.join(sub))?;
        }

        let config = paths::config_file(&self.root);
        if !config.exists() {
            json2::write(
                &config,
                &MochiConfig {
                    workspace_name: self.name.clone(),
                    created_at: jstime::now(),
                    last_modified: jstime::now(),
                    version: "1.0.0".into(),
                },
            )?;
        }

        let sidebar = paths::sidebar_file(&self.root);
        if !sidebar.exists() {
            json2::write(
                &sidebar,
                &SidebarConfig {
                    sections: vec![SidebarSection {
                        id: "workspace-root".into(),
                        title: "Workspace".into(),
                        path: self.root.to_string_lossy().into_owned(),
                        collapsed: false,
                        pinned: true,
                        order: None,
                    }],
                },
            )?;
        }

        let perms = paths::ai_permissions_file(&self.root);
        if !perms.exists() {
            // 种子形状对齐 `metadata.ts` 的 initializeConfigFiles：**不带** version。
            // 保存路径（ai-permissions.ts）才会写 version，两种形状都能原样往返。
            json2::write(
                &perms,
                &AiPermissionsConfig {
                    version: None,
                    permissions: vec![AiFolderPermission {
                        path: "/".into(),
                        level: "suggest".into(),
                        inherited: None,
                    }],
                },
            )?;
        }
        Ok(())
    }

    fn ensure_gitignore(&self) -> Result<()> {
        let path = self.root.join(".gitignore");
        if !path.exists() {
            fs::write(&path, default_gitignore())?;
        } else {
            let existing = fs::read_to_string(&path).unwrap_or_default();
            if !existing.contains(".mochi/") {
                let appended = format!("{existing}\n{}", default_gitignore());
                fs::write(&path, appended)?;
            }
        }
        Ok(())
    }

    // ---------- 迁移 ----------

    /// 旧结构（根目录散落知识库文件夹）→ 新结构（`知识库/<实例>/`）。幂等。
    fn migrate_legacy_structure(&self) -> Result<bool> {
        let kb_root = self.root.join(paths::KNOWLEDGE_BASE_TYPE_NAME);
        let mut moved_any = false;

        for dir in sorted_subdirs(&self.root)? {
            let name = match dir.file_name().map(|s| s.to_string_lossy().into_owned()) {
                Some(n) => n,
                None => continue,
            };
            if MIGRATION_SKIP_DIRS.contains(&name.as_str()) || name.starts_with('.') {
                continue;
            }

            fs::create_dir_all(&kb_root)?;
            let target = kb_root.join(&name);
            if !target.exists() {
                fs::rename(&dir, &target).with_context(|| {
                    format!("迁移失败: {} → {}", dir.display(), target.display())
                })?;
            } else {
                // 目标已存在（此前迁移中断）：逐项合并后清空源目录
                for entry in fs::read_dir(&dir)? {
                    let child = entry?.path();
                    let child_target = target.join(child.file_name().unwrap_or_default());
                    if !child_target.exists() {
                        fs::rename(&child, &child_target)?;
                    }
                }
                try_delete_if_empty(&dir);
            }
            moved_any = true;
        }

        Ok(moved_any)
    }

    /// `AI提示词` → `Agent配置` 目录改名 + `libraries.json` 路径改写。幂等。
    fn ensure_agent_config_rename(&self) -> Result<()> {
        let legacy = self.root.join(paths::LEGACY_AGENT_CONFIG_DIR);
        let current = self.root.join(paths::AGENT_CONFIG_DIR);

        if legacy.is_dir() && !current.exists() {
            fs::rename(&legacy, &current).with_context(|| {
                format!("改名失败: {} → {}", legacy.display(), current.display())
            })?;
        }
        if !current.is_dir() {
            return Ok(());
        }

        let file = paths::libraries_file(&self.root);
        let Some(mut libraries) = json2::read_from_file::<Vec<Library>>(&file)? else {
            return Ok(());
        };

        let mut changed = false;
        let legacy_segment = format!("/{}/", paths::LEGACY_AGENT_CONFIG_DIR);
        for lib in &mut libraries {
            if lib.kind != "ai-prompts" {
                continue;
            }
            if paths::to_forward_slashes(&lib.path).contains(&legacy_segment) {
                lib.path = paths::to_forward_slashes(
                    &lib.path
                        .replace(paths::LEGACY_AGENT_CONFIG_DIR, paths::AGENT_CONFIG_DIR),
                );
                changed = true;
            }
        }
        if changed {
            json2::write(&file, &libraries)?;
        }
        Ok(())
    }

    // ---------- 库配置 ----------

    fn ensure_library_types(&self) -> Result<Vec<LibraryType>> {
        let file = paths::library_types_file(&self.root);
        let mut types = json2::read_from_file::<Vec<LibraryType>>(&file)?.unwrap_or_default();
        let mut changed = false;

        for builtin in built_in_library_types() {
            match types
                .iter_mut()
                .find(|t| t.id == builtin.id && t.is_built_in)
            {
                None => {
                    types.push(builtin);
                    changed = true;
                }
                Some(existing) => {
                    // 旧版行为：内建类型每次启动刷新 name/icon/description
                    if existing.name != builtin.name
                        || existing.icon != builtin.icon
                        || existing.description != builtin.description
                    {
                        existing.name = builtin.name;
                        existing.icon = builtin.icon;
                        existing.description = builtin.description;
                        changed = true;
                    }
                }
            }
        }

        if changed || !file.exists() {
            json2::write(&file, &types)?;
        }
        Ok(types)
    }

    fn load_or_create_libraries(&self) -> Result<Vec<Library>> {
        let file = paths::libraries_file(&self.root);
        let _lock = crate::settings::file::Lock::acquire(&file)?;
        let mut libraries = json2::read_from_file::<Vec<Library>>(&file)?.unwrap_or_default();
        let mut added = false;

        // 知识库/ 下尚无登记实例的目录补建条目（迁移产物 + 用户手动建的文件夹）
        let kb_root = self.root.join(paths::KNOWLEDGE_BASE_TYPE_NAME);
        if kb_root.is_dir() {
            let known: HashSet<String> = libraries
                .iter()
                .map(|l| paths::to_forward_slashes(&l.path))
                .collect();

            for dir in sorted_subdirs(&kb_root)? {
                let fwd = paths::to_forward_slashes(&dir.to_string_lossy());
                if known.contains(&fwd) {
                    continue;
                }
                libraries.push(Library {
                    id: paths::new_library_id("knowledge-base"),
                    name: dir
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    kind: "knowledge-base".into(),
                    path: fwd,
                    icon: None,
                    created_at: jstime::now(),
                    updated_at: jstime::now(),
                    sidebar_config: Some(LibrarySidebarConfig {
                        show_file_tree: true,
                        show_outline: true,
                        show_tags: None,
                    }),
                });
                added = true;
            }
        }

        if !file.exists() || added {
            json2::write(&file, &libraries)?;
        }
        Ok(libraries)
    }

    /// 创建库实例：建目录 + 登记 `libraries.json`。
    pub fn create_library(
        &self,
        type_id: &str,
        type_name: &str,
        name: &str,
        icon: Option<String>,
    ) -> Result<Library> {
        let dir = self.root.join(type_name).join(name);
        fs::create_dir_all(&dir)?;

        let file = paths::libraries_file(&self.root);
        let _lock = crate::settings::file::Lock::acquire(&file)?;
        let mut libraries = json2::read_from_file::<Vec<Library>>(&file)?.unwrap_or_default();
        let normalized = paths::to_forward_slashes(&dir.to_string_lossy());
        if let Some(existing) = libraries.iter().find(|l| l.path == normalized) {
            return Ok(existing.clone());
        }
        let lib = Library {
            id: paths::new_library_id(type_id),
            name: name.to_owned(),
            kind: type_id.to_owned(),
            path: paths::to_forward_slashes(&dir.to_string_lossy()),
            icon,
            created_at: jstime::now(),
            updated_at: jstime::now(),
            sidebar_config: Some(LibrarySidebarConfig {
                show_file_tree: true,
                show_outline: true,
                show_tags: None,
            }),
        };
        libraries.push(lib.clone());
        json2::write(&file, &libraries)?;
        Ok(lib)
    }
}

/// 内建库类型（与 `src/services/library.ts` 的 `getBuiltInLibraryTypes` 一致）。
pub fn built_in_library_types() -> Vec<LibraryType> {
    let sidebar = Some(LibrarySidebarConfig {
        show_file_tree: true,
        show_outline: true,
        show_tags: None,
    });
    vec![
        LibraryType {
            id: "knowledge-base".into(),
            name: paths::KNOWLEDGE_BASE_TYPE_NAME.into(),
            icon: "BookOpen".into(),
            description: Some("用于存储和组织知识笔记".into()),
            default_sidebar_config: sidebar.clone(),
            created_at: jstime::now(),
            is_built_in: true,
        },
        LibraryType {
            id: "ai-prompts".into(),
            name: paths::AGENT_CONFIG_DIR.into(),
            icon: "Sparkles".into(),
            description: Some("管理 Agents、Skills、Tools、MCPs 与快捷 AI 功能块配置".into()),
            default_sidebar_config: sidebar,
            created_at: jstime::now(),
            is_built_in: true,
        },
    ]
}

/// 子目录列表，按 Ordinal 排序——C# 版对 `知识库/` 下的目录做了同样的排序，
/// 决定了自动登记的库在 `libraries.json` 里的顺序。
fn sorted_subdirs(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

fn try_delete_if_empty(dir: &Path) {
    if let Ok(mut it) = fs::read_dir(dir) {
        if it.next().is_none() {
            let _ = fs::remove_dir(dir);
        }
    }
}

/// Windows 的 `canonicalize` 会返回 `\\?\D:\...`，去掉前缀免得写进配置文件。
fn strip_unc_prefix(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct TempWs(PathBuf);

    impl TempWs {
        fn new(tag: &str) -> Self {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("mochi-ws-{}-{tag}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn svc(&self) -> WorkspaceService {
            WorkspaceService::new(&self.0).unwrap()
        }
    }

    impl Drop for TempWs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn open_seeds_mochi_structure() {
        let ws = TempWs::new("seed");
        let info = ws.svc().open().unwrap();

        for sub in ["cache", "comments", "thumbnails", "link-cache"] {
            assert!(
                paths::mochi_dir(&info.root_path).join(sub).is_dir(),
                "缺 {sub}"
            );
        }
        assert!(paths::config_file(&info.root_path).is_file());
        assert!(paths::sidebar_file(&info.root_path).is_file());
        assert!(paths::ai_permissions_file(&info.root_path).is_file());
        assert!(!info.migrated_this_open);
    }

    #[test]
    fn workspace_includes_release_guides() {
        let ws = TempWs::new("guide");
        let info = ws.svc().open().unwrap();
        assert!(info
            .libraries
            .iter()
            .any(|l| l.path.ends_with("知识库/墨池")));
        for (name, bytes) in crate::bundled_guides::FILES {
            assert_eq!(
                fs::read(info.root_path.join("知识库/墨池").join(name)).unwrap(),
                *bytes
            );
        }
    }

    #[test]
    fn seeded_configs_match_expected_bytes() {
        let ws = TempWs::new("bytes");
        let svc = ws.svc();
        svc.open().unwrap();

        let perms = fs::read_to_string(paths::ai_permissions_file(svc.root())).unwrap();
        assert_eq!(
            perms,
            "{\n  \"permissions\": [\n    {\n      \"path\": \"/\",\n      \"level\": \"suggest\"\n    }\n  ]\n}"
        );
    }

    #[test]
    fn gitignore_is_written_with_lf_only() {
        let ws = TempWs::new("gitignore");
        let svc = ws.svc();
        svc.open().unwrap();

        let bytes = fs::read(svc.root().join(".gitignore")).unwrap();
        assert!(
            !bytes.contains(&b'\r'),
            "写出了 CRLF —— 会与 Electron 版互相改写"
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("# Mochi cache directories\n.mochi/\n"));
        assert!(text.ends_with("*~\n"));
    }

    #[test]
    fn gitignore_is_appended_when_mochi_pattern_missing() {
        let ws = TempWs::new("gitignore-append");
        let svc = ws.svc();
        fs::write(svc.root().join(".gitignore"), "node_modules/\n").unwrap();
        svc.open().unwrap();

        let text = fs::read_to_string(svc.root().join(".gitignore")).unwrap();
        assert!(text.starts_with("node_modules/\n"), "覆盖了用户已有内容");
        assert!(text.contains(".mochi/"));
    }

    #[test]
    fn gitignore_is_left_alone_when_already_covered() {
        let ws = TempWs::new("gitignore-noop");
        let svc = ws.svc();
        let original = "# mine\n.mochi/\n";
        fs::write(svc.root().join(".gitignore"), original).unwrap();
        svc.open().unwrap();
        assert_eq!(
            fs::read_to_string(svc.root().join(".gitignore")).unwrap(),
            original
        );
    }

    #[test]
    fn legacy_dirs_move_under_knowledge_base() {
        let ws = TempWs::new("migrate");
        let svc = ws.svc();
        fs::create_dir_all(svc.root().join("计算机通识")).unwrap();
        fs::write(svc.root().join("计算机通识").join("a.md"), "hi").unwrap();
        fs::create_dir_all(svc.root().join("node_modules")).unwrap();

        let info = svc.open().unwrap();

        assert!(info.migrated_this_open);
        let moved = svc
            .root()
            .join(paths::KNOWLEDGE_BASE_TYPE_NAME)
            .join("计算机通识")
            .join("a.md");
        assert!(moved.is_file(), "文件没跟着目录一起搬");
        assert!(!svc.root().join("计算机通识").exists());
        assert!(svc.root().join("node_modules").is_dir(), "跳过清单没生效");
    }

    #[test]
    fn migration_is_idempotent() {
        let ws = TempWs::new("migrate-idem");
        let svc = ws.svc();
        fs::create_dir_all(svc.root().join("面试经历")).unwrap();

        assert!(svc.open().unwrap().migrated_this_open);
        // 第二次打开：已无可迁移目录
        assert!(!svc.open().unwrap().migrated_this_open);
    }

    #[test]
    fn interrupted_migration_merges_into_existing_target() {
        let ws = TempWs::new("migrate-merge");
        let svc = ws.svc();
        // 模拟上次迁移中断：源与目标同时存在，各有一个文件
        fs::create_dir_all(svc.root().join("旧库")).unwrap();
        fs::write(svc.root().join("旧库").join("new.md"), "new").unwrap();
        let target = svc
            .root()
            .join(paths::KNOWLEDGE_BASE_TYPE_NAME)
            .join("旧库");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("old.md"), "old").unwrap();

        svc.open().unwrap();

        assert!(target.join("old.md").is_file(), "已有文件被覆盖了");
        assert!(target.join("new.md").is_file(), "源文件没合并过来");
        assert!(!svc.root().join("旧库").exists(), "空的源目录没清掉");
    }

    #[test]
    fn agent_config_dir_is_renamed_and_paths_rewritten() {
        let ws = TempWs::new("agent-rename");
        let svc = ws.svc();
        fs::create_dir_all(svc.root().join(paths::LEGACY_AGENT_CONFIG_DIR)).unwrap();
        json2::write(
            paths::libraries_file(svc.root()),
            &vec![Library {
                id: "ai-1".into(),
                name: "提示词".into(),
                kind: "ai-prompts".into(),
                path: format!("D:/ws/{}/x", paths::LEGACY_AGENT_CONFIG_DIR),
                ..Default::default()
            }],
        )
        .unwrap();

        svc.open().unwrap();

        assert!(svc.root().join(paths::AGENT_CONFIG_DIR).is_dir());
        assert!(!svc.root().join(paths::LEGACY_AGENT_CONFIG_DIR).exists());
        let libs: Vec<Library> = json2::read_from_file(paths::libraries_file(svc.root()))
            .unwrap()
            .unwrap();
        assert_eq!(libs[0].path, format!("D:/ws/{}/x", paths::AGENT_CONFIG_DIR));
    }

    #[test]
    fn built_in_types_are_seeded_and_refreshed() {
        let ws = TempWs::new("types");
        let svc = ws.svc();
        let info = svc.open().unwrap();
        assert_eq!(info.library_types.len(), 2);
        assert!(info.library_types.iter().all(|t| t.is_built_in));

        // 篡改内建类型的展示字段，再打开应被刷回
        let file = paths::library_types_file(svc.root());
        let mut types: Vec<LibraryType> = json2::read_from_file(&file).unwrap().unwrap();
        types[0].name = "被改坏的名字".into();
        json2::write(&file, &types).unwrap();

        let refreshed = svc.open().unwrap();
        assert_eq!(
            refreshed.library_types[0].name,
            paths::KNOWLEDGE_BASE_TYPE_NAME
        );
    }

    #[test]
    fn unregistered_knowledge_base_dirs_are_auto_registered() {
        let ws = TempWs::new("auto-register");
        let svc = ws.svc();
        let kb = svc.root().join(paths::KNOWLEDGE_BASE_TYPE_NAME);
        fs::create_dir_all(kb.join("bbb")).unwrap();
        fs::create_dir_all(kb.join("aaa")).unwrap();

        let info = svc.open().unwrap();

        let names: Vec<_> = info.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["aaa", "bbb", "墨池"], "登记顺序应按名称排序");
        assert!(info.libraries.iter().all(|l| l.kind == "knowledge-base"));
        assert!(
            info.libraries.iter().all(|l| !l.path.contains('\\')),
            "路径必须是正斜杠"
        );

        // 幂等：再开一次不应重复登记
        assert_eq!(svc.open().unwrap().libraries.len(), 3);
    }

    #[test]
    fn create_library_makes_dir_and_registers() {
        let ws = TempWs::new("create-lib");
        let svc = ws.svc();
        svc.open().unwrap();

        let lib = svc
            .create_library(
                "knowledge-base",
                paths::KNOWLEDGE_BASE_TYPE_NAME,
                "新库",
                None,
            )
            .unwrap();

        assert!(svc
            .root()
            .join(paths::KNOWLEDGE_BASE_TYPE_NAME)
            .join("新库")
            .is_dir());
        assert!(lib.id.starts_with("knowledge-base-"));
        let libs: Vec<Library> = json2::read_from_file(paths::libraries_file(svc.root()))
            .unwrap()
            .unwrap();
        assert!(libs.iter().any(|l| l.name == "新库"));
    }

    #[test]
    fn open_initializes_git_repository() {
        let ws = TempWs::new("git");
        let svc = ws.svc();
        svc.open().unwrap();

        assert!(
            paths::git_dir(svc.root()).is_dir(),
            "open() 没建 .mochi/git"
        );
        assert!(crate::git::GitService::new(svc.root()).is_valid());
    }

    #[test]
    fn open_missing_dir_fails() {
        let missing = std::env::temp_dir().join("mochi-no-such-ws-7c1a");
        let svc = WorkspaceService::new(&missing).unwrap();
        assert!(svc.open().is_err());
    }

    #[test]
    fn daily_journal_keeps_the_electron_home_path_when_reopening() {
        let ws = TempWs::new("journal-path");
        let svc = ws.svc();
        let journal = svc.root().join("日记/2026-09-13.md");
        fs::create_dir_all(journal.parent().unwrap()).unwrap();
        fs::write(&journal, "# 今天\n\n已有记录").unwrap();
        svc.open().unwrap();
        svc.open().unwrap();
        assert_eq!(fs::read_to_string(journal).unwrap(), "# 今天\n\n已有记录");
        assert!(!svc.root().join("知识库/日记").exists());
    }
}
