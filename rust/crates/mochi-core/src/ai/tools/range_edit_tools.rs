//! 只生成提案，不写文件；确认时重新核对 originalHash，冲突则拒绝覆盖。

use std::sync::Arc;

#[cfg(test)]
use serde_json::json;

use super::host::{resolve_workspace_path, ResolveOptions, ToolHost};
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;
use crate::document_range::{self, PendingDocumentEdit};
use crate::{jstime, paths};

const TOOL_NAME: &str = "document_range_propose_edit";

pub struct RangeEditToolExecutor {
    host: Arc<dyn ToolHost>,
}

impl RangeEditToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host }
    }

    /// 目标文档：给了 path 就用它，否则用当前活动文档。
    /// **相对路径以工作区根为基准**（`fallback_to_selection: false`）——
    /// 局部改的是某篇具体文档，落到"当前选中的知识库"下会指错文件。
    fn resolve(&self, args: &ToolArgs) -> Result<String, String> {
        let input = match args.str_opt("path") {
            Some(path) => path.to_owned(),
            None => {
                self.host
                    .active_document()
                    .ok_or("当前没有打开的活动文档")?
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

impl ToolExecutor for RangeEditToolExecutor {
    fn handles(&self, name: &str) -> bool {
        name == TOOL_NAME
    }

    fn required_actions(
        &self,
        _name: &str,
        args: &ToolArgs,
    ) -> Vec<(AiToolAction, Option<String>)> {
        // 提案本身不落盘，但目标是「打算写这个文件」，按写入权限把关——
        // 只读目录下连改动建议都不该出现在用户面前
        vec![(AiToolAction::WriteFile, self.resolve(args).ok())]
    }

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        if name != TOOL_NAME {
            return Err(format!("未知的文档编辑工具: {name}"));
        }

        let target = self.resolve(args)?;
        let content = self
            .host
            .read_document_text(&target)
            .map_err(|e| format!("读取文档失败: {e}"))?;

        let start_line = args.i64_opt("startLine").unwrap_or(1).max(1) as usize;
        let end_line =
            (args.i64_opt("endLine").unwrap_or(start_line as i64).max(1) as usize).max(start_line);

        let old_text = document_range::line_range_text(&content, start_line, end_line);
        if old_text.is_empty() && start_line > document_range::line_count(&content) {
            return Err("指定行范围超出文档长度".into());
        }

        let edit = PendingDocumentEdit {
            id: format!("edit-{}-{}", jstime::now_millis(), paths::random_base36(6)),
            title: Some(basename(&target)),
            path: target,
            start_line,
            end_line,
            original_hash: document_range::hash_text(&old_text),
            old_text,
            new_text: args.str_opt("replacement").unwrap_or_default().to_owned(),
            summary: args
                .str_opt("summary")
                .map(str::to_owned)
                .unwrap_or_else(|| "局部修改建议".to_owned()),
            status: "pending".into(),
            error: None,
        };

        self.host.propose_document_edit(edit)
    }
}

fn basename(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::{AiToolCall, AiToolFunction};
    use crate::ai::permission::{AiPermissionLevel, AiPermissionService};
    use crate::ai::tools::host::ActiveDocument;
    use crate::ai::tools::ToolRegistry;
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    const DOC: &str = "# 标题\n第一段\n第二段\n第三段";

    struct DocHost {
        root: PathBuf,
        active: Option<ActiveDocument>,
        selected: Option<PathBuf>,
    }

    impl ToolHost for DocHost {
        fn workspace_root(&self) -> &Path {
            &self.root
        }
        fn active_document(&self) -> Option<ActiveDocument> {
            self.active.clone()
        }
        fn selected_knowledge_base(&self) -> Option<PathBuf> {
            self.selected.clone()
        }
    }

    struct Fixture {
        root: PathBuf,
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
            out["data"]["pendingEdit"].clone()
        }
        fn err(&self, args: Value) -> String {
            let out = self.run(args);
            assert_eq!(out["ok"], false, "{out}");
            out["error"].as_str().unwrap().to_owned()
        }
        fn abs(&self, rel: &str) -> String {
            format!("{}/{rel}", self.root.to_string_lossy().replace('\\', "/"))
        }
    }

    fn fixture(tag: &str, active: bool) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-rangetool-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("知识库")).unwrap();
        let fwd = root.to_string_lossy().replace('\\', "/");
        std::fs::write(root.join("知识库").join("笔记.md"), DOC).unwrap();

        let host = Arc::new(DocHost {
            root: PathBuf::from(&fwd),
            active: active.then(|| ActiveDocument {
                id: "tab-1".into(),
                title: "笔记.md".into(),
                path: format!("{fwd}/知识库/笔记.md"),
                is_dirty: false,
            }),
            // 故意选中一个知识库：相对路径**不该**以它为基准
            selected: Some(PathBuf::from(format!("{fwd}/知识库"))),
        });
        let perms = Arc::new(AiPermissionService::new(&root));
        let registry = ToolRegistry::new(perms).with(Arc::new(RangeEditToolExecutor::new(host)));
        Fixture { root, registry }
    }

    #[test]
    fn proposal_carries_the_original_text_and_hash() {
        let f = fixture("basic", false);
        let edit = f.ok(json!({
            "path": "知识库/笔记.md", "startLine": 3, "endLine": 3,
            "replacement": "改过的第二段", "summary": "润色第二段",
        }));

        assert_eq!(edit["oldText"], "第二段");
        assert_eq!(edit["newText"], "改过的第二段");
        assert_eq!(edit["summary"], "润色第二段");
        assert_eq!(edit["startLine"], 3);
        assert_eq!(edit["endLine"], 3);
        assert_eq!(edit["status"], "pending");
        assert_eq!(edit["title"], "笔记.md");
        assert_eq!(
            edit["originalHash"],
            document_range::hash_text("第二段"),
            "指纹必须是原文的，冲突检测全靠它"
        );
        assert!(edit["id"].as_str().unwrap().starts_with("edit-"));
    }

    /// 只产出提案，绝不落盘。
    #[test]
    fn the_document_is_left_untouched() {
        let f = fixture("no-write", false);
        f.ok(json!({
            "path": "知识库/笔记.md", "startLine": 1, "endLine": 4,
            "replacement": "全删了", "summary": "清空",
        }));
        assert_eq!(
            std::fs::read_to_string(f.abs("知识库/笔记.md")).unwrap(),
            DOC
        );
    }

    #[test]
    fn multi_line_ranges_are_inclusive() {
        let f = fixture("range", false);
        let edit = f.ok(json!({
            "path": "知识库/笔记.md", "startLine": 2, "endLine": 3,
            "replacement": "合并成一段", "summary": "合并",
        }));
        assert_eq!(edit["oldText"], "第一段\n第二段");
    }

    #[test]
    fn it_falls_back_to_the_active_document() {
        let f = fixture("active", true);
        let edit = f.ok(json!({
            "startLine": 1, "endLine": 1, "replacement": "# 新标题", "summary": "改标题",
        }));
        assert_eq!(edit["oldText"], "# 标题");
        assert!(edit["path"].as_str().unwrap().ends_with("知识库/笔记.md"));
    }

    #[test]
    fn without_a_path_or_active_document_it_fails() {
        let f = fixture("no-doc", false);
        assert!(f
            .err(json!({ "startLine": 1, "endLine": 1, "replacement": "x", "summary": "s" }))
            .contains("活动文档"));
    }

    /// 相对路径以工作区根为基准，不跟着"当前选中的知识库"跑。
    #[test]
    fn relative_paths_resolve_from_the_workspace_root() {
        let f = fixture("base", false);
        // 若以选中的「知识库」为基准，这里会去找 知识库/知识库/笔记.md 而失败
        let edit = f.ok(json!({
            "path": "知识库/笔记.md", "startLine": 1, "endLine": 1,
            "replacement": "x", "summary": "s",
        }));
        assert!(edit["path"].as_str().unwrap().ends_with("/知识库/笔记.md"));
    }

    #[test]
    fn ranges_past_the_end_are_refused() {
        let f = fixture("oob", false);
        let err = f.err(json!({
            "path": "知识库/笔记.md", "startLine": 99, "endLine": 100,
            "replacement": "x", "summary": "s",
        }));
        assert!(err.contains("超出文档长度"), "{err}");
    }

    /// 末尾越界要截断而不是报错——模型给的 endLine 常比实际大。
    #[test]
    fn an_over_long_end_line_is_clamped() {
        let f = fixture("clamp", false);
        let edit = f.ok(json!({
            "path": "知识库/笔记.md", "startLine": 3, "endLine": 999,
            "replacement": "x", "summary": "s",
        }));
        assert_eq!(edit["oldText"], "第二段\n第三段");
    }

    #[test]
    fn defaults_fill_in_for_missing_arguments() {
        let f = fixture("defaults", false);
        let edit = f.ok(json!({ "path": "知识库/笔记.md", "startLine": 2, "endLine": 2 }));
        assert_eq!(edit["newText"], "", "没给 replacement 视作删除该行");
        assert_eq!(edit["summary"], "局部修改建议");
    }

    /// 只读目录下连改动建议都不该出现在用户面前。
    #[test]
    fn read_only_folders_refuse_even_a_proposal() {
        let f = fixture("readonly", false);
        f.registry
            .permissions()
            .set_folder_permission(&f.abs("知识库"), AiPermissionLevel::ReadOnly)
            .unwrap();
        assert!(f
            .err(
                json!({ "path": "知识库/笔记.md", "startLine": 1, "endLine": 1,
                         "replacement": "x", "summary": "s" })
            )
            .contains("只读"));
    }

    #[test]
    fn a_missing_file_fails_clearly() {
        let f = fixture("missing", false);
        assert!(f
            .err(
                json!({ "path": "知识库/不存在.md", "startLine": 1, "endLine": 1,
                         "replacement": "x", "summary": "s" })
            )
            .contains("读取文档失败"));
    }
}
