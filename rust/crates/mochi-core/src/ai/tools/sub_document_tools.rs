//! 伴生目录被文件监听忽略，创建子文档后要显式通知宿主刷新文件树。

use std::sync::Arc;

use serde_json::json;

use super::host::{resolve_workspace_path, validate_name, FileChange, ResolveOptions, ToolHost};
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;
use crate::app_settings::{self, AppSettings, SettingValue};
use crate::sub_documents;

const TOOL_NAME: &str = "subdocument_create";

pub struct SubDocumentToolExecutor {
    host: Arc<dyn ToolHost>,
    settings: Option<Arc<AppSettings>>,
}

impl SubDocumentToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self {
            host,
            settings: None,
        }
    }

    pub fn with_settings(mut self, settings: Arc<AppSettings>) -> Self {
        self.settings = Some(settings);
        self
    }

    fn default_extension(&self) -> &'static str {
        let simple = self
            .settings
            .as_ref()
            .and_then(|settings| {
                app_settings::descriptor("editor.simpleDocumentMode")
                    .map(|descriptor| settings.read(descriptor))
            })
            .is_none_or(|value| value != SettingValue::Bool(false));
        if simple {
            ".md"
        } else {
            ".mc"
        }
    }

    /// 父文档：给了 parentPath 就用它，否则用当前活动文档。
    /// 相对路径以工作区根为基准（对齐 TS 的 `resolvePath(raw, false)`）。
    fn resolve_parent(&self, args: &ToolArgs) -> Result<String, String> {
        let input = match args.str_opt("parentPath") {
            Some(path) => path.to_owned(),
            None => {
                self.host
                    .active_document()
                    .ok_or("未指定 parentPath，且当前没有打开的活动文档")?
                    .path
            }
        };
        resolve_workspace_path(
            self.host.as_ref(),
            Some(&input),
            ResolveOptions {
                fallback_to_selection: false,
                allow_mochi_dir: false,
            },
        )
    }
}

impl ToolExecutor for SubDocumentToolExecutor {
    fn handles(&self, name: &str) -> bool {
        name == TOOL_NAME
    }

    fn required_actions(
        &self,
        _name: &str,
        args: &ToolArgs,
    ) -> Vec<(AiToolAction, Option<String>)> {
        // 权限按**父文档**判定：子文档落在父文档旁的伴生夹里，
        // 父文档所在目录是只读的，就不该往它旁边塞东西
        vec![(AiToolAction::WriteFile, self.resolve_parent(args).ok())]
    }

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        if name != TOOL_NAME {
            return Err(format!("未知的子文档工具: {name}"));
        }

        let parent_path = self.resolve_parent(args)?;
        // 父文档必须是已存在的**文件**：挂到目录上没有意义，
        // 而且伴生夹的命名规则（.<全名>.sub）会造出一个诡异的兄弟目录
        let metadata = std::fs::metadata(&parent_path).map_err(|e| format!("父文档不可用: {e}"))?;
        if metadata.is_dir() {
            return Err("父文档必须是文件，不能是文件夹".into());
        }

        let raw_name = args.str_required("name")?.trim().to_owned();
        validate_name(&raw_name)?;
        let name = if has_extension(&raw_name) {
            raw_name
        } else {
            format!("{raw_name}{}", self.default_extension())
        };

        let content = args.str_opt("content").unwrap_or_default();
        let content = self
            .host
            .transform_content_before_write(&parent_path, content);

        let child_path =
            sub_documents::create(self.host.workspace_root(), &parent_path, &name, &content)
                .map_err(|e| format!("创建子文档失败: {e}"))?;

        // 伴生夹是隐藏目录，文件监听不会报事件——必须显式让文件树按索引重建
        self.host.notify_change(FileChange::SubDocumentCreate {
            parent_path: parent_path.clone(),
            path: child_path.clone(),
        });

        Ok(json!({
            "parentPath": parent_path,
            "path": child_path,
            // 回报**实际**用的名字：重名时会变成「试题 1.mc」，
            // 报入参会让模型以为文件叫另一个名字
            "name": child_path.rsplit('/').next().unwrap_or(&name),
        }))
    }
}

/// 末尾是否已有扩展名。对齐 TS 的 `/\.[^./\\]+$/`——
/// `笔记.md` 有，`版本 1.` 没有，`a/b.` 也没有。
fn has_extension(name: &str) -> bool {
    match name.rfind('.') {
        Some(dot) => {
            let ext = &name[dot + 1..];
            !ext.is_empty() && !ext.contains(['.', '/', '\\'])
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::{AiToolCall, AiToolFunction};
    use crate::ai::permission::{AiPermissionLevel, AiPermissionService};
    use crate::ai::tools::host::ActiveDocument;
    use crate::ai::tools::ToolRegistry;
    use crate::paths;
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct DocHost {
        root: PathBuf,
        active: Option<ActiveDocument>,
        changes: Mutex<Vec<FileChange>>,
    }

    impl ToolHost for DocHost {
        fn workspace_root(&self) -> &Path {
            &self.root
        }
        fn active_document(&self) -> Option<ActiveDocument> {
            self.active.clone()
        }
        fn notify_change(&self, change: FileChange) {
            self.changes.lock().unwrap().push(change);
        }
    }

    struct Fixture {
        root: PathBuf,
        host: Arc<DocHost>,
        registry: ToolRegistry,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    impl Fixture {
        fn run(&self, args: Value) -> Value {
            let call = AiToolCall {
                id: "c".into(),
                kind: "function".into(),
                function: AiToolFunction {
                    name: TOOL_NAME.into(),
                    arguments: args.to_string(),
                },
            };
            serde_json::from_str(&self.registry.execute(&call)).unwrap()
        }
        fn ok(&self, args: Value) -> Value {
            let out = self.run(args);
            assert_eq!(out["ok"], true, "{out}");
            out["data"].clone()
        }
        fn err(&self, args: Value) -> String {
            let out = self.run(args);
            assert_eq!(out["ok"], false, "{out}");
            out["error"].as_str().unwrap().to_owned()
        }
        fn abs(&self, rel: &str) -> String {
            format!(
                "{}/{rel}",
                paths::to_forward_slashes(&self.root.to_string_lossy())
            )
        }
    }

    fn fixture(tag: &str, active: bool) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-subtool-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("知识库")).unwrap();
        std::fs::write(root.join("知识库").join("笔记.md"), "# 父文档").unwrap();
        let fwd = paths::to_forward_slashes(&root.to_string_lossy());

        let host = Arc::new(DocHost {
            root: PathBuf::from(&fwd),
            active: active.then(|| ActiveDocument {
                id: "tab-1".into(),
                title: "笔记.md".into(),
                path: format!("{fwd}/知识库/笔记.md"),
                is_dirty: false,
            }),
            changes: Mutex::new(Vec::new()),
        });
        let perms = Arc::new(AiPermissionService::new(&root));
        let registry =
            ToolRegistry::new(perms).with(Arc::new(SubDocumentToolExecutor::new(host.clone())));
        Fixture {
            root,
            host,
            registry,
        }
    }

    #[test]
    fn new_child_format_reads_the_current_setting_and_preserves_explicit_names() {
        let mut f = fixture("document-mode", false);
        let store = Arc::new(crate::settings::SettingsService::new(Some(
            f.root.join("settings.json"),
        )));
        let settings = Arc::new(AppSettings::new(store.clone()));
        let executor = SubDocumentToolExecutor::new(f.host.clone()).with_settings(settings);
        f.registry =
            ToolRegistry::new(Arc::new(AiPermissionService::new(&f.root))).with(Arc::new(executor));
        let args = json!({"parentPath":"知识库/笔记.md", "name":"跟随设置", "content":"# 原文"});
        assert_eq!(f.ok(args.clone())["name"], "跟随设置.md");
        store.set("app.editor.simpleDocumentMode", "false");
        assert_eq!(f.ok(args.clone())["name"], "跟随设置.mc");
        store.set("app.editor.simpleDocumentMode", "true");
        assert_eq!(f.ok(args)["name"], "跟随设置 1.md");
        let explicit =
            f.ok(json!({"parentPath":"知识库/笔记.md", "name":"指定.mc", "content":"# 原文"}));
        assert_eq!(explicit["name"], "指定.mc");
    }

    #[test]
    fn it_creates_the_child_in_the_hidden_sidecar() {
        let f = fixture("create", false);
        let data = f.ok(json!({
            "parentPath": "知识库/笔记.md", "name": "试题", "content": "# 试题\n\n1. 略",
        }));

        assert_eq!(data["name"], "试题.md", "简洁模式不带扩展名时补 .md");
        assert_eq!(data["parentPath"], f.abs("知识库/笔记.md"));

        let child = data["path"].as_str().unwrap();
        assert!(child.contains("/.笔记.md.sub/"), "{child}");
        assert_eq!(std::fs::read_to_string(child).unwrap(), "# 试题\n\n1. 略");
        assert_eq!(
            sub_documents::children_of(&f.host.root, &f.abs("知识库/笔记.md")),
            [child]
        );
    }

    #[test]
    fn an_explicit_extension_is_kept() {
        let f = fixture("ext", false);
        let data = f.ok(json!({
            "parentPath": "知识库/笔记.md", "name": "摘要.md", "content": "x",
        }));
        assert_eq!(data["name"], "摘要.md");
    }

    /// 伴生夹是隐藏目录，文件监听不报事件——不通知宿主，侧栏就不会刷新。
    #[test]
    fn the_host_is_told_to_rebuild_the_tree() {
        let f = fixture("notify", false);
        let data = f.ok(json!({ "parentPath": "知识库/笔记.md", "name": "试题", "content": "x" }));

        let changes = f.host.changes.lock().unwrap();
        assert_eq!(changes.len(), 1);
        match &changes[0] {
            FileChange::SubDocumentCreate { parent_path, path } => {
                assert_eq!(parent_path, data["parentPath"].as_str().unwrap());
                assert_eq!(path, data["path"].as_str().unwrap());
            }
            other => panic!("变更类型不对: {other:?}"),
        }
    }

    #[test]
    fn it_falls_back_to_the_active_document() {
        let f = fixture("active", true);
        let data = f.ok(json!({ "name": "试题", "content": "x" }));
        assert_eq!(data["parentPath"], f.abs("知识库/笔记.md"));
    }

    #[test]
    fn without_a_parent_or_active_document_it_fails() {
        let f = fixture("no-parent", false);
        assert!(f
            .err(json!({ "name": "试题", "content": "x" }))
            .contains("活动文档"));
    }

    /// 挂到文件夹上会造出一个诡异的兄弟目录，直接拒绝。
    #[test]
    fn a_directory_parent_is_refused() {
        let f = fixture("dir-parent", false);
        let err = f.err(json!({ "parentPath": "知识库", "name": "试题", "content": "x" }));
        assert!(err.contains("必须是文件"), "{err}");
    }

    #[test]
    fn a_missing_parent_fails_clearly() {
        let f = fixture("ghost-parent", false);
        assert!(f
            .err(json!({ "parentPath": "知识库/不存在.md", "name": "试题", "content": "x" }))
            .contains("父文档不可用"));
    }

    #[test]
    fn illegal_names_are_refused() {
        let f = fixture("bad-name", false);
        for bad in ["", "   ", "a/b", "a\\b", "a:b", "a?b", "..", "."] {
            let out = f.run(json!({ "parentPath": "知识库/笔记.md", "name": bad, "content": "x" }));
            assert_eq!(out["ok"], false, "{bad:?} 应被拒: {out}");
        }
    }

    /// 重名时回报**实际**文件名，否则模型会以为文件叫另一个名字。
    #[test]
    fn duplicate_names_report_the_actual_file_name() {
        let f = fixture("dup", false);
        let args = json!({ "parentPath": "知识库/笔记.md", "name": "试题", "content": "x" });
        assert_eq!(f.ok(args.clone())["name"], "试题.md");

        let second = f.ok(args);
        assert_eq!(second["name"], "试题 1.md");
        assert!(second["path"].as_str().unwrap().ends_with("试题 1.md"));
    }

    /// 权限按父文档判定：父文档所在目录只读，就不该往它旁边塞东西。
    #[test]
    fn a_read_only_parent_folder_blocks_creation() {
        let f = fixture("readonly", false);
        f.registry
            .permissions()
            .set_folder_permission(&f.abs("知识库"), AiPermissionLevel::ReadOnly)
            .unwrap();

        let err = f.err(json!({ "parentPath": "知识库/笔记.md", "name": "试题", "content": "x" }));
        assert!(err.contains("只读"), "{err}");
        assert!(!Path::new(&sub_documents::sidecar_path(&f.abs("知识库/笔记.md"))).exists());
    }

    #[test]
    fn extension_detection_matches_the_ts_regex() {
        assert!(has_extension("笔记.md"));
        assert!(has_extension("a.b.c"), "只看最后一段");
        assert!(
            has_extension(".gitignore"),
            "TS 的 /\\.[^./\\\\]+$/ 也认这个"
        );
        assert!(!has_extension("无扩展名"));
        assert!(!has_extension("结尾是点."));
    }
}
