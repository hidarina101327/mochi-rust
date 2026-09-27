//! Agent 画布工具只执行经过校验的 `.mcanvas` 修改。

use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use super::host::{resolve_workspace_path, FileChange, ResolveOptions, ToolHost};
use super::{ToolArgs, ToolExecutor, ToolOutcome};
use crate::ai::permission::AiToolAction;

const GET: &str = "canvas_get";
const ADD: &str = "canvas_add_cards";
const MOVE: &str = "canvas_move_card";
const DRAW: &str = "canvas_draw";

#[cfg(test)]
mod tests;

pub struct CanvasToolExecutor {
    host: Arc<dyn ToolHost>,
}
impl CanvasToolExecutor {
    pub fn new(host: Arc<dyn ToolHost>) -> Self {
        Self { host }
    }
    fn path(&self, args: &ToolArgs) -> Result<String, String> {
        let active = self.host.active_document();
        let path = args
            .str_opt("path")
            .or_else(|| active.as_ref().map(|d| d.path.as_str()))
            .ok_or("请先打开一个 .mcanvas 画布，或提供 path")?;
        let resolved = resolve_workspace_path(
            self.host.as_ref(),
            Some(path),
            ResolveOptions {
                fallback_to_selection: false,
                allow_mochi_dir: false,
            },
        )?;
        if !Path::new(&resolved)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(crate::canvas::EXTENSION))
        {
            return Err("画布路径必须以 .mcanvas 结尾".into());
        }
        Ok(resolved)
    }
}
impl ToolExecutor for CanvasToolExecutor {
    fn handles(&self, name: &str) -> bool {
        matches!(name, GET | ADD | MOVE | DRAW)
    }
    fn required_actions(&self, name: &str, args: &ToolArgs) -> Vec<(AiToolAction, Option<String>)> {
        let action = if name == GET {
            AiToolAction::ReadFile
        } else {
            AiToolAction::WriteFile
        };
        vec![(action, self.path(args).ok())]
    }
    fn call(&self, name: &str, args: &ToolArgs) -> ToolOutcome {
        let path = self.path(args)?;
        // 画布工具是直接编辑，既有的 add/move 工具也是如此。
        // 文档提案偏好并不是画布的权限规则。
        if name != GET && self.host.canvas_requires_proposal(&path) {
            return Err(
                "画布处于 AI「建议」权限；请允许 AI 修改该画布，或在 AI 设置中开启自动应用后重试"
                    .into(),
            );
        }
        self.host.canvas_action(&path, name, args.value())
    }
}

pub fn snapshot(path: &str, canvas: &crate::canvas::CanvasDocument) -> Value {
    json!({"path":path,"version":canvas.version,"revision":crate::canvas::drawing::revision(canvas),"viewport":canvas.viewport,"cards":canvas.cards,"texts":canvas.texts,"strokes":canvas.strokes,
        "suggestedBounds":{"x":canvas.viewport.x+40.0,"y":canvas.viewport.y+40.0,"width":480.0,"height":360.0}})
}

/// UI 事务与无头宿主共用的纯编辑逻辑。
pub fn edit(
    canvas: &crate::canvas::CanvasDocument,
    name: &str,
    args: &Value,
) -> Result<(crate::canvas::CanvasDocument, Value), String> {
    let args = ToolArgs::from_value(args.clone());
    let mut canvas = canvas.clone();
    let result = match name {
        "canvas_manage" => {
            canvas = super::structured_manage::canvas_batch(&canvas, args.value())
                .map_err(|e| e.to_string())?;
            json!({"applied":true})
        }
        ADD => {
            let references = args.str_array("references");
            if references.is_empty() {
                return Err("references 至少需要一个 Mochi 对象 URL".into());
            }
            let count = references.len();
            for reference in references {
                canvas
                    .add_mochi_reference(reference)
                    .map_err(|e| format!("不能加入画布: {e}"))?;
            }
            json!({"added":count,"cards":canvas.cards})
        }
        MOVE => {
            let id = args.str_required("cardId")?;
            let x = args.get("x").and_then(Value::as_f64).ok_or("缺少数值 x")?;
            let y = args.get("y").and_then(Value::as_f64).ok_or("缺少数值 y")?;
            let card = canvas
                .cards
                .iter_mut()
                .find(|card| card.id == id)
                .ok_or("未找到画布卡片")?;
            card.x = x;
            card.y = y;
            json!({"cardId":id,"x":x,"y":y})
        }
        DRAW => {
            let drawing = args.get("drawing").ok_or("缺少 drawing")?;
            if drawing.to_string().len() > 1_048_576 {
                return Err("单次绘图不能超过 1 MiB".into());
            }
            let drawing = serde_json::from_value(drawing.clone())
                .map_err(|e| format!("绘图参数无效：{e}"))?;
            return crate::canvas::drawing::apply(
                &canvas,
                args.str_required("expectedRevision")?,
                drawing,
            )
            .map_err(|e| e.to_string());
        }
        _ => return Err(format!("未知画布工具: {name}")),
    };
    canvas.validate().map_err(|e| e.to_string())?;
    Ok((canvas, result))
}

/// 没有打开编辑器的宿主也照样整个批次原子校验、原子保存。
pub fn file_action(
    host: &(impl ToolHost + ?Sized),
    path: &str,
    name: &str,
    args: &Value,
) -> ToolOutcome {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("读取画布失败：{e}"))?;
    let current = crate::canvas::parse(&raw).map_err(|e| format!("画布格式无效：{e}"))?;
    if name == GET {
        return Ok(snapshot(path, &current));
    }
    let (next, mut result) = edit(&current, name, args)?;
    let content = crate::canvas::serialize(&next).map_err(|e| e.to_string())?;
    if std::fs::read_to_string(path).map_err(|e| e.to_string())? != raw {
        return Err("画布文件已变化，请重新读取后重试".into());
    }
    crate::files::FileService::new()
        .write_file_safe(Path::new(path), &content)
        .map_err(|e| e.to_string())?;
    host.notify_change(FileChange::Write { path: path.into() });
    result["path"] = json!(path);
    result["undoAvailable"] = json!(false);
    result["revision"] = json!(crate::canvas::drawing::revision(&next));
    Ok(result)
}
