//! UI 状态由 ToolHost 提供；无头宿主仍从磁盘列出知识库。

use std::sync::Arc;

use serde_json::json;

use super::host::{join_agent_path, validate_name, ToolHost};
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;
use crate::paths;

const TOOL_NAMES: &[&str] = &[
    "workspace_get_state",
    "current_document_get",
    "kb_list",
    "kb_select",
    "kb_create",
];

pub struct WorkspaceToolExecutor {
    host: Arc<dyn ToolHost>,
}

impl WorkspaceToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host }
    }

    fn workspace(&self) -> String {
        paths::to_forward_slashes(&self.host.workspace_root().to_string_lossy())
    }

    /// 按名称或路径找知识库。TS 两种都认——模型经常直接给名字。
    fn find_kb(&self, name_or_path: &str) -> Option<super::host::KnowledgeBase> {
        let needle = paths::to_forward_slashes(name_or_path);
        self.host
            .knowledge_bases()
            .into_iter()
            .find(|kb| kb.path == needle || kb.name == name_or_path)
    }
}

impl ToolExecutor for WorkspaceToolExecutor {
    fn handles(&self, name: &str) -> bool {
        TOOL_NAMES.contains(&name)
    }

    fn required_actions(&self, name: &str, args: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        match name {
            // 只报工作区自身的元信息，不碰任何文件
            "workspace_get_state" => Vec::new(),
            "kb_list" => vec![(AiToolAction::ReadFile, None)],
            "current_document_get" => vec![(
                AiToolAction::ReadFile,
                self.host.active_document().map(|d| d.path),
            )],
            "kb_select" => vec![(
                AiToolAction::ReadFile,
                args.str_opt("nameOrPath")
                    .and_then(|v| self.find_kb(v))
                    .map(|kb| kb.path),
            )],
            "kb_create" => vec![(
                AiToolAction::WriteFile,
                args.str_opt("name")
                    .map(|n| join_agent_path(&self.workspace(), n)),
            )],
            _ => Vec::new(),
        }
    }

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        match name {
            "workspace_get_state" => {
                let selected = self.host.selected_knowledge_base();
                let selected_path = selected
                    .as_ref()
                    .map(|p| paths::to_forward_slashes(&p.to_string_lossy()));
                let selected_kb = selected_path.as_ref().and_then(|p| self.find_kb(p));
                Ok(json!({
                    "workspacePath": self.workspace(),
                    "selectedKnowledgeBasePath": selected_path,
                    "selectedKnowledgeBase": selected_kb,
                    "activeDocument": self.host.active_document().map(|d| json!({
                        "id": d.id, "title": d.title, "path": d.path, "isDirty": d.is_dirty,
                    })),
                    "knowledgeBases": self.host.knowledge_bases(),
                }))
            }

            "current_document_get" => {
                let doc = self
                    .host
                    .active_document()
                    .ok_or("当前没有打开的活动文档")?;
                // **始终从磁盘读**：编辑器里存的是 HTML，给模型要的是原始 Markdown。
                // TS 那边专门为此加了注释，是个容易踩的坑。
                let content = self
                    .host
                    .read_document_text(&doc.path)
                    .map_err(|e| format!("读取当前文档失败: {e}"))?;
                Ok(json!({
                    "id": doc.id,
                    "title": doc.title,
                    "path": doc.path,
                    "isDirty": doc.is_dirty,
                    "source": "file",
                    "content": content,
                }))
            }

            "kb_list" => {
                // 设为不可见的知识库对 AI 完全不可访问
                let visible: Vec<_> = self
                    .host
                    .knowledge_bases()
                    .into_iter()
                    .filter(|kb| self.host.is_path_visible(&kb.path))
                    .collect();
                Ok(json!({ "knowledgeBases": visible }))
            }

            "kb_select" => {
                let name_or_path = args.str_required("nameOrPath")?;
                let target = self.find_kb(name_or_path).ok_or("未找到知识库")?;
                self.host.select_knowledge_base(&target.path)?;
                Ok(json!({ "selectedKnowledgeBase": target }))
            }

            "kb_create" => {
                let name = args.str_required("name")?.trim().to_owned();
                validate_name(&name)?;
                let target = join_agent_path(&self.workspace(), &name);
                std::fs::create_dir_all(&target).map_err(|e| format!("创建知识库失败: {e}"))?;
                // 建完顺手切过去——TS 就是这么做的，符合"新建即进入"的直觉。
                // 无头环境下切换会失败，但目录已经建好，不该因此报错。
                let _ = self.host.select_knowledge_base(&target);
                self.host
                    .notify_change(super::host::FileChange::FolderCreate {
                        path: target.clone(),
                    });
                Ok(json!({ "path": target, "name": name }))
            }

            other => Err(format!("未知的工作区工具: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::{AiToolCall, AiToolFunction};
    use crate::ai::permission::{AiPermissionLevel, AiPermissionService};
    use crate::ai::tools::host::{ActiveDocument, HeadlessHost};
    use crate::ai::tools::ToolRegistry;
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct UiHost {
        root: PathBuf,
        active: Option<ActiveDocument>,
        selected: Mutex<Option<PathBuf>>,
        invisible: Option<String>,
    }

    impl ToolHost for UiHost {
        fn workspace_root(&self) -> &Path {
            &self.root
        }
        fn selected_knowledge_base(&self) -> Option<PathBuf> {
            self.selected.lock().unwrap().clone()
        }
        fn active_document(&self) -> Option<ActiveDocument> {
            self.active.clone()
        }
        fn is_path_visible(&self, p: &str) -> bool {
            self.invisible
                .as_ref()
                .is_none_or(|pre| !p.starts_with(pre))
        }
        fn select_knowledge_base(&self, path: &str) -> Result<(), String> {
            *self.selected.lock().unwrap() = Some(PathBuf::from(path));
            Ok(())
        }
    }

    struct Fixture {
        root: PathBuf,
        host: Arc<UiHost>,
        registry: ToolRegistry,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn fixture(tag: &str, active: Option<&str>, invisible: Option<&str>) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-wstool-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fwd = paths::to_forward_slashes(&root.to_string_lossy());

        for kb in ["知识库", "刷题库"] {
            std::fs::create_dir_all(root.join(kb)).unwrap();
        }
        std::fs::create_dir_all(root.join(".mochi")).unwrap();

        let active_doc = active.map(|rel| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "# 当前文档\n正文").unwrap();
            ActiveDocument {
                id: "tab-1".into(),
                title: rel.into(),
                path: paths::to_forward_slashes(&p.to_string_lossy()),
                is_dirty: false,
            }
        });

        let host = Arc::new(UiHost {
            root: PathBuf::from(&fwd),
            active: active_doc,
            selected: Mutex::new(None),
            invisible: invisible.map(|s| format!("{fwd}/{s}")),
        });
        let perms = Arc::new(AiPermissionService::new(&root));
        let registry =
            ToolRegistry::new(perms).with(Arc::new(WorkspaceToolExecutor::new(host.clone())));
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
    }

    #[test]
    fn workspace_state_reports_path_and_knowledge_bases() {
        let f = fixture("state", None, None);
        let out = f.run("workspace_get_state", json!({}));
        assert_eq!(out["ok"], true, "{out}");

        let kbs = out["data"]["knowledgeBases"].as_array().unwrap();
        let names: Vec<&str> = kbs.iter().map(|k| k["name"].as_str().unwrap()).collect();
        assert!(
            names.contains(&"知识库") && names.contains(&"刷题库"),
            "{names:?}"
        );
        assert!(!names.contains(&".mochi"), "点目录不该出现在知识库列表里");
        assert_eq!(kbs[0]["type"], "directory");
        assert!(out["data"]["activeDocument"].is_null());
    }

    #[test]
    fn current_document_reads_from_disk() {
        let f = fixture("current", Some("知识库/笔记.md"), None);
        let out = f.run("current_document_get", json!({}));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["data"]["source"], "file", "必须标明来自磁盘而非编辑器");
        assert_eq!(out["data"]["content"], "# 当前文档\n正文");
        assert_eq!(out["data"]["isDirty"], false);
    }

    #[test]
    fn current_document_without_an_open_tab_fails_clearly() {
        let f = fixture("no-doc", None, None);
        let out = f.run("current_document_get", json!({}));
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().contains("活动文档"), "{out}");
    }

    #[test]
    fn kb_list_hides_invisible_knowledge_bases() {
        let f = fixture("kb-hide", None, Some("刷题库"));
        let out = f.run("kb_list", json!({}));
        let names: Vec<&str> = out["data"]["knowledgeBases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["知识库"], "invisible 的知识库对 AI 应完全不存在");
    }

    #[test]
    fn kb_select_accepts_both_name_and_path() {
        let f = fixture("kb-select", None, None);
        assert_eq!(
            f.run("kb_select", json!({ "nameOrPath": "知识库" }))["ok"],
            true
        );
        assert!(f
            .host
            .selected_knowledge_base()
            .unwrap()
            .ends_with("知识库"));

        let path = paths::to_forward_slashes(&f.root.join("刷题库").to_string_lossy());
        assert_eq!(
            f.run("kb_select", json!({ "nameOrPath": path }))["ok"],
            true
        );
        assert!(f
            .host
            .selected_knowledge_base()
            .unwrap()
            .ends_with("刷题库"));
    }

    #[test]
    fn kb_select_on_a_missing_target_fails() {
        let f = fixture("kb-missing", None, None);
        let out = f.run("kb_select", json!({ "nameOrPath": "不存在的库" }));
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().contains("未找到"), "{out}");
    }

    #[test]
    fn kb_create_makes_the_directory_and_selects_it() {
        let f = fixture("kb-create", None, None);
        let out = f.run("kb_create", json!({ "name": "新库" }));
        assert_eq!(out["ok"], true, "{out}");
        assert!(f.root.join("新库").is_dir());
        assert!(
            f.host.selected_knowledge_base().unwrap().ends_with("新库"),
            "建完应自动切过去"
        );
    }

    /// 字符集对齐 TS：Windows 文件名的非法字符全部拒绝。
    #[test]
    fn kb_create_rejects_illegal_names() {
        let f = fixture("kb-bad-name", None, None);
        for bad in [
            "", "   ", "a/b", "a\\b", "a:b", "a*b", "a?b", "a<b", "a>b", "a\"b", "a|b",
        ] {
            let out = f.run("kb_create", json!({ "name": bad }));
            assert_eq!(out["ok"], false, "{bad:?} 应被拒: {out}");
        }
    }

    #[test]
    fn readonly_workspace_blocks_kb_create_but_not_listing() {
        let f = fixture("kb-perm", None, None);
        let fwd = paths::to_forward_slashes(&f.root.to_string_lossy());
        f.registry
            .permissions()
            .set_folder_permission(&fwd, AiPermissionLevel::ReadOnly)
            .unwrap();

        assert_eq!(f.run("kb_list", json!({}))["ok"], true, "只读不该挡住列举");
        let out = f.run("kb_create", json!({ "name": "新库" }));
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().contains("只读"), "{out}");
        assert!(!f.root.join("新库").exists());
    }

    /// 无头环境下知识库列表仍应从磁盘列出来——后台任务该看得见工作区。
    #[test]
    fn headless_host_still_lists_knowledge_bases() {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-wstool-headless-{}-{n}", std::process::id()));
        std::fs::create_dir_all(root.join("知识库")).unwrap();

        let host = Arc::new(HeadlessHost::new(&root));
        assert_eq!(host.knowledge_bases().len(), 1);
        assert!(
            host.select_knowledge_base("任意").is_err(),
            "无头环境没有'选中'这回事"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
