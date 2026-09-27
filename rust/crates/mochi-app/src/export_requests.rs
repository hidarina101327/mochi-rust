//! 原生专用导出审批队列，保存在原生设置目录，不扩展 Electron 的共享提案格式。
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Mutex,
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub session_id: Option<String>,
    pub id: String,
    pub source: String,
    pub output: String,
    pub format: String,
    pub content: Option<String>,
    pub created_at: i64,
}
impl Request {
    pub fn as_inbox(&self) -> mochi_core::ai::agent_inbox::InboxEntry {
        if self.format == "shell" {
            let data = self.shell_data();
            let script = data.get("script").is_some() || data["kind"] == "script";
            return mochi_core::ai::agent_inbox::InboxEntry::from_value(&serde_json::json!({"id":self.id,"createdAt":self.created_at,"taskName":if script {"脚本审批"} else {"命令审批"},"operation":{"kind":"native-shell-command","path":self.output,"title":if self.state=="interrupted"{"上次执行结果未知"}else if script {"脚本需要批准"} else {"命令需要批准"},"summary":if script {data["summary"].as_str().unwrap_or("查看完整脚本后批准")} else {&self.source},"status":"pending"}})).unwrap();
        }
        mochi_core::ai::agent_inbox::InboxEntry::from_value(&serde_json::json!({"id":self.id,"createdAt":self.created_at,"taskId":null,"taskName":"文档导出","operation":{"kind":"native-document-export","path":self.output,"title":format!("导出 {}",self.format.to_uppercase()),"summary":format!("{} → {}",self.source,self.output),"status":"pending"}})).unwrap()
    }
}
impl Request {
    pub fn shell_data(&self) -> serde_json::Value {
        let mut value = self
            .content
            .as_ref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .filter(|v| v.is_object())
            .unwrap_or_else(|| serde_json::json!({}));
        value["id"] = self.id.clone().into();
        value["command"] = self.source.clone().into();
        value["cwd"] = self.output.clone().into();
        value["status"] = if matches!(self.state.as_str(), "applied" | "error") {
            self.state.clone()
        } else {
            "pending".into()
        }
        .into();
        value
    }
    fn is_pending(&self) -> bool {
        !matches!(self.state.as_str(), "applied" | "error" | "running")
    }
}
struct State {
    items: Vec<Request>,
    busy: HashSet<String>,
}
pub struct Queue {
    path: PathBuf,
    state: Mutex<State>,
}
impl Queue {
    pub fn new(profile: &Path, root: &Path) -> Result<Self> {
        let root = root.to_string_lossy().replace('\\', "/").to_lowercase();
        let hash = root.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        });
        let path = profile.join(format!("export-requests-{hash:016x}.json"));
        let mut items: Vec<Request> = if path.is_file() {
            serde_json::from_str(&std::fs::read_to_string(&path)?)?
        } else {
            Vec::new()
        };
        for item in &mut items {
            if item.state == "running" {
                item.state = "interrupted".into();
            }
        }
        Ok(Self {
            path,
            state: Mutex::new(State {
                items,
                busy: HashSet::new(),
            }),
        })
    }
    fn write(&self, items: &[Request]) -> Result<()> {
        mochi_core::files::FileService::new()
            .write_file_safe(&self.path, &serde_json::to_string_pretty(items)?)
    }
    pub fn add(
        &self,
        source: &str,
        format: &str,
        output: &str,
        content: Option<String>,
    ) -> Result<Request> {
        self.add_for_session(source, format, output, content, None)
    }
    pub fn add_for_session(
        &self,
        source: &str,
        format: &str,
        output: &str,
        content: Option<String>,
        session_id: Option<String>,
    ) -> Result<Request> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let size = state
            .items
            .iter()
            .filter_map(|r| r.content.as_ref())
            .map(String::len)
            .sum::<usize>()
            + content.as_ref().map(String::len).unwrap_or(0);
        if state.items.len() >= 32 || size > 16 * 1024 * 1024 {
            bail!("待批准导出过多，请先处理现有请求")
        }
        let request = Request {
            state: "pending".into(),
            session_id,
            id: format!(
                "native-export-{}-{}",
                mochi_core::jstime::now_millis(),
                mochi_core::paths::random_base36(12)
            ),
            source: source.into(),
            format: format.into(),
            output: output.into(),
            content,
            created_at: mochi_core::jstime::now_millis(),
        };
        let mut next = state.items.clone();
        next.push(request.clone());
        self.write(&next)?;
        state.items = next;
        Ok(request)
    }
    pub fn pending(&self) -> Vec<Request> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .items
            .iter()
            .filter(|r| !state.busy.contains(&r.id) && r.is_pending())
            .cloned()
            .collect()
    }
    pub fn claim(&self, id: &str) -> Result<Request> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.busy.contains(id) {
            bail!("该导出正在执行")
        };
        let item = state
            .items
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("导出请求已不存在"))?;
        if !item.is_pending() {
            bail!("请求已经处理")
        }
        if item.format == "shell" {
            let mut next = state.items.clone();
            if let Some(r) = next.iter_mut().find(|r| r.id == id) {
                r.state = "running".into();
            }
            self.write(&next)?;
            state.items = next;
        }
        state.busy.insert(id.into());
        Ok(item)
    }
    pub fn release(&self, id: &str) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .busy
            .remove(id);
    }
    pub fn complete_shell(
        &self,
        id: &str,
        result: &mochi_core::shell::ShellRunResult,
    ) -> Result<Request> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = state.items.clone();
        let request = next
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| anyhow::anyhow!("命令记录不存在"))?;
        request.state = if result.ok { "applied" } else { "error" }.into();
        let mut data = request.shell_data();
        data["result"] = serde_json::to_value(result)?;
        if let Some(error) = &result.error {
            data["error"] = error.clone().into();
        }
        request.content = Some(serde_json::to_string(&data)?);
        let complete = request.clone();
        self.write(&next)?;
        state.items = next;
        state.busy.remove(id);
        Ok(complete)
    }
    pub fn completed_shells(&self) -> Vec<Request> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .items
            .iter()
            .filter(|r| r.format == "shell" && matches!(r.state.as_str(), "applied" | "error"))
            .cloned()
            .collect()
    }
    pub fn remove(&self, id: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let next = state
            .items
            .iter()
            .filter(|r| r.id != id)
            .cloned()
            .collect::<Vec<_>>();
        self.write(&next)?;
        state.items = next;
        state.busy.remove(id);
        Ok(())
    }

    /// 只有请求仍显示为待处理时才允许拒绝。这样可避免
    /// 旧的审核对话框删除其他操作已接手或
    /// 已经完成的请求。
    pub fn reject_pending(&self, id: &str) -> Result<Request> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.busy.contains(id) {
            bail!("该请求正在执行，不能再拒绝")
        }
        let position = state
            .items
            .iter()
            .position(|request| request.id == id)
            .ok_or_else(|| anyhow::anyhow!("请求已处理或不存在"))?;
        if !state.items[position].is_pending() {
            bail!("请求已经处理")
        }
        let request = state.items[position].clone();
        let mut next = state.items.clone();
        next.remove(position);
        self.write(&next)?;
        state.items = next;
        Ok(request)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_shell_requires_an_explicit_retry() {
        let root = std::env::temp_dir().join(format!(
            "mochi-command-recovery-{}",
            mochi_core::paths::random_base36(12)
        ));
        let queue = Queue::new(&root, &root.join("ws")).unwrap();
        let request = queue
            .add_for_session(
                "echo test",
                "shell",
                ".",
                Some("{}".into()),
                Some("session".into()),
            )
            .unwrap();
        queue.claim(&request.id).unwrap();
        drop(queue);
        let queue = Queue::new(&root, &root.join("ws")).unwrap();
        assert_eq!(queue.pending()[0].state, "interrupted");
        queue.claim(&request.id).unwrap();
        queue
            .complete_shell(
                &request.id,
                &mochi_core::shell::ShellRunResult {
                    ok: true,
                    exit_code: Some(0),
                    stdout: "done".into(),
                    stderr: String::new(),
                    timed_out: false,
                    error: None,
                },
            )
            .unwrap();
        assert!(queue.pending().is_empty());
        assert_eq!(queue.completed_shells().len(), 1);
        drop(queue);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn queue_survives_restart_without_writing_the_shared_workspace() {
        let root = std::env::temp_dir().join(format!(
            "mochi-export-queue-{}-{}",
            std::process::id(),
            mochi_core::paths::random_base36(12)
        ));
        let profile = root.join("native-profile");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let queue = Queue::new(&profile, &workspace).unwrap();
        let item = queue
            .add(
                "note.md",
                "pdf",
                "Export/note.pdf",
                Some("未保存内容".into()),
            )
            .unwrap();
        assert_eq!(item.as_inbox().kind(), "native-document-export");
        queue.claim(&item.id).unwrap();
        assert!(queue.claim(&item.id).is_err());
        assert!(queue.pending().is_empty());
        drop(queue);
        let queue = Queue::new(&profile, &workspace).unwrap();
        assert_eq!(queue.pending()[0].content.as_deref(), Some("未保存内容"));
        queue.remove(&item.id).unwrap();
        assert!(queue.pending().is_empty());
        assert_eq!(std::fs::read_dir(&workspace).unwrap().count(), 0);
        drop(queue);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejecting_requires_a_pending_unclaimed_request() {
        let root = std::env::temp_dir().join(format!(
            "mochi-export-reject-{}",
            mochi_core::paths::random_base36(12)
        ));
        let queue = Queue::new(&root.join("profile"), &root.join("workspace")).unwrap();
        let request = queue
            .add("note.md", "html", "note.html", Some("text".into()))
            .unwrap();
        queue.claim(&request.id).unwrap();
        assert!(queue.reject_pending(&request.id).is_err());
        queue.release(&request.id);
        assert_eq!(queue.reject_pending(&request.id).unwrap().id, request.id);
        assert!(queue.reject_pending(&request.id).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
