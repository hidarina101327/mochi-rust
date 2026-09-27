//! 调用 `history` 和 `diff` 需要 `read_file`；调用 `restore` 和 `commit` 需要 `write_file`。

use std::sync::Arc;

use serde_json::json;

use super::host::{resolve_workspace_path, ResolveOptions, ToolHost};
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;
use crate::git::GitService;

const TOOL_NAMES: &[&str] = &[
    "git_history",
    "git_file_diff",
    "git_restore_file",
    "git_commit",
];

pub struct GitToolExecutor {
    host: Arc<dyn ToolHost>,
    git: Arc<GitService>,
}

impl GitToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>, git: Arc<GitService>) -> Self {
        Self { host, git }
    }

    /// Git 工具的路径参数是**可选**的（`git_history` 不带 filepath 就是全库历史），
    /// 所以这里用 `fallback_to_selection: false`——不该因为当前选了某个知识库
    /// 就把"全库历史"悄悄变成"该知识库的历史"。
    fn resolve(&self, input: Option<&str>) -> Result<String, String> {
        resolve_workspace_path(
            self.host.as_ref(),
            input,
            ResolveOptions {
                fallback_to_selection: false,
                allow_mochi_dir: false,
            },
        )
    }
}

impl ToolExecutor for GitToolExecutor {
    fn handles(&self, name: &str) -> bool {
        TOOL_NAMES.contains(&name)
    }

    fn required_actions(&self, name: &str, args: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        let path = args
            .str_opt("filepath")
            .and_then(|p| self.resolve(Some(p)).ok());
        match name {
            "git_history" | "git_file_diff" => vec![(AiToolAction::ReadFile, path)],
            "git_restore_file" => vec![(AiToolAction::WriteFile, path)],
            // 提交动的是整个工作区，没有单一路径可查，只查动作开关
            "git_commit" => vec![(AiToolAction::WriteFile, None)],
            _ => Vec::new(),
        }
    }

    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        match name {
            "git_history" => self.history(args),
            "git_file_diff" => self.file_diff(args),
            "git_restore_file" => self.restore_file(args),
            "git_commit" => self.commit(args),
            other => Err(format!("未知的 Git 工具: {other}")),
        }
    }
}

impl GitToolExecutor {
    fn history(&self, args: &ToolArgs) -> ToolOutcome {
        // 对齐 TS：Math.max(1, Math.min(Number(limit) || 20, 200))
        let limit = args
            .i64_opt("limit")
            .filter(|n| *n != 0)
            .unwrap_or(20)
            .clamp(1, 200) as usize;
        let filepath = match args.str_opt("filepath") {
            Some(p) => Some(self.resolve(Some(p))?),
            None => None,
        };
        let commits = self
            .git
            .get_history_filtered(limit, filepath.as_deref(), args.str_opt("author"))
            .map_err(|e| format!("读取历史失败: {e}"))?;

        Ok(json!({
            "count": commits.len(),
            "commits": commits.iter().map(|c| json!({
                "oid": c.oid,
                "message": c.message,
                "type": c.kind,
                "author": c.author_name,
                // TS 给的是秒级时间戳（isomorphic-git 的 committer.timestamp）
                "timestamp": c.timestamp_ms / 1000,
            })).collect::<Vec<_>>(),
        }))
    }

    fn file_diff(&self, args: &ToolArgs) -> ToolOutcome {
        let filepath = self.resolve(Some(args.str_required("filepath")?))?;
        let old_oid = args.str_required("oldOid")?;
        let new_oid = args.str_required("newOid")?;

        let (old_content, new_content) = self
            .git
            .file_diff(&filepath, old_oid, new_oid)
            .map_err(|e| format!("取差异失败: {e}"))?;
        Ok(json!({ "filepath": filepath, "oldContent": old_content, "newContent": new_content }))
    }

    fn restore_file(&self, args: &ToolArgs) -> ToolOutcome {
        let filepath = self.resolve(Some(args.str_required("filepath")?))?;
        let oid = args.str_required("oid")?;
        // 只认 overwrite，其余一律 copy——默认走非破坏性的那一档
        let mode = if args.str_opt("mode") == Some("overwrite") {
            "overwrite"
        } else {
            "copy"
        };

        let restored = self
            .git
            .restore_file(&filepath, oid, mode)
            .map_err(|e| format!("恢复失败: {e}"))?;
        Ok(json!({
            "filepath": filepath,
            "oid": oid,
            "mode": mode,
            "restoredPath": crate::paths::to_forward_slashes(&restored.to_string_lossy()),
        }))
    }

    fn commit(&self, args: &ToolArgs) -> ToolOutcome {
        let message = args.str_required("message")?.trim().to_owned();
        if message.is_empty() {
            return Err("提交说明不能为空".into());
        }
        let oid = self
            .git
            .commit(&message, "manual")
            .map_err(|e| format!("提交失败: {e}"))?;
        Ok(json!({ "oid": oid, "message": message, "committed": oid.is_some() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::models::{AiToolCall, AiToolFunction};
    use crate::ai::permission::{AiPermissionLevel, AiPermissionService};
    use crate::ai::tools::host::HeadlessHost;
    use crate::ai::tools::ToolRegistry;
    use serde_json::Value;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        root: PathBuf,
        git: Arc<GitService>,
        registry: ToolRegistry,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("mochi-gittool-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let git = Arc::new(GitService::new(&root));
        git.ensure_repository().unwrap();
        // GitService 会把根路径 canonicalize，宿主必须用同一份，否则路径前缀对不上
        let canonical = git
            .git_dir_path()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();

        let host: Arc<dyn ToolHost> = Arc::new(HeadlessHost::new(&canonical));
        let perms = Arc::new(AiPermissionService::new(&canonical));
        let registry =
            ToolRegistry::new(perms).with(Arc::new(GitToolExecutor::new(host, git.clone())));
        Fixture {
            root: canonical,
            git,
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
        /// 写文件 + 提交，返回 oid。
        fn commit(&self, rel: &str, content: &str, message: &str) -> String {
            let p = self.write(rel, content);
            self.git.mark_write(&p);
            self.git
                .commit(message, "manual")
                .unwrap()
                .expect("应产生提交")
        }
    }

    #[test]
    fn commit_tool_creates_a_commit() {
        let f = fixture("commit");
        f.write("a.md", "内容");
        let out = f.run("git_commit", json!({ "message": "AI 提交的" }));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["data"]["committed"], true);
        assert!(out["data"]["oid"].is_string());

        let history = f.git.get_history(10).unwrap();
        assert_eq!(history[0].message, "manual: AI 提交的");
    }

    #[test]
    fn commit_tool_rejects_an_empty_message() {
        let f = fixture("commit-empty");
        for args in [json!({}), json!({ "message": "   " })] {
            assert_eq!(f.run("git_commit", args)["ok"], false);
        }
    }

    /// 没有可提交的改动时返回 committed=false，而不是报错。
    #[test]
    fn commit_with_nothing_staged_reports_not_committed() {
        let f = fixture("commit-noop");
        f.commit("a.md", "x", "第一次");
        let out = f.run("git_commit", json!({ "message": "空提交" }));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["data"]["committed"], false);
    }

    #[test]
    fn history_returns_newest_first_with_types() {
        let f = fixture("history");
        f.commit("a.md", "v1", "第一次");
        f.commit("a.md", "v2", "第二次");

        let out = f.run("git_history", json!({ "limit": 10 }));
        let commits = out["data"]["commits"].as_array().unwrap();
        assert_eq!(out["data"]["count"], 2);
        assert_eq!(commits[0]["message"], "manual: 第二次");
        assert_eq!(commits[0]["type"], "manual");
        assert!(
            commits[0]["timestamp"].as_i64().unwrap() > 1_000_000_000,
            "应是秒级时间戳"
        );
    }

    #[test]
    fn history_can_be_filtered_by_file() {
        let f = fixture("history-file");
        f.commit("甲.md", "x", "动甲");
        f.commit("乙.md", "y", "动乙");
        f.commit("甲.md", "xx", "又动甲");

        let out = f.run("git_history", json!({ "filepath": "甲.md" }));
        let commits = out["data"]["commits"].as_array().unwrap();
        assert_eq!(commits.len(), 2, "只该返回动过甲.md 的提交: {out}");
        assert!(commits
            .iter()
            .all(|c| c["message"].as_str().unwrap().contains("甲")));
    }

    #[test]
    fn history_can_be_filtered_by_author() {
        let f = fixture("history-author");
        f.commit("a.md", "x", "提交");
        assert_eq!(
            f.run("git_history", json!({ "author": "Mochi" }))["data"]["count"],
            1
        );
        assert_eq!(
            f.run("git_history", json!({ "author": "别人" }))["data"]["count"],
            0
        );
    }

    #[test]
    fn history_limit_is_clamped() {
        let f = fixture("history-limit");
        for i in 0..5 {
            f.commit("a.md", &format!("v{i}"), &format!("第{i}次"));
        }
        assert_eq!(
            f.run("git_history", json!({ "limit": 2 }))["data"]["count"],
            2
        );
        // 0 / 负数 / 超大值都要被夹到合法区间，不能把 0 当成"不限"
        assert_eq!(
            f.run("git_history", json!({ "limit": 0 }))["data"]["count"],
            5
        );
        assert_eq!(
            f.run("git_history", json!({ "limit": 99999 }))["data"]["count"],
            5
        );
    }

    #[test]
    fn history_on_a_fresh_repo_is_empty() {
        let f = fixture("history-fresh");
        assert_eq!(f.run("git_history", json!({}))["data"]["count"], 0);
    }

    #[test]
    fn file_diff_returns_both_sides() {
        let f = fixture("diff");
        let first = f.commit("a.md", "旧内容", "v1");
        let second = f.commit("a.md", "新内容", "v2");

        let out = f.run(
            "git_file_diff",
            json!({ "filepath": "a.md", "oldOid": first, "newOid": second }),
        );
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["data"]["oldContent"], "旧内容");
        assert_eq!(out["data"]["newContent"], "新内容");
    }

    /// 如果文件只存在于一侧，就返回空字符串而不是报错——差异的一侧本来就可能是“文件尚不存在”。
    #[test]
    fn file_diff_treats_a_missing_side_as_empty() {
        let f = fixture("diff-missing");
        let first = f.commit("a.md", "只有甲", "v1");
        let second = f.commit("b.md", "新增乙", "v2");

        let out = f.run(
            "git_file_diff",
            json!({ "filepath": "b.md", "oldOid": first, "newOid": second }),
        );
        assert_eq!(out["data"]["oldContent"], "", "旧提交里没有 b.md，应给空串");
        assert_eq!(out["data"]["newContent"], "新增乙");
    }

    #[test]
    fn file_diff_requires_both_oids() {
        let f = fixture("diff-args");
        let oid = f.commit("a.md", "x", "v1");
        assert_eq!(
            f.run(
                "git_file_diff",
                json!({ "filepath": "a.md", "oldOid": oid })
            )["ok"],
            false
        );
        assert_eq!(
            f.run("git_file_diff", json!({ "filepath": "a.md" }))["ok"],
            false
        );
    }

    /// copy 模式（默认）写成旁路副本，**绝不动原文件**。
    #[test]
    fn restore_copy_mode_leaves_the_original_alone() {
        let f = fixture("restore-copy");
        let old = f.commit("笔记.md", "第一版", "v1");
        f.commit("笔记.md", "第二版", "v2");

        let out = f.run(
            "git_restore_file",
            json!({ "filepath": "笔记.md", "oid": old }),
        );
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["data"]["mode"], "copy");

        assert_eq!(
            std::fs::read_to_string(f.root.join("笔记.md")).unwrap(),
            "第二版",
            "copy 模式不该覆盖原文件"
        );
        let restored = out["data"]["restoredPath"].as_str().unwrap();
        let file_name = restored.rsplit('/').next().unwrap();
        assert!(file_name.starts_with("笔记-restored-"), "{file_name}");
        // 只查文件名——路径里的盘符 `C:` 本来就带冒号
        assert!(
            !file_name.contains(':'),
            "时间戳里的冒号在 Windows 上非法: {file_name}"
        );
        assert!(file_name.ends_with(".md"), "扩展名应保留: {file_name}");
        assert_eq!(std::fs::read_to_string(restored).unwrap(), "第一版");
    }

    /// overwrite 模式覆盖前必须先自动提交一次快照——恢复本身是破坏性操作。
    #[test]
    fn restore_overwrite_snapshots_before_clobbering() {
        let f = fixture("restore-overwrite");
        let old = f.commit("笔记.md", "第一版", "v1");
        f.write("笔记.md", "未提交的改动");

        let out = f.run(
            "git_restore_file",
            json!({ "filepath": "笔记.md", "oid": old, "mode": "overwrite" }),
        );
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(
            std::fs::read_to_string(f.root.join("笔记.md")).unwrap(),
            "第一版"
        );

        let messages: Vec<String> = f
            .git
            .get_history(10)
            .unwrap()
            .into_iter()
            .map(|c| c.message)
            .collect();
        assert!(
            messages.iter().any(|m| m.contains("恢复前快照")),
            "覆盖前应留下快照提交，否则未提交的改动就永久没了: {messages:?}"
        );
        assert!(messages.iter().any(|m| m.starts_with("restore:")));
    }

    #[test]
    fn restore_requires_an_oid() {
        let f = fixture("restore-args");
        assert_eq!(
            f.run("git_restore_file", json!({ "filepath": "a.md" }))["ok"],
            false
        );
    }

    #[test]
    fn readonly_paths_block_restore_but_not_history() {
        let f = fixture("perm");
        f.commit("参考/a.md", "内容", "v1");
        let readonly = crate::paths::to_forward_slashes(&f.root.join("参考").to_string_lossy());
        f.registry
            .permissions()
            .set_folder_permission(&readonly, AiPermissionLevel::ReadOnly)
            .unwrap();

        assert_eq!(
            f.run("git_history", json!({ "filepath": "参考/a.md" }))["ok"],
            true,
            "只读不该挡住读历史"
        );

        let out = f.run(
            "git_restore_file",
            json!({ "filepath": "参考/a.md", "oid": "deadbeef", "mode": "overwrite" }),
        );
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().contains("只读"), "{out}");
    }

    #[test]
    fn paths_outside_the_workspace_are_rejected() {
        let f = fixture("escape");
        let out = f.run("git_history", json!({ "filepath": "../外面.md" }));
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().contains(".."), "{out}");
    }

    /// 不带 filepath 时是**全库**历史，不该被"当前选中的知识库"缩小范围。
    #[test]
    fn history_without_a_path_covers_the_whole_repo() {
        let f = fixture("history-all");
        f.commit("知识库/甲/a.md", "x", "动甲");
        f.commit("知识库/乙/b.md", "y", "动乙");
        assert_eq!(f.run("git_history", json!({}))["data"]["count"], 2);
    }

    // 沿用 getFileAtCommitOrEmpty：取值失败返回空串，包括无效 oid。
    #[test]
    fn unknown_oid_yields_empty_sides_not_an_error() {
        let f = fixture("bad-oid");
        f.commit("a.md", "x", "v1");
        let out = f.run(
            "git_file_diff",
            json!({ "filepath": "a.md", "oldOid": "不是oid", "newOid": "也不是" }),
        );
        assert_eq!(out["ok"], true, "TS 行为如此: {out}");
        assert_eq!(out["data"]["oldContent"], "");
        assert_eq!(out["data"]["newContent"], "");
    }
}
