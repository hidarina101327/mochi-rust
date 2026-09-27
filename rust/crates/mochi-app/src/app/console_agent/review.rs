//! 执行控制台审阅任务，并管理审阅结果的应用流程。
use super::*;
impl App {
    pub(super) fn console_update_job_receipts(&mut self, id: &str, result: &Value) {
        if result["status"] == "running" {
            return;
        }
        let mut changed = false;
        if let Some(c) = self.ai.panel.active.as_mut() {
            for m in &mut c.messages {
                if let Some(raw) = m
                    .get("pendingConsoleAction")
                    .filter(|v| v["result"]["jobId"] == id)
                {
                    let mut updated = raw.clone();
                    updated["status"] = json!(if result["status"] == "completed" {
                        "applied"
                    } else {
                        "error"
                    });
                    updated["result"] = result.clone();
                    if updated != *raw {
                        m.set("pendingConsoleAction", updated);
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.ai_persist_active();
        }
    }
    pub(in crate::app) fn console_is_reviewing(&self) -> bool {
        self.console_jobs.reviewing
    }
    pub(super) fn console_dispatch(
        &mut self,
        hwnd: HWND,
        name: &str,
        args: &Value,
    ) -> CResult<Value> {
        ensure!(args.is_object(), "参数必须是对象");
        ensure!(
            args.get("data").is_none_or(Value::is_object),
            "data 必须是对象"
        );
        ensure!(
            args.get("operations").is_none_or(Value::is_array),
            "operations 必须是数组"
        );
        let mut args = args.clone();
        if matches!(
            name,
            "base_manage"
                | "canvas_manage"
                | "comments_manage"
                | "annotations_manage"
                | "exam_manage"
        ) && args.get("path").is_none()
        {
            args["path"] = json!(self.active_file_path().context("缺少目标 path")?);
        }
        match self.console_execute(hwnd, name, &args) {
            Err(e) if e.is::<ApprovalRequired>() => {
                let spec = mochi_core::ai::tools::console_tools::SPECS
                    .iter()
                    .find(|s| s.0 == name)
                    .context("未知工具")?;
                let proposal = json!({"id":mochi_core::paths::random_base36(16),"status":"pending","workspaceRoot":self.console_root()?,"toolName":name,"args":args,"summary":format!("{} · {}",spec.1,args["action"].as_str().unwrap_or("get"))});
                Ok(
                    json!({"applied":false,"pendingConsoleAction":proposal,"note":"尚未执行；用户可在会话审批卡查看完整参数并批准。不要重复提交同一操作。"}),
                )
            }
            r => r,
        }
    }
    pub(in crate::app) fn open_console_review(&mut self, raw: Value) -> bool {
        if raw["status"] != "pending" {
            return false;
        }
        self.dialog = Some(Dialog {
            title: "确认控制台操作".into(),
            description: raw["summary"].as_str().unwrap_or("控制台变更").into(),
            field: None,
            error: String::new(),
            note: Some("完整操作参数在会话正文中。批准时会重新核对权限、版本和未保存内容。".into()),
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "拒绝".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::ReviewConsoleAction {
                        raw: raw.clone(),
                        approve: false,
                    },
                },
                DialogButton {
                    label: "批准执行".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::ReviewConsoleAction { raw, approve: true },
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
        true
    }
    fn execute_console_review(&mut self, raw: &Value, approve: bool) -> CResult<Value> {
        let root = self.console_root()?;
        ensure!(
            raw["workspaceRoot"]
                .as_str()
                .is_some_and(|s| mochi_core::paths::paths_equal(Path::new(s), &root)),
            "请回到生成提案的工作区"
        );
        let current = self
            .ai
            .panel
            .active
            .as_ref()
            .and_then(|c| {
                c.messages.iter().find_map(|m| {
                    m.get("pendingConsoleAction")
                        .filter(|v| v["id"] == raw["id"])
                        .cloned()
                })
            })
            .context("原提案已不存在")?;
        ensure!(
            current["status"] == "pending" && current == *raw,
            "提案已处理或发生变化，请重新查看"
        );
        if !approve {
            return Ok(json!({"status":"rejected"}));
        }
        let name = text(raw, "toolName")?;
        self.console_jobs.reviewing = true;
        let result = self.console_execute(HWND(self.hwnd_raw as *mut _), name, &raw["args"]);
        self.console_jobs.reviewing = false;
        let value = result?;
        Ok(json!({"status":if value["status"]=="running"{"running"}else{"applied"},"result":value}))
    }
    pub(in crate::app) fn apply_console_review(&mut self, raw: Value, approve: bool) {
        match self.execute_console_review(&raw, approve) {
            Ok(receipt) => {
                if let Some(c) = self.ai.panel.active.as_mut() {
                    for m in &mut c.messages {
                        if m.get("pendingConsoleAction")
                            .is_some_and(|v| v["id"] == raw["id"])
                        {
                            let mut updated = raw.clone();
                            updated["status"] = receipt["status"].clone();
                            updated["result"] = receipt["result"].clone();
                            m.set("pendingConsoleAction", updated);
                        }
                    }
                }
                self.ai_persist_active();
                self.close_dialog();
            }
            Err(e) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.error = format!("{e:#}");
                }
                self.ai.panel.error = format!("控制台操作未完成：{e:#}");
            }
        }
    }
}
