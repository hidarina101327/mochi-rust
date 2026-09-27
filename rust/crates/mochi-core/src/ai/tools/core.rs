//! 文件工具语义与 shared/agent-tools/core-tools.ts 对齐；suggest 权限只生成提案。

use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use super::host::{
    agent_path_basename, join_agent_path, resolve_workspace_path, validate_name, FileChange,
    PendingFileOperation, ResolveOptions, ToolHost,
};
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;
use crate::jstime;
use crate::search::{SearchOptions, SearchService};

/// 单次 `file_read` 的上限。超过就让模型改用 `content_search` 定位，
/// 而不是把一本书塞进上下文。
const MAX_READ_BYTES: u64 = 1024 * 1024;

const TOOL_NAMES: &[&str] = &[
    "folder_list",
    "folder_create",
    "file_read",
    "file_write",
    "file_delete",
    "folder_delete",
    "path_rename",
    "content_search",
];

pub struct CoreToolExecutor {
    host: Arc<dyn ToolHost>,
    /// `content_search` 需要；缺省时该工具报错而不是静默返回空结果。
    search: Option<Arc<SearchService>>,
}

impl CoreToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host, search: None }
    }

    pub fn with_search(mut self, search: Arc<SearchService>) -> Self {
        self.search = Some(search);
        self
    }

    fn resolve(&self, input: Option<&str>) -> Result<String, String> {
        resolve_workspace_path(self.host.as_ref(), input, ResolveOptions::default())
    }

    /// 解析失败时不做权限声明——`call` 会用同样的解析再失败一次并给出原因。
    /// 这里返回 `None` 只是"没有可检查的路径"，不代表放行。
    fn resolve_quiet(&self, input: Option<&str>) -> Option<String> {
        self.resolve(input).ok()
    }
}

impl ToolExecutor for CoreToolExecutor {
    fn handles(&self, name: &str) -> bool {
        TOOL_NAMES.contains(&name)
    }

    fn required_actions(&self, name: &str, args: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        let path = |key: &str| self.resolve_quiet(args.str_opt(key));
        match name {
            "folder_list" => vec![(AiToolAction::ReadFile, path("path"))],
            "file_read" => vec![(AiToolAction::ReadFile, path("path"))],
            "content_search" => vec![(AiToolAction::ReadFile, path("path"))],
            "folder_create" => {
                // 检查的是**将要创建的目标**，不是父目录——父目录可写不代表
                // 子路径可写（子路径可能命中更具体的只读规则）
                let target = match (
                    self.resolve_quiet(args.str_opt("parentPath")),
                    args.str_opt("name"),
                ) {
                    (Some(parent), Some(name)) => Some(join_agent_path(&parent, name)),
                    (parent, _) => parent,
                };
                vec![(AiToolAction::WriteFile, target)]
            }
            "file_write" => vec![(AiToolAction::WriteFile, path("path"))],
            "file_delete" | "folder_delete" => vec![(AiToolAction::DeleteFile, path("path"))],
            // 改名同时影响两端，两个路径都要过门
            "path_rename" => vec![
                (AiToolAction::WriteFile, path("oldPath")),
                (AiToolAction::WriteFile, path("newPath")),
            ],
            _ => Vec::new(),
        }
    }

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        match name {
            "folder_list" => self.folder_list(args),
            "folder_create" => self.folder_create(args),
            "file_read" => self.file_read(args),
            "file_write" => self.file_write(args),
            "file_delete" => self.file_delete(args),
            "folder_delete" => self.folder_delete(args),
            "path_rename" => self.path_rename(args),
            "content_search" => self.content_search(args),
            other => Err(format!("未知的核心工具: {other}")),
        }
    }
}

impl CoreToolExecutor {
    fn folder_list(&self, args: &ToolArgs) -> ToolOutcome {
        let target = self.resolve(args.str_opt("path"))?;
        let entries = std::fs::read_dir(&target).map_err(|e| format!("读取目录失败: {e}"))?;

        let mut children = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = crate::paths::to_forward_slashes(&entry.path().to_string_lossy());
            if !self.host.is_path_visible(&path) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let kind = if meta.is_dir() {
                "directory"
            } else if name.to_ascii_lowercase().ends_with(".link.json") {
                "link"
            } else {
                "file"
            };
            children.push(json!({
                "name": name,
                "path": path,
                "type": kind,
                "size": meta.len(),
                "mtime": meta.modified().ok().map(|t| jstime::from(t.into())),
            }));
        }
        // read_dir 的顺序由文件系统决定，排一下让模型看到稳定的结果
        children.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });

        Ok(json!({ "path": target, "children": children }))
    }

    fn folder_create(&self, args: &ToolArgs) -> ToolOutcome {
        let parent = self.resolve(args.str_opt("parentPath"))?;
        let name = args.str_required("name")?.trim().to_owned();
        validate_name(&name)?;
        let target = join_agent_path(&parent, &name);

        if self.host.should_propose(&target) {
            return self.host.submit_proposal(PendingFileOperation {
                kind: "create-folder".into(),
                title: name.clone(),
                summary: format!("创建文件夹 {name}"),
                content: None,
                previous_content: None,
                path: target,
                new_path: None,
                entries: Vec::new(),
                reason: "permission-suggest".into(),
            });
        }

        std::fs::create_dir_all(&target).map_err(|e| format!("创建文件夹失败: {e}"))?;
        self.host.notify_change(FileChange::FolderCreate {
            path: target.clone(),
        });
        Ok(json!({ "path": target, "name": name }))
    }

    fn file_read(&self, args: &ToolArgs) -> ToolOutcome {
        let target = self.resolve(args.str_opt("path"))?;
        let meta = std::fs::metadata(&target).map_err(|e| format!("读取失败: {e}"))?;
        if meta.is_dir() {
            return Err("目标是文件夹，不是文件".into());
        }
        if meta.len() > MAX_READ_BYTES {
            return Err("文件超过 1MB，拒绝直接读取".into());
        }
        let content = self
            .host
            .read_document_text(&target)
            .map_err(|e| format!("读取失败: {e}"))?;
        if content.len() as u64 > MAX_READ_BYTES {
            return Err("文件超过 1MB，拒绝直接读取".into());
        }
        Ok(json!({ "path": target, "content": content }))
    }

    fn file_write(&self, args: &ToolArgs) -> ToolOutcome {
        let target = self.resolve(args.str_opt("path"))?;
        let mode = args.str_opt("mode").unwrap_or("create");
        let raw = args.get("content").and_then(Value::as_str).unwrap_or("");
        let is_base = target.to_ascii_lowercase().ends_with(".mcb");
        let content = if is_base {
            crate::base::parse_base_document(raw)
                .map_err(|error| format!("多维表格校验失败：{error}"))?;
            raw.to_owned()
        } else {
            self.host.transform_content_before_write(&target, raw)
        };
        let exists = Path::new(&target).exists();

        if exists && mode == "create" {
            return Err("文件已存在，mode=create 不允许覆盖".into());
        }

        // suggest 级路径不直接落盘，转成待批准提案
        if self.host.should_propose(&target) {
            let previous = if exists {
                Some(self.host.read_document_text(&target)?)
            } else {
                None
            };
            return self.host.submit_proposal(PendingFileOperation {
                kind: if exists { "overwrite" } else { "write" }.into(),
                title: agent_path_basename(&target),
                summary: previous.as_ref().map_or_else(
                    || {
                        format!(
                            "新建文件 {}（{} 字符）",
                            agent_path_basename(&target),
                            content.encode_utf16().count()
                        )
                    },
                    |before| {
                        format!(
                            "覆盖写入 {}（{} → {} 字符）",
                            agent_path_basename(&target),
                            before.encode_utf16().count(),
                            content.encode_utf16().count()
                        )
                    },
                ),
                content: Some(content),
                previous_content: previous,
                path: target,
                new_path: None,
                entries: Vec::new(),
                reason: "permission-suggest".into(),
            });
        }

        if let Some(parent) = Path::new(&target).parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建父目录失败: {e}"))?;
        }
        if is_base {
            crate::files::FileService::new()
                .write_file_safe(&target, &content)
                .map_err(|error| format!("写入失败: {error}"))?;
        } else {
            std::fs::write(&target, content.as_bytes()).map_err(|e| format!("写入失败: {e}"))?;
        }
        self.host.notify_change(FileChange::Write {
            path: target.clone(),
        });

        Ok(json!({
            "path": target,
            "bytes": content.encode_utf16().count(),
            "overwritten": exists,
        }))
    }

    fn file_delete(&self, args: &ToolArgs) -> ToolOutcome {
        let target = self.resolve(args.str_opt("path"))?;
        let meta = std::fs::metadata(&target).map_err(|e| format!("读取失败: {e}"))?;
        if meta.is_dir() {
            return Err("目标是文件夹，不是文件".into());
        }

        if self.host.should_propose(&target) {
            return self.host.submit_proposal(PendingFileOperation {
                kind: "delete-file".into(),
                title: agent_path_basename(&target),
                summary: format!("删除文件 {}", agent_path_basename(&target)),
                path: target,
                new_path: None,
                content: None,
                previous_content: None,
                entries: Vec::new(),
                reason: "destructive".into(),
            });
        }

        std::fs::remove_file(&target).map_err(|e| format!("删除失败: {e}"))?;
        self.host.notify_change(FileChange::Delete {
            path: target.clone(),
        });
        Ok(json!({ "path": target, "deleted": true }))
    }

    fn folder_delete(&self, args: &ToolArgs) -> ToolOutcome {
        let target = self.resolve(args.str_opt("path"))?;
        let meta = std::fs::metadata(&target).map_err(|e| format!("读取失败: {e}"))?;
        if !meta.is_dir() {
            return Err("目标是文件，不是文件夹".into());
        }

        if self.host.should_propose(&target) {
            let entries: Vec<String> = std::fs::read_dir(&target)
                .map(|it| {
                    it.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            return self.host.submit_proposal(PendingFileOperation {
                kind: "delete-folder".into(),
                title: agent_path_basename(&target),
                summary: format!(
                    "递归删除文件夹 {}（含 {} 个直接子项）",
                    agent_path_basename(&target),
                    entries.len()
                ),
                path: target,
                new_path: None,
                content: None,
                previous_content: None,
                entries,
                reason: "destructive".into(),
            });
        }

        std::fs::remove_dir_all(&target).map_err(|e| format!("删除失败: {e}"))?;
        self.host.notify_change(FileChange::Delete {
            path: target.clone(),
        });
        Ok(json!({ "path": target, "deleted": true }))
    }

    fn path_rename(&self, args: &ToolArgs) -> ToolOutcome {
        let old_path = self.resolve(args.str_opt("oldPath"))?;
        let new_path = self.resolve(args.str_opt("newPath"))?;

        if self.host.should_propose(&old_path) || self.host.should_propose(&new_path) {
            return self.host.submit_proposal(PendingFileOperation {
                kind: "rename".into(),
                title: agent_path_basename(&old_path),
                summary: format!(
                    "重命名 {} → {}",
                    agent_path_basename(&old_path),
                    agent_path_basename(&new_path)
                ),
                path: old_path,
                new_path: Some(new_path),
                content: None,
                previous_content: None,
                entries: Vec::new(),
                reason: "destructive".into(),
            });
        }

        if Path::new(&new_path).exists() {
            return Err("目标路径已存在".into());
        }
        if let Some(parent) = Path::new(&new_path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建父目录失败: {e}"))?;
        }
        crate::rename::with_companions(Path::new(&old_path), Path::new(&new_path))
            .map_err(|e| format!("重命名失败: {e}"))?;
        let metadata_error =
            crate::sub_documents::update_path(self.host.workspace_root(), &old_path, &new_path)
                .err();
        self.host.notify_change(FileChange::Rename {
            path: old_path.clone(),
            new_path: new_path.clone(),
        });
        if let Some(error) = metadata_error {
            return Err(format!("文件已改名，但子文档索引更新失败：{error}"));
        }
        Ok(json!({ "oldPath": old_path, "newPath": new_path }))
    }

    fn content_search(&self, args: &ToolArgs) -> ToolOutcome {
        let query = args.str_required("query")?.to_owned();
        if query.trim().is_empty() {
            return Err("搜索关键词不能为空".into());
        }
        let root = self.resolve(args.str_opt("path"))?;
        let Some(search) = &self.search else {
            return Err("搜索索引不可用".into());
        };

        // 对齐 TS：Math.max(1, Math.min(Number(maxResults) || 20, 50))
        let max_results = args
            .i64_opt("maxResults")
            .filter(|n| *n != 0)
            .unwrap_or(20)
            .clamp(1, 50) as usize;

        let outcome = search.search(
            &query,
            &SearchOptions {
                max_results,
                ..Default::default()
            },
        );

        let mut results = Vec::new();
        for group in outcome.groups {
            let path = crate::paths::to_forward_slashes(&group.path);
            // 只保留 root 之下且对 AI 可见的
            if !crate::paths::path_is_within(Path::new(&root), Path::new(&path))
                || !self.host.is_path_visible(&path)
            {
                continue;
            }
            for line in group.matches {
                results.push(json!({
                    "path": path,
                    "line": line.line,
                    "content": line.content,
                    "score": line.score,
                    "matchStart": line.match_start,
                    "matchEnd": line.match_end,
                }));
                if results.len() >= max_results {
                    break;
                }
            }
            if results.len() >= max_results {
                break;
            }
        }
        Ok(json!({ "query": query, "root": root, "results": results }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::{AiToolCall, AiToolFunction};
    use crate::ai::permission::{AiPermissionLevel, AiPermissionService};
    use crate::ai::tools::ToolRegistry;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct TestHost {
        root: PathBuf,
        propose_under: Option<String>,
        invisible_under: Option<String>,
        changes: Mutex<Vec<FileChange>>,
        proposals: Mutex<Vec<PendingFileOperation>>,
    }

    impl ToolHost for TestHost {
        fn workspace_root(&self) -> &Path {
            &self.root
        }
        fn is_path_visible(&self, p: &str) -> bool {
            self.invisible_under
                .as_ref()
                .is_none_or(|pre| !p.starts_with(pre))
        }
        fn should_propose(&self, p: &str) -> bool {
            self.propose_under
                .as_ref()
                .is_some_and(|pre| p.starts_with(pre))
        }
        fn submit_proposal(&self, op: PendingFileOperation) -> Result<Value, String> {
            self.proposals.lock().unwrap().push(op.clone());
            Ok(json!({ "pendingFileOperation": { "kind": op.kind, "path": op.path } }))
        }
        fn notify_change(&self, change: FileChange) {
            self.changes.lock().unwrap().push(change);
        }
    }

    struct Fixture {
        root: PathBuf,
        host: Arc<TestHost>,
        registry: ToolRegistry,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn fixture(tag: &str) -> Fixture {
        fixture_with(tag, None, None)
    }

    fn fixture_with(
        tag: &str,
        propose_under: Option<&str>,
        invisible_under: Option<&str>,
    ) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-coretool-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fwd = crate::paths::to_forward_slashes(&root.to_string_lossy());

        let host = Arc::new(TestHost {
            root: PathBuf::from(&fwd),
            propose_under: propose_under.map(|s| format!("{fwd}/{s}")),
            invisible_under: invisible_under.map(|s| format!("{fwd}/{s}")),
            changes: Mutex::new(Vec::new()),
            proposals: Mutex::new(Vec::new()),
        });
        let perms = Arc::new(AiPermissionService::new(&root));
        let registry = ToolRegistry::new(perms).with(Arc::new(CoreToolExecutor::new(host.clone())));
        Fixture {
            root,
            host,
            registry,
        }
    }

    impl Fixture {
        fn run(&self, name: &str, args: Value) -> Value {
            let call = AiToolCall {
                id: "c".into(),
                kind: "function".into(),
                function: AiToolFunction {
                    name: name.into(),
                    arguments: args.to_string(),
                },
            };
            serde_json::from_str(&self.registry.execute(&call)).unwrap()
        }
        fn write(&self, rel: &str, content: &str) -> PathBuf {
            let p = self.root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
            p
        }
        fn fwd(&self, rel: &str) -> String {
            crate::paths::to_forward_slashes(&self.root.join(rel).to_string_lossy())
        }
    }

    #[test]
    fn file_write_then_read_round_trip() {
        let f = fixture("rw");
        let out = f.run(
            "file_write",
            json!({ "path": "笔记.md", "content": "# 标题\n正文" }),
        );
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["data"]["overwritten"], false);
        assert_eq!(out["data"]["bytes"], 7, "bytes 应是 UTF-16 码元数");

        let back = f.run("file_read", json!({ "path": "笔记.md" }));
        assert_eq!(back["data"]["content"], "# 标题\n正文");
    }

    #[test]
    fn create_mode_refuses_to_overwrite() {
        let f = fixture("create-mode");
        f.write("a.md", "旧内容");
        let out = f.run("file_write", json!({ "path": "a.md", "content": "新" }));
        assert_eq!(out["ok"], false);
        assert!(
            out["error"].as_str().unwrap().contains("mode=create"),
            "{out}"
        );
        assert_eq!(
            std::fs::read_to_string(f.root.join("a.md")).unwrap(),
            "旧内容"
        );
    }

    #[test]
    fn overwrite_mode_replaces_and_reports_it() {
        let f = fixture("overwrite");
        f.write("a.md", "旧");
        let out = f.run(
            "file_write",
            json!({ "path": "a.md", "content": "新", "mode": "overwrite" }),
        );
        assert_eq!(out["data"]["overwritten"], true);
        assert_eq!(std::fs::read_to_string(f.root.join("a.md")).unwrap(), "新");
    }

    #[test]
    fn file_read_rejects_directories_and_huge_files() {
        let f = fixture("read-guard");
        std::fs::create_dir_all(f.root.join("目录")).unwrap();
        assert!(f.run("file_read", json!({ "path": "目录" }))["error"]
            .as_str()
            .unwrap()
            .contains("文件夹"));

        f.write("big.md", &"x".repeat((MAX_READ_BYTES + 1) as usize));
        assert!(f.run("file_read", json!({ "path": "big.md" }))["error"]
            .as_str()
            .unwrap()
            .contains("1MB"));
    }

    #[test]
    fn folder_list_hides_dotfiles_and_invisible_paths() {
        let f = fixture_with("list", None, Some("私密"));
        f.write("公开/a.md", "x");
        f.write("私密/b.md", "x");
        f.write(".隐藏/c.md", "x");
        f.write("链接.link.json", "{}");

        let out = f.run("folder_list", json!({}));
        let children = out["data"]["children"].as_array().unwrap();
        let names: Vec<&str> = children
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();

        assert!(names.contains(&"公开"));
        assert!(
            !names.contains(&"私密"),
            "invisible 路径应被隐藏: {names:?}"
        );
        assert!(!names.contains(&".隐藏"), "点目录应被跳过: {names:?}");

        let link = children
            .iter()
            .find(|c| c["name"] == "链接.link.json")
            .unwrap();
        assert_eq!(link["type"], "link", ".link.json 应识别为 link 类型");
    }

    #[test]
    fn folder_create_validates_the_name() {
        let f = fixture("mkdir");
        assert_eq!(
            f.run("folder_create", json!({ "name": "新目录" }))["ok"],
            true
        );
        assert!(f.root.join("新目录").is_dir());

        for bad in ["", "a/b", ".."] {
            let out = f.run("folder_create", json!({ "name": bad }));
            assert_eq!(out["ok"], false, "{bad:?} 应被拒: {out}");
        }
    }

    #[test]
    fn delete_tools_check_the_target_type() {
        let f = fixture("delete-type");
        f.write("文件.md", "x");
        std::fs::create_dir_all(f.root.join("目录")).unwrap();

        assert!(f.run("file_delete", json!({ "path": "目录" }))["error"]
            .as_str()
            .unwrap()
            .contains("文件夹"));
        assert!(
            f.run("folder_delete", json!({ "path": "文件.md" }))["error"]
                .as_str()
                .unwrap()
                .contains("文件")
        );
    }

    #[test]
    fn delete_removes_and_notifies() {
        let f = fixture("delete");
        f.write("弃用.md", "x");
        assert_eq!(
            f.run("file_delete", json!({ "path": "弃用.md" }))["data"]["deleted"],
            true
        );
        assert!(!f.root.join("弃用.md").exists());
        assert!(matches!(
            f.host.changes.lock().unwrap().last(),
            Some(FileChange::Delete { .. })
        ));
    }

    #[test]
    fn rename_moves_and_refuses_to_clobber() {
        let f = fixture("rename");
        f.write("旧.md", "内容");
        f.write("占位.md", "别动我");

        let out = f.run(
            "path_rename",
            json!({ "oldPath": "旧.md", "newPath": "新.md" }),
        );
        assert_eq!(out["ok"], true, "{out}");
        assert!(f.root.join("新.md").exists() && !f.root.join("旧.md").exists());

        let clash = f.run(
            "path_rename",
            json!({ "oldPath": "新.md", "newPath": "占位.md" }),
        );
        assert_eq!(clash["ok"], false, "不该覆盖已有文件");
        assert_eq!(
            std::fs::read_to_string(f.root.join("占位.md")).unwrap(),
            "别动我"
        );
    }

    /// suggest 级路径上的写操作必须转成提案，**不能直接落盘**。
    /// C# 版完全没有这个机制，等于「建议」档退化成「放行」。
    #[test]
    fn suggest_level_paths_produce_proposals_instead_of_writing() {
        let f = fixture_with("propose", Some("草稿"), None);
        f.write("草稿/a.md", "原内容");

        let out = f.run(
            "file_write",
            json!({ "path": "草稿/a.md", "content": "改后", "mode": "overwrite" }),
        );
        assert_eq!(out["ok"], true);
        assert!(
            out["data"]["pendingFileOperation"].is_object(),
            "应产出提案: {out}"
        );
        assert_eq!(
            std::fs::read_to_string(f.root.join("草稿/a.md")).unwrap(),
            "原内容",
            "提案阶段绝不能落盘"
        );

        let proposals = f.host.proposals.lock().unwrap();
        assert_eq!(proposals[0].kind, "overwrite");
        assert!(proposals[0].summary.contains("→"), "摘要应给出字符数变化");
    }

    #[test]
    fn suggest_level_delete_also_becomes_a_proposal() {
        let f = fixture_with("propose-del", Some("草稿"), None);
        f.write("草稿/a.md", "x");
        f.run("file_delete", json!({ "path": "草稿/a.md" }));
        assert!(f.root.join("草稿/a.md").exists(), "提案阶段不该真删");
        assert_eq!(f.host.proposals.lock().unwrap()[0].kind, "delete-file");
    }

    #[test]
    fn folder_delete_proposal_lists_affected_entries() {
        let f = fixture_with("propose-dir", Some("草稿"), None);
        f.write("草稿/子/a.md", "x");
        f.write("草稿/b.md", "x");
        f.run("folder_delete", json!({ "path": "草稿" }));

        let proposals = f.host.proposals.lock().unwrap();
        assert_eq!(proposals[0].kind, "delete-folder");
        assert_eq!(
            proposals[0].entries.len(),
            2,
            "应列出直接子项让用户知道波及范围"
        );
    }

    #[test]
    fn creating_a_new_file_under_suggest_creates_a_proposal() {
        let f = fixture_with("propose-new", Some("草稿"), None);
        let out = f.run(
            "file_write",
            json!({ "path": "草稿/新的.md", "content": "x" }),
        );
        assert_eq!(out["data"]["pendingFileOperation"]["kind"], "write");
        assert!(!f.root.join("草稿/新的.md").exists());
        assert_eq!(f.host.proposals.lock().unwrap().len(), 1);
    }

    #[test]
    fn creating_a_folder_under_suggest_creates_a_proposal() {
        let f = fixture_with("propose-folder", Some("草稿"), None);
        let out = f.run(
            "folder_create",
            json!({ "parentPath": "草稿", "name": "新目录" }),
        );
        assert_eq!(out["data"]["pendingFileOperation"]["kind"], "create-folder");
        assert!(!f.root.join("草稿/新目录").exists());
    }

    /// 权限门由 registry 强制：只读目录下的写入根本到不了执行体。
    #[test]
    fn readonly_paths_are_blocked_by_the_registry() {
        let f = fixture("readonly");
        f.registry
            .permissions()
            .set_folder_permission(&f.fwd("参考"), AiPermissionLevel::ReadOnly)
            .unwrap();
        f.write("参考/a.md", "原内容");

        let out = f.run(
            "file_write",
            json!({ "path": "参考/a.md", "content": "改", "mode": "overwrite" }),
        );
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().contains("只读"), "{out}");
        assert_eq!(
            std::fs::read_to_string(f.root.join("参考/a.md")).unwrap(),
            "原内容"
        );

        assert_eq!(
            f.run("file_read", json!({ "path": "参考/a.md" }))["ok"],
            true
        );
    }

    /// 改名影响两端，任一端只读就该整体拒绝。
    #[test]
    fn rename_checks_both_endpoints() {
        let f = fixture("rename-perm");
        f.registry
            .permissions()
            .set_folder_permission(&f.fwd("锁定"), AiPermissionLevel::ReadOnly)
            .unwrap();
        f.write("自由/a.md", "x");

        let out = f.run(
            "path_rename",
            json!({ "oldPath": "自由/a.md", "newPath": "锁定/a.md" }),
        );
        assert_eq!(out["ok"], false, "目标落在只读目录，应整体拒绝: {out}");
        assert!(f.root.join("自由/a.md").exists());
    }

    #[test]
    fn path_escapes_are_rejected() {
        let f = fixture("escape");
        for evil in ["../外面.md", ".mochi/index.db"] {
            let out = f.run("file_read", json!({ "path": evil }));
            assert_eq!(out["ok"], false, "{evil} 应被拒: {out}");
        }
    }

    #[test]
    fn content_search_requires_an_index() {
        let f = fixture("search-noindex");
        let out = f.run("content_search", json!({ "query": "目标" }));
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().contains("索引"), "{out}");
    }

    #[test]
    fn content_search_rejects_an_empty_query() {
        let f = fixture("search-empty");
        assert_eq!(f.run("content_search", json!({ "query": "" }))["ok"], false);
        assert_eq!(f.run("content_search", json!({}))["ok"], false);
    }
}
