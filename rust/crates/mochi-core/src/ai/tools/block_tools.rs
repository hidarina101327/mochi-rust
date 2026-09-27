//! 块定位使用持久 ID；替换由文档宿主处理，工具层负责权限、过期检查和审批提案。

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::host::{resolve_workspace_path, BlockEditProposal, ResolveOptions, ToolHost};
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;

const LIST_TOOL: &str = "document_blocks_list";
const READ_TOOL: &str = "document_block_read";
const PROPOSE_TOOL: &str = "document_block_propose_edit";
const MAX_BLOCK_EDITS: usize = 32;

/// 块列出/读取/提案操作的核心执行器。
pub struct BlockToolExecutor {
    host: Arc<dyn ToolHost>,
}

impl BlockToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host }
    }

    /// 块工具用具体的文档路径。相对路径即使选了知识库，也一律以
    /// 工作区为根，与范围编辑工具一致，避免编辑被改道到别的文件夹。
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

    fn required_text(value: Option<&Value>, key: &str) -> Result<String, String> {
        value
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("缺少必填参数 {key}"))
    }

    fn edit_values(args: &ToolArgs) -> Result<Vec<Value>, String> {
        if let Some(value) = args.get("edits") {
            let edits = value
                .as_array()
                .ok_or_else(|| "参数 edits 必须是数组".to_owned())?;
            if edits.is_empty() {
                return Err("参数 edits 不能为空".into());
            }
            if edits.len() > MAX_BLOCK_EDITS {
                return Err(format!("一次最多提出 {MAX_BLOCK_EDITS} 个块修改"));
            }
            return Ok(edits.clone());
        }

        // 保留单项操作形式，供只需编辑一次的调用方使用。对外数据结构采用
        // `edits`，同时接受这种写法既兼容早期原生客户端，也让小型
        // 宿主集成测试更容易调这个工具。
        if args.get("blockId").is_some() {
            let mut edit = Map::new();
            for key in ["blockId", "originalHash", "oldText", "newText", "summary"] {
                if let Some(value) = args.get(key) {
                    edit.insert(key.to_owned(), value.clone());
                }
            }
            return Ok(vec![Value::Object(edit)]);
        }

        Err("缺少必填参数 edits".into())
    }

    /// 宿主适配器可能直接返回块，也可能包在 `block` 或 `data` 里。
    /// 校验乐观并发字段之前，先把这些无害的信封差异抹平。
    fn block_value(value: &Value) -> &Value {
        if value.get("blockId").is_some() || value.get("id").is_some() {
            return value;
        }
        if let Some(block) = value.get("block") {
            return Self::block_value(block);
        }
        if let Some(data) = value.get("data") {
            return Self::block_value(data);
        }
        value
    }

    fn block_id(value: &Value) -> Option<&str> {
        Self::block_value(value)
            .get("blockId")
            .or_else(|| Self::block_value(value).get("id"))
            .and_then(Value::as_str)
    }

    fn block_text(value: &Value) -> Option<&str> {
        let block = Self::block_value(value);
        block
            .get("text")
            .or_else(|| block.get("content"))
            .or_else(|| block.get("oldText"))
            .and_then(Value::as_str)
    }

    fn block_hash(value: &Value) -> Option<&str> {
        let block = Self::block_value(value);
        block
            .get("originalHash")
            .or_else(|| block.get("contentHash"))
            .or_else(|| block.get("hash"))
            .and_then(Value::as_str)
    }

    fn needs_conversion(value: &Value) -> bool {
        let block = Self::block_value(value);
        block
            .get("needsConversion")
            .and_then(Value::as_bool)
            .or_else(|| value.get("needsConversion").and_then(Value::as_bool))
            .unwrap_or(false)
            || block
                .get("hasPersistedIds")
                .and_then(Value::as_bool)
                .is_some_and(|persisted| !persisted)
            || value
                .get("hasPersistedIds")
                .and_then(Value::as_bool)
                .is_some_and(|persisted| !persisted)
    }

    fn validate_block_snapshot(
        &self,
        path: &str,
        block_id: &str,
        old_text: &str,
        original_hash: &str,
    ) -> Result<(), String> {
        let snapshot = self
            .host
            .read_document_block(path, block_id)
            .map_err(|e| format!("读取块失败: {e}"))?;
        if Self::needs_conversion(&snapshot) {
            return Err("该文档尚未持久化块 ID，请先在界面中转换文档".into());
        }
        let actual_id = Self::block_id(&snapshot).ok_or("块读取结果缺少 blockId")?;
        if actual_id != block_id {
            return Err("块 ID 与读取结果不一致，拒绝生成提案".into());
        }
        let actual_text = Self::block_text(&snapshot).ok_or("块读取结果缺少文本")?;
        if actual_text != old_text {
            return Err("块原文已变化，请重新读取后生成提案".into());
        }
        let actual_hash = Self::block_hash(&snapshot).ok_or("块读取结果缺少 originalHash")?;
        if actual_hash != original_hash {
            return Err("块原文哈希已变化，请重新读取后生成提案".into());
        }
        Ok(())
    }

    fn normalize_pending(edit: &BlockEditProposal, response: Value) -> Value {
        let edit_value = serde_json::to_value(edit).unwrap_or_else(|_| json!({}));
        let mut response = match response {
            Value::Object(object) => object,
            other => {
                let mut object = Map::new();
                object.insert("result".into(), other);
                object
            }
        };
        response
            .entry("status")
            .or_insert_with(|| Value::String("pending".into()));
        response
            .entry("pendingBlockEdit")
            .or_insert_with(|| edit_value.clone());
        response.entry("pendingFileOperation").or_insert_with(|| {
            json!({
                "kind": "block-edit",
                "path": edit.path,
                "blockId": edit.block_id,
                "originalHash": edit.original_hash,
                "oldText": edit.old_text,
                "newText": edit.new_text,
                "summary": edit.summary,
                "status": "pending"
            })
        });
        Value::Object(response)
    }

    fn call_list(&self, args: &ToolArgs) -> ToolOutcome {
        let path = self.resolve(args)?;
        let result = self
            .host
            .list_document_blocks(&path)
            .map_err(|e| format!("读取文档块失败: {e}"))?;
        // 宿主的转换状态和记录原样透传，但把 path 显式带上，
        // 模型之后提案时才能可靠地对应上文档。
        let mut result = match result {
            Value::Object(object) => object,
            other => {
                let mut object = Map::new();
                object.insert("blocks".into(), other);
                object
            }
        };
        result.entry("path").or_insert_with(|| path.into());
        Ok(Value::Object(result))
    }

    fn call_read(&self, args: &ToolArgs) -> ToolOutcome {
        let path = self.resolve(args)?;
        let block_id = args.str_required("blockId")?;
        let result = self
            .host
            .read_document_block(&path, block_id)
            .map_err(|e| format!("读取块失败: {e}"))?;
        // 读取未标注文档属于「要不要转换」的 UI 决策。读取调用
        // 以带内方式上报；绝不悄悄改写缓冲。
        if Self::needs_conversion(&result) && args.bool_opt("requestConversion").unwrap_or(false) {
            return self
                .host
                .request_document_block_conversion(&path)
                .map_err(|e| format!("请求块 ID 转换失败: {e}"));
        }
        Ok(result)
    }

    fn call_propose(&self, args: &ToolArgs) -> ToolOutcome {
        let path = self.resolve(args)?;
        let raw_edits = Self::edit_values(args)?;
        let mut seen = HashSet::new();
        let mut edits = Vec::with_capacity(raw_edits.len());

        // 任何一条入队之前，先把所有块预检一遍。否则可能出现混合结果：
        // 第一条编辑已入队，后面一条过期的编辑又让整个请求失败。
        // 校验通过后，每次对宿主的调用仍可独立审批，各自有自己的
        // InboxEntry。
        for raw in raw_edits {
            let block_id = Self::required_text(raw.get("blockId"), "blockId")?;
            if !seen.insert(block_id.clone()) {
                return Err(format!("同一块不能重复提出修改: {block_id}"));
            }
            let original_hash = Self::required_text(raw.get("originalHash"), "originalHash")?;
            if original_hash.is_empty() {
                return Err("参数 originalHash 不能为空".into());
            }
            let old_text = Self::required_text(raw.get("oldText"), "oldText")?;
            let new_text = Self::required_text(raw.get("newText"), "newText")?;
            let summary = raw
                .get("summary")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .unwrap_or("块级修改建议")
                .to_owned();
            self.validate_block_snapshot(&path, &block_id, &old_text, &original_hash)?;
            edits.push(BlockEditProposal {
                block_id,
                original_hash,
                old_text,
                new_text,
                path: path.clone(),
                summary,
            });
        }

        let mut responses = Vec::with_capacity(edits.len());
        let mut pending_blocks = Vec::with_capacity(edits.len());
        let mut pending_operations = Vec::with_capacity(edits.len());
        for edit in &edits {
            let response = self
                .host
                .propose_block_edit(edit.clone())
                .map_err(|e| format!("提交块修改提案失败: {e}"))?;
            let response = Self::normalize_pending(edit, response);
            if let Some(value) = response.get("pendingBlockEdit") {
                pending_blocks.push(value.clone());
            }
            if let Some(value) = response.get("pendingFileOperation") {
                pending_operations.push(value.clone());
            }
            responses.push(response);
        }

        let mut result = json!({
            "status": "pending",
            "count": pending_blocks.len(),
            "pendingBlockEdits": pending_blocks,
            "pendingFileOperations": pending_operations,
            "results": responses
        });
        if edits.len() == 1 {
            result["pendingBlockEdit"] = result["pendingBlockEdits"][0].clone();
            result["pendingFileOperation"] = result["pendingFileOperations"][0].clone();
        }
        Ok(result)
    }
}

impl ToolExecutor for BlockToolExecutor {
    fn handles(&self, name: &str) -> bool {
        matches!(name, LIST_TOOL | READ_TOOL | PROPOSE_TOOL)
    }

    fn required_actions(&self, name: &str, args: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        let path = self.resolve(args).ok();
        match name {
            LIST_TOOL | READ_TOOL => vec![(AiToolAction::ReadFile, path)],
            PROPOSE_TOOL => vec![(AiToolAction::WriteFile, path)],
            _ => Vec::new(),
        }
    }

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        match name {
            LIST_TOOL => self.call_list(args),
            READ_TOOL => self.call_read(args),
            PROPOSE_TOOL => self.call_propose(args),
            _ => Err(format!("未知的文档块工具: {name}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::{AiToolCall, AiToolFunction};
    use crate::ai::permission::{AiPermissionLevel, AiPermissionService};
    use crate::ai::tools::host::ActiveDocument;
    use crate::ai::tools::ToolRegistry;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    struct Host {
        root: PathBuf,
        block: Mutex<Value>,
        proposals: Mutex<Vec<BlockEditProposal>>,
        dirty: String,
    }

    impl ToolHost for Host {
        fn workspace_root(&self) -> &Path {
            &self.root
        }
        fn active_document(&self) -> Option<ActiveDocument> {
            Some(ActiveDocument {
                id: "tab".into(),
                title: "note.md".into(),
                path: self.root.join("note.md").to_string_lossy().into_owned(),
                is_dirty: true,
            })
        }
        fn read_document_text(&self, _path: &str) -> Result<String, String> {
            Ok(self.dirty.clone())
        }
        fn list_document_blocks(&self, path: &str) -> Result<Value, String> {
            Ok(json!({
                "path": path,
                "documentId": "doc-1",
                "hasPersistedIds": true,
                "blocks": [self.block.lock().unwrap().clone()]
            }))
        }
        fn read_document_block(&self, _path: &str, _id: &str) -> Result<Value, String> {
            Ok(self.block.lock().unwrap().clone())
        }
        fn propose_block_edit(&self, edit: BlockEditProposal) -> Result<Value, String> {
            self.proposals.lock().unwrap().push(edit.clone());
            Ok(json!({"status":"pending", "pendingBlockEdit":edit}))
        }
    }

    struct Fixture {
        root: PathBuf,
        host: Arc<Host>,
        registry: ToolRegistry,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn fixture() -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "mochi-block-tools-{}-{}",
            std::process::id(),
            crate::paths::random_base36(8)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let host = Arc::new(Host {
            root: root.clone(),
            block: Mutex::new(json!({
                "blockId":"block_00000000-0000-4000-8000-000000000001",
                "text":"脏缓冲区原文",
                "originalHash":"hash-1",
                "persisted":true
            })),
            proposals: Mutex::new(Vec::new()),
            dirty: "脏缓冲区原文".into(),
        });
        let registry = ToolRegistry::new(Arc::new(AiPermissionService::new(&root)))
            .with(Arc::new(BlockToolExecutor::new(host.clone())));
        Fixture {
            root,
            host,
            registry,
        }
    }

    fn call(name: &str, args: Value) -> AiToolCall {
        AiToolCall {
            id: "call".into(),
            kind: "function".into(),
            function: AiToolFunction {
                name: name.into(),
                arguments: args.to_string(),
            },
        }
    }

    #[test]
    fn list_and_read_are_available_through_the_registry() {
        let f = fixture();
        let listed: Value = serde_json::from_str(
            &f.registry
                .execute(&call(LIST_TOOL, json!({"path":"note.md"}))),
        )
        .unwrap();
        assert_eq!(listed["ok"], true, "{listed}");
        assert_eq!(
            listed["data"]["blocks"][0]["blockId"],
            "block_00000000-0000-4000-8000-000000000001"
        );

        let read: Value = serde_json::from_str(&f.registry.execute(&call(
            READ_TOOL,
            json!({"path":"note.md","blockId":"block_00000000-0000-4000-8000-000000000001"}),
        )))
        .unwrap();
        assert_eq!(read["ok"], true, "{read}");
        assert_eq!(read["data"]["text"], "脏缓冲区原文");
    }

    #[test]
    fn proposal_contains_required_fields_and_never_writes() {
        let f = fixture();
        let out: Value = serde_json::from_str(&f.registry.execute(&call(
            PROPOSE_TOOL,
            json!({
                "path":"note.md",
                "edits":[{
                    "blockId":"block_00000000-0000-4000-8000-000000000001",
                    "originalHash":"hash-1",
                    "oldText":"脏缓冲区原文",
                    "newText":"建议后的块",
                    "summary":"润色"
                }]
            }),
        )))
        .unwrap();
        assert_eq!(out["ok"], true, "{out}");
        let edit = &out["data"]["pendingBlockEdit"];
        for key in ["blockId", "originalHash", "oldText", "newText", "path"] {
            assert!(edit.get(key).is_some(), "missing {key}: {out}");
        }
        let expected_path = f.root.join("note.md").to_string_lossy().replace('\\', "/");
        assert_eq!(edit["path"], expected_path);
        assert_eq!(f.host.proposals.lock().unwrap().len(), 1);
        assert!(!f.root.join("note.md").exists());
    }

    #[test]
    fn stale_hash_is_rejected_before_any_inbox_proposal() {
        let f = fixture();
        let out: Value = serde_json::from_str(&f.registry.execute(&call(
            PROPOSE_TOOL,
            json!({
                "path":"note.md",
                "edits":[{
                    "blockId":"block_00000000-0000-4000-8000-000000000001",
                    "originalHash":"stale",
                    "oldText":"脏缓冲区原文",
                    "newText":"不应排队"
                }]
            }),
        )))
        .unwrap();
        assert_eq!(out["ok"], false, "{out}");
        assert!(out["error"].as_str().unwrap().contains("哈希"));
        assert!(f.host.proposals.lock().unwrap().is_empty());
    }

    #[test]
    fn read_only_permission_blocks_proposals_but_allows_reads() {
        let f = fixture();
        let target = f.root.to_string_lossy().replace('\\', "/");
        f.registry
            .permissions()
            .set_folder_permission(&target, AiPermissionLevel::ReadOnly)
            .unwrap();
        let out: Value = serde_json::from_str(&f.registry.execute(&call(
            PROPOSE_TOOL,
            json!({
                "path":"note.md",
                "edits":[{
                    "blockId":"block_00000000-0000-4000-8000-000000000001",
                    "originalHash":"hash-1",
                    "oldText":"脏缓冲区原文",
                    "newText":"x"
                }]
            }),
        )))
        .unwrap();
        assert_eq!(out["ok"], false, "{out}");
        assert!(out["error"].as_str().unwrap().contains("只读"));
        assert!(f.host.proposals.lock().unwrap().is_empty());
    }
}
