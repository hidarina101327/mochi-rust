//! 批注与标注受 read_file 权限约束；无图形导出能力的宿主不暴露 document_export。

use std::sync::Arc;

use serde_json::json;

use super::host::{resolve_workspace_path, ResolveOptions, ToolHost};
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;
use crate::sidecars;

const TOOL_NAMES: &[&str] = &["comment_list", "annotation_list"];

pub struct DocumentToolExecutor {
    host: Arc<dyn ToolHost>,
}

impl DocumentToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host }
    }

    fn resolve(&self, args: &ToolArgs) -> Result<String, String> {
        resolve_workspace_path(
            self.host.as_ref(),
            args.str_opt("path"),
            ResolveOptions::default(),
        )
    }
}

impl ToolExecutor for DocumentToolExecutor {
    fn handles(&self, name: &str) -> bool {
        TOOL_NAMES.contains(&name)
            || (name == "document_export" && self.host.supports_document_export())
    }

    fn required_actions(&self, name: &str, args: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        // 解析失败时不声明路径；`call` 会用同样的解析再失败一次并给出原因
        let source = self.resolve(args).ok();
        let mut actions = vec![(AiToolAction::ReadFile, source.clone())];
        if name == "document_export" {
            let output = source
                .and_then(|p| {
                    crate::exports::output_path(
                        self.host.workspace_root(),
                        std::path::Path::new(&p),
                        args.str_opt("format").unwrap_or("pdf"),
                    )
                    .ok()
                })
                .map(|p| p.to_string_lossy().into_owned());
            actions.push((AiToolAction::WriteFile, output));
        }
        actions
    }

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        // path 是必填：批注/标注总是针对某一篇文档，落到"当前知识库目录"没有意义
        args.str_required("path")?;
        let path = self.resolve(args)?;

        match name {
            "document_export" => {
                let format = args.str_opt("format").unwrap_or("pdf");
                let output = crate::exports::output_path(
                    self.host.workspace_root(),
                    std::path::Path::new(&path),
                    format,
                )
                .map_err(|e| e.to_string())?;
                let output = output.to_string_lossy().into_owned();
                if self.host.should_propose(&output) {
                    return self.host.propose_document_export(&path, format, &output);
                }
                self.host.export_document(&path, format, &output)?;
                self.host.notify_change(super::host::FileChange::Write {
                    path: output.clone(),
                });
                Ok(json!({"sourcePath":path,"format":format,"outputPath":output}))
            }
            "comment_list" => {
                let file = sidecars::load_comments(&path);
                Ok(json!({
                    "path": path,
                    "count": file.comments.len(),
                    "comments": file.comments,
                }))
            }

            "annotation_list" => {
                let document = sidecars::load_pdf_annotations(&path);
                Ok(json!({
                    "path": path,
                    "count": document.annotations.len(),
                    // label 是 TS 侧 UI 现算的；这里一并给出，省得模型自己猜标注写了什么
                    "annotations": document
                        .annotations
                        .iter()
                        .map(|a| {
                            let mut value = serde_json::to_value(a).unwrap_or_else(|_| json!({}));
                            if let Some(object) = value.as_object_mut() {
                                object.insert("label".into(), json!(a.label()));
                            }
                            value
                        })
                        .collect::<Vec<_>>(),
                }))
            }

            other => Err(format!("未知的文档工具: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::{AiToolCall, AiToolFunction};
    use crate::ai::permission::{AiPermissionLevel, AiPermissionService};
    use crate::ai::tools::host::HeadlessHost;
    use crate::ai::tools::ToolRegistry;
    use crate::sidecars::{DocumentComment, PdfAnnotation};
    use serde_json::Value;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

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
        fn ok(&self, name: &str, args: Value) -> Value {
            let out = self.run(name, args);
            assert_eq!(out["ok"], true, "{out}");
            out["data"].clone()
        }
        fn err(&self, name: &str, args: Value) -> String {
            let out = self.run(name, args);
            assert_eq!(out["ok"], false, "{out}");
            out["error"].as_str().unwrap().to_owned()
        }
        fn abs(&self, rel: &str) -> String {
            format!("{}/{rel}", self.root.to_string_lossy().replace('\\', "/"))
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-doctool-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("知识库")).unwrap();

        let host = Arc::new(HeadlessHost::new(&root));
        let perms = Arc::new(AiPermissionService::new(&root));
        let registry = ToolRegistry::new(perms).with(Arc::new(DocumentToolExecutor::new(host)));
        Fixture { root, registry }
    }

    fn comment(id: &str, content: &str) -> DocumentComment {
        DocumentComment {
            resolved: false,
            id: id.into(),
            parent_id: None,
            target_type: "text".into(),
            author: "我".into(),
            content: content.into(),
            created_at: "2026-08-29T00:00:00.000Z".into(),
            updated_at: None,
            anchor: None,
            attachments: Vec::new(),
        }
    }

    #[test]
    fn comment_list_reads_the_sidecar() {
        let f = fixture("comments");
        let doc = f.abs("知识库/笔记.md");
        std::fs::write(&doc, "# 标题").unwrap();
        sidecars::save_comments(&doc, vec![comment("c1", "这段要改"), comment("c2", "同意")])
            .unwrap();

        let data = f.ok("comment_list", json!({ "path": "知识库/笔记.md" }));
        assert_eq!(data["count"], 2);
        assert_eq!(data["path"], doc, "回给模型的是解析后的绝对路径");
        assert_eq!(data["comments"][0]["content"], "这段要改");
        assert_eq!(
            data["comments"][0]["targetType"], "text",
            "字段名保持 camelCase"
        );
    }

    /// 没有批注不是错误——模型该看到"这篇文档没人批注过"，而不是一个失败。
    #[test]
    fn comment_list_on_an_unannotated_document_returns_zero() {
        let f = fixture("comments-empty");
        std::fs::write(f.abs("知识库/笔记.md"), "# 标题").unwrap();
        let data = f.ok("comment_list", json!({ "path": "知识库/笔记.md" }));
        assert_eq!(data["count"], 0);
        assert!(data["comments"].as_array().unwrap().is_empty());
    }

    #[test]
    fn annotation_list_includes_a_readable_label() {
        let f = fixture("annotations");
        let pdf = f.abs("知识库/论文.pdf");
        std::fs::write(&pdf, "%PDF-1.7").unwrap();
        sidecars::save_pdf_annotations(
            &pdf,
            vec![
                PdfAnnotation {
                    id: "a1".into(),
                    kind: "text".into(),
                    page: 2,
                    x: 0.1,
                    y: 0.1,
                    width: 0.2,
                    height: 0.05,
                    color: "#e11d48".into(),
                    text: Some("这里存疑".into()),
                    created_at: 1_700_000_000_000,
                    updated_at: 1_700_000_000_000,
                },
                PdfAnnotation {
                    id: "a2".into(),
                    kind: "circle".into(),
                    text: None,
                    ..serde_json::from_value(json!({
                        "id": "a2", "type": "circle", "page": 3,
                        "x": 0.0, "y": 0.0, "width": 0.1, "height": 0.1,
                        "color": "#e11d48", "createdAt": 1, "updatedAt": 1,
                    }))
                    .unwrap()
                },
            ],
        )
        .unwrap();

        let data = f.ok("annotation_list", json!({ "path": "知识库/论文.pdf" }));
        assert_eq!(data["count"], 2);
        assert_eq!(data["annotations"][0]["label"], "这里存疑");
        assert_eq!(data["annotations"][0]["page"], 2);
        assert_eq!(
            data["annotations"][1]["label"], "圆圈标注",
            "没有文字就按类型给标签"
        );
    }

    #[test]
    fn path_is_required() {
        let f = fixture("no-path");
        assert!(f.err("comment_list", json!({})).contains("path"));
        assert!(f.err("annotation_list", json!({})).contains("path"));
    }

    #[test]
    fn paths_outside_the_workspace_are_refused() {
        let f = fixture("escape");
        assert!(f
            .err("comment_list", json!({ "path": "../别的地方/x.md" }))
            .contains(".."));
        assert!(f
            .err("comment_list", json!({ "path": "C:/Windows/win.ini" }))
            .contains("超出当前工作区"));
    }

    /// 设为不可见的目录下也无法读取批注，因为批注是文档内容的一部分。
    #[test]
    fn invisible_folders_block_comment_reads() {
        let f = fixture("invisible");
        let doc = f.abs("知识库/笔记.md");
        std::fs::write(&doc, "# 标题").unwrap();
        sidecars::save_comments(&doc, vec![comment("c1", "秘密")]).unwrap();

        f.registry
            .permissions()
            .set_folder_permission(&f.abs("知识库"), AiPermissionLevel::Invisible)
            .unwrap();

        let err = f.err("comment_list", json!({ "path": "知识库/笔记.md" }));
        assert!(!err.contains("秘密"), "拒绝信息里不该泄漏内容: {err}");
    }
}
