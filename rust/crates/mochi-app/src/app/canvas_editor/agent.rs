//! 构造画布 AI 请求，并应用 AI 返回的画布编辑操作。
use super::*;
use mochi_core::ai::tools::{canvas_tools, host::ToolHost};

impl App {
    pub(in crate::app) fn canvas_agent_requests(&mut self) {
        let Some(request) = self
            .ai
            .host
            .as_ref()
            .and_then(|host| host.take_canvas_request())
        else {
            return;
        };
        let result = if self
            .ai
            .run
            .as_ref()
            .is_some_and(|run| run.cancel.load(std::sync::atomic::Ordering::Relaxed))
        {
            Err("AI 绘图已停止，尚未应用".into())
        } else if std::time::Instant::now() > request.deadline {
            Err("画布请求已过期，请重试".into())
        } else {
            self.canvas_agent_action(&request.name, &request.args)
        };
        let _ = request.reply.send(result);
    }

    pub(in crate::app) fn canvas_agent_action(
        &mut self,
        name: &str,
        args: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let host = self.ai.host.clone().ok_or("AI 工作区未连接")?;
        let path = args
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or("缺少画布路径")?;
        // 在 UI 线程上重新检查：排队的请求必须遵守当前的路径和权限状态。
        let path = mochi_core::ai::tools::host::resolve_workspace_path(
            host.as_ref(),
            Some(path),
            Default::default(),
        )?;
        if !Path::new(&path)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mcanvas"))
        {
            return Err("画布路径必须以 .mcanvas 结尾".into());
        }
        let write = name != "canvas_get";
        let permissions = self.ai.permissions.as_ref().ok_or("AI 权限不可用")?;
        permissions
            .assert_tool_action_allowed(
                if write {
                    mochi_core::ai::permission::AiToolAction::WriteFile
                } else {
                    mochi_core::ai::permission::AiToolAction::ReadFile
                },
                Some(&path),
            )
            .map_err(|e| e.message)?;
        if write && host.canvas_requires_proposal(&path) && !self.console_is_reviewing() {
            return Err("该画布处于 AI「建议」权限，不能直接绘制".into());
        }
        let canonical = Path::new(&path).canonicalize().map_err(|e| e.to_string())?;
        let index = self.shell.tabs().iter().position(|t| {
            t.path()
                .and_then(|p| p.canonicalize().ok())
                .is_some_and(|p| p == canonical)
        });
        let Some(index) = index else {
            // 明确指定的已关闭画布可以直接编辑磁盘文件；已打开的画布始终使用撤销操作。
            return canvas_tools::file_action(host.as_ref(), &path, name, args);
        };
        let state = match &self.shell.tabs()[index].kind {
            TabKind::Viewer {
                content: viewer::Content::Canvas(state),
                ..
            } => state,
            _ => return Err("目标不是自由笔记画布".into()),
        };
        if !state.ai_edit_ready() {
            return Err("画布正在手动编辑，请结束笔迹或文字输入后重试".into());
        }
        if !write {
            let mut result = canvas_tools::snapshot(&path, &state.document);
            let v = &state.document.viewport;
            if self.shell.active_tab() == Some(index) {
                let margin = 24.0 / v.zoom;
                let width = (f64::from(self.viewer_layout.body.width()) / v.zoom - margin * 2.0)
                    .clamp(1.0, 480.0);
                let height = (f64::from(self.viewer_layout.body.height()) / v.zoom - margin * 2.0)
                    .clamp(1.0, 360.0);
                result["suggestedBounds"] = serde_json::json!({"x":v.x+margin,"y":v.y+margin,"width":width,"height":height});
            }
            result["style"] = serde_json::json!({"color":format!("#{:06x}",canvas_view::COLORS[state.color]),"width":canvas_view::WIDTHS[state.width],"fontSize":canvas_view::FONT_SIZES[state.font_size]});
            result["selectedId"] = serde_json::json!(match state.selected {
                Some(canvas_view::Element::Stroke(i)) =>
                    state.document.strokes.get(i).map(|s| s.id.as_str()),
                Some(canvas_view::Element::Text(i)) =>
                    state.document.texts.get(i).map(|s| s.id.as_str()),
                Some(canvas_view::Element::Card(i)) =>
                    state.document.cards.get(i).map(|s| s.id.as_str()),
                None => None,
            });
            return Ok(result);
        }
        let disk = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let disk_doc = mochi_core::canvas::parse(&disk).map_err(|e| e.to_string())?;
        if mochi_core::canvas::drawing::revision(&disk_doc)
            != mochi_core::canvas::drawing::revision(&state.document)
        {
            return Err(
                "画布磁盘内容与编辑器不同，请先保存或重新打开画布，再重新读取后绘制".into(),
            );
        }
        let mut next = state.clone();
        let mut result = next.apply_ai_edit(name, args)?;
        let content = next.serialized().map_err(|e| e.to_string())?;
        if std::fs::read_to_string(&path).map_err(|e| e.to_string())? != disk {
            return Err("画布文件已变化，请重新读取后重试".into());
        }
        mochi_core::files::FileService::new()
            .write_file_safe(Path::new(&path), &content)
            .map_err(|e| format!("保存画布失败：{e}"))?;
        if let TabKind::Viewer {
            content: viewer::Content::Canvas(state),
            ..
        } = &mut self.shell.tabs_mut()[index].kind
        {
            *state = next;
        }
        result["path"] = serde_json::json!(path);
        result["undoAvailable"] = serde_json::json!(true);
        // 不要触发 FileChange::Write：否则此处重新加载会丢弃刚记录的撤销历史。
        self.invalidate_main();
        Ok(result)
    }
}
