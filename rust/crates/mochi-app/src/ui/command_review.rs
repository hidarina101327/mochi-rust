//! 命令审批只读预览；完整命令和工作目录可滚动查看，不允许编辑后执行旧内容。
use super::{
    draw::{DrawList, TextStyle},
    layout::Rect,
    theme::Palette,
    widgets::{FieldLook, TextField},
};
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Copy,
    Cancel,
    Reject,
    Execute,
}

/// 审批界面由 shell 命令、文件提案和原生导出队列共用。类型必须显式区分，
/// 这样应用层才能把审批派发给对应的操作，而不是把所有预览都当成
/// shell 命令处理。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewKind {
    ShellCommand,
    Script,
    FileChange,
    DocumentExport,
}

pub struct State {
    pub request: crate::export_requests::Request,
    pub field: TextField,
    pub error: String,
    pub review_kind: ReviewKind,
    pub file: Option<mochi_core::ai::agent_inbox::InboxEntry>,
    /// 本次预览涵盖的全部持久化收件箱条目。单条目预览继续用原有的
    /// `file` 字段以保持兼容；文档分组用这个向量整行批准或拒绝。
    pub files: Vec<mochi_core::ai::agent_inbox::InboxEntry>,
    pub base_text: Option<String>,
    pub base_disk: Option<Vec<u8>>,
    /// 分组文档写入的预检内容。只有当前缓冲区和磁盘内容仍与 `base_*`
    /// 一致时才会真正写入。
    pub proposed_text: Option<String>,
}
pub struct Layout {
    pub card: Rect,
    pub field: Rect,
    pub entries: Vec<(Rect, Hit)>,
}

fn large_title_extra() -> f32 {
    (TextStyle::Large.line_height() - 28.0).max(0.0)
}
impl State {
    pub fn kind(&self) -> ReviewKind {
        self.review_kind
    }

    pub fn new(request: crate::export_requests::Request) -> Self {
        let mut field = TextField::new("");
        field.style = TextStyle::Mono;
        let data = request.shell_data();
        let is_script = data.get("script").is_some() || data["kind"] == "script";
        if is_script {
            field.set_text(&format!("脚本：{} · {}\n目的：{}\n工作目录：{}\n运行环境：{}\n超时：{} ms\n\n注意：以当前用户权限运行，可读写文件及访问网络，不是安全沙箱。inspect / preview 仅为意图声明。请先保存未保存文档。\n\nJSON 参数（MOCHI_SCRIPT_INPUT）：\n{}\n\n完整脚本：\n{}",
                data["script"]["language"].as_str().unwrap_or("未知"), data["script"]["intent"].as_str().unwrap_or("未知"),
                data["summary"].as_str().unwrap_or(""), request.output, data["runtime"].as_str().unwrap_or("不可用"), data["timeoutMs"],
                serde_json::to_string_pretty(&data["script"]["input"]).unwrap_or_default(), request.source));
        } else {
            field.set_text(&format!(
                "工作目录：{}\n\n完整命令：\n{}",
                request.output, request.source
            ));
        }
        Self {
            request,
            field,
            error: String::new(),
            review_kind: if is_script {
                ReviewKind::Script
            } else {
                ReviewKind::ShellCommand
            },
            file: None,
            files: Vec::new(),
            base_text: None,
            base_disk: None,
            proposed_text: None,
        }
    }
    pub fn file(
        entry: mochi_core::ai::agent_inbox::InboxEntry,
        current: Option<String>,
        disk: Option<Vec<u8>>,
    ) -> Self {
        Self::files(vec![entry], current, disk)
    }

    /// 为排队中的原生文档导出构建只读预览。
    ///
    /// 导出请求带有源路径、输出格式、目标位置，以及入队时捕获的源内容
    /// （可选）。把这些字段一起展示，审批决策就有据可查，也能与 shell
    /// 命令预览区分开。
    pub fn document_export(request: crate::export_requests::Request) -> Self {
        let mut field = TextField::new("");
        field.style = TextStyle::Mono;
        let body = document_export_details(&request);
        field.set_text(&body);
        Self {
            request,
            field,
            error: String::new(),
            review_kind: ReviewKind::DocumentExport,
            file: None,
            files: Vec::new(),
            base_text: None,
            base_disk: None,
            proposed_text: None,
        }
    }

    /// 给偏爱 `new` 前缀构造函数命名的调用方一个语义化别名；两个构造
    /// 函数产生相同的导出预览状态。
    pub fn new_document_export(request: crate::export_requests::Request) -> Self {
        Self::document_export(request)
    }

    pub fn files(
        entries: Vec<mochi_core::ai::agent_inbox::InboxEntry>,
        current: Option<String>,
        disk: Option<Vec<u8>>,
    ) -> Self {
        let Some(entry) = entries.first() else {
            return Self::new(crate::export_requests::Request {
                id: String::new(),
                source: String::new(),
                output: String::new(),
                format: "file".into(),
                content: None,
                created_at: 0,
                state: "pending".into(),
                session_id: None,
            });
        };
        let body = if entries.len() > 1 {
            let details = entries
                .iter()
                .enumerate()
                .map(|(index, item)| approval_summary(index, item))
                .collect::<Vec<_>>()
                .join("\n\n");
            format!(
                "文档：{}\n共 {} 项待批准修改\n\n{}",
                entry.path(),
                entries.len(),
                details
            )
        } else if entry.kind() == "block-edit" {
            let value = entry.to_value();
            let edit = &value["operation"]["blockEdit"];
            let before = edit["oldText"].as_str().unwrap_or("（缺少原文）");
            let after = edit["newText"].as_str().unwrap_or("（缺少修改）");
            let removed = before
                .lines()
                .map(|line| format!("- {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            let added = after
                .lines()
                .map(|line| format!("+ {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "文档：{}\n块：{}\n说明：{}\n\n修改前（-）：\n{}\n\n修改后（+）：\n{}",
                entry.path(),
                edit["blockId"].as_str().unwrap_or("?"),
                edit["summary"].as_str().unwrap_or(""),
                removed,
                added
            )
        } else if matches!(entry.kind(), "write" | "overwrite") {
            format!(
                "目标：{}\n\n当前内容：\n{}\n\n修改后内容：\n{}",
                entry.path(),
                current.as_deref().unwrap_or("（文件不存在或不是文本）"),
                entry.content().unwrap_or("（未提供内容）")
            )
        } else {
            format!(
                "操作：{}\n目标：{}\n新路径：{}\n\n{}",
                entry.kind(),
                entry.path(),
                entry.new_path().unwrap_or("—"),
                if entry.kind() == "delete-folder" {
                    "会永久删除目录及其全部子项。"
                } else if entry.kind() == "delete-file" {
                    "会永久删除这个文件。"
                } else {
                    "请核对路径。"
                }
            )
        };
        let proposed_source = if entries.len() > 1 {
            body.clone()
        } else if entry.kind() == "block-edit" {
            entry.to_value()["operation"]["blockEdit"]["newText"]
                .as_str()
                .unwrap_or(&body)
                .to_owned()
        } else {
            entry.content().unwrap_or(&body).to_owned()
        };
        let request = crate::export_requests::Request {
            id: entry.id().into(),
            source: proposed_source,
            output: entry.path().into(),
            format: "file".into(),
            content: None,
            created_at: 0,
            state: "pending".into(),
            session_id: None,
        };
        let mut s = Self::new(request);
        s.field.set_text(&body);
        s.base_text = current;
        s.base_disk = disk;
        s.review_kind = ReviewKind::FileChange;
        s.file = Some(entry.clone());
        s.files = entries;
        s
    }
}

fn document_export_details(request: &crate::export_requests::Request) -> String {
    let snapshot = match request.content.as_deref() {
        Some(content) => format!(
            "有（{} 字符）\n\n内容快照：\n{}",
            content.chars().count(),
            if content.is_empty() {
                "（快照为空）"
            } else {
                content
            }
        ),
        None => "无\n\n（批准时将从源文件读取当前内容。）".into(),
    };
    format!(
        "源文件：{}\n导出格式：{}\n输出路径：{}\n内容快照：{}",
        request.source, request.format, request.output, snapshot
    )
}

fn approval_summary(index: usize, entry: &mochi_core::ai::agent_inbox::InboxEntry) -> String {
    let heading = if entry.summary().is_empty() {
        entry.title()
    } else {
        entry.summary()
    };
    if entry.kind() == "block-edit" {
        let value = entry.to_value();
        let edit = &value["operation"]["blockEdit"];
        let before = edit["oldText"].as_str().unwrap_or("（缺少原文）");
        let after = edit["newText"].as_str().unwrap_or("（缺少修改）");
        return format!(
            "{}. {}\n块：{}\n- {}\n+ {}",
            index + 1,
            heading,
            edit["blockId"].as_str().unwrap_or("?"),
            before.replace('\n', "\n  - "),
            after.replace('\n', "\n  + ")
        );
    }
    if entry.path().to_ascii_lowercase().ends_with(".mcb")
        && matches!(entry.kind(), "write" | "overwrite")
    {
        if let Some(delta) = base_change_summary(entry) {
            return format!("{}. {}\n{}", index + 1, heading, delta);
        }
    }
    format!("{}. {}", index + 1, heading)
}

fn base_change_summary(entry: &mochi_core::ai::agent_inbox::InboxEntry) -> Option<String> {
    let before = mochi_core::base::parse_base_document(entry.previous_content()?).ok()?;
    let after = mochi_core::base::parse_base_document(entry.content()?).ok()?;
    let mut lines = Vec::new();
    for table in &after.tables {
        let previous = before
            .tables
            .iter()
            .find(|candidate| candidate.id == table.id);
        let Some(previous) = previous else {
            lines.push(format!("新增数据表「{}」", table.name));
            continue;
        };
        for record in &table.records {
            match previous
                .records
                .iter()
                .find(|candidate| candidate.id == record.id)
            {
                None => {
                    let values = record
                        .values
                        .iter()
                        .map(|(field_id, value)| {
                            let name = base_field_name(previous, table, field_id);
                            format!("{}={}", name, compact_json(value))
                        })
                        .collect::<Vec<_>>()
                        .join("，");
                    lines.push(format!(
                        "新增「{}」记录 {}：{}",
                        table.name, record.id, values
                    ));
                }
                Some(old) if old != record => {
                    let changed = base_record_field_changes(previous, table, old, record);
                    let changed = if changed.is_empty() {
                        "记录元数据已更新".into()
                    } else {
                        changed.join("，")
                    };
                    lines.push(format!(
                        "更新「{}」记录 {}：{}",
                        table.name, record.id, changed
                    ));
                }
                _ => {}
            }
        }
        for old_record in &previous.records {
            if !table
                .records
                .iter()
                .any(|record| record.id == old_record.id)
            {
                lines.push(format!("删除「{}」记录 {}", table.name, old_record.id));
            }
        }
        for old_field in &previous.fields {
            if !table.fields.iter().any(|field| field.id == old_field.id) {
                lines.push(format!("删除「{}」字段", old_field.name));
            }
        }
    }
    for old_table in &before.tables {
        if !after.tables.iter().any(|table| table.id == old_table.id) {
            lines.push(format!("删除数据表「{}」", old_table.name));
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn base_field_name(
    previous: &mochi_core::base::BaseTable,
    current: &mochi_core::base::BaseTable,
    field_id: &str,
) -> String {
    current
        .fields
        .iter()
        .find(|field| field.id == field_id)
        .or_else(|| previous.fields.iter().find(|field| field.id == field_id))
        .map(|field| field.name.clone())
        .unwrap_or_else(|| field_id.to_owned())
}

fn base_record_field_changes(
    previous: &mochi_core::base::BaseTable,
    current: &mochi_core::base::BaseTable,
    old: &mochi_core::base::BaseRecord,
    new: &mochi_core::base::BaseRecord,
) -> Vec<String> {
    // 先按数据结构定义中的顺序排列，再补上记录中存在但没有对应字段定义的值。
    // 后者对旧快照很重要：删掉一列后，其旧值可能只留在之前的记录里，
    // 预览中仍必须能看到它。
    let mut field_ids = Vec::<String>::new();
    for field in previous.fields.iter().chain(current.fields.iter()) {
        if !field_ids.iter().any(|id| id == &field.id) {
            field_ids.push(field.id.clone());
        }
    }
    for field_id in old.values.keys().chain(new.values.keys()) {
        if !field_ids.iter().any(|id| id == field_id) {
            field_ids.push(field_id.clone());
        }
    }

    field_ids
        .into_iter()
        .filter_map(|field_id| {
            let old_value = old.values.get(&field_id);
            let new_value = new.values.get(&field_id);
            if old_value == new_value {
                return None;
            }
            let name = base_field_name(previous, current, &field_id);
            Some(match (old_value, new_value) {
                (Some(before), Some(after)) => format!(
                    "{}：{} → {}",
                    name,
                    compact_json(before),
                    compact_json(after)
                ),
                (Some(before), None) => {
                    format!("{}：{} → 已删除", name, compact_json(before))
                }
                (None, Some(after)) => {
                    format!("{}：已新增 → {}", name, compact_json(after))
                }
                (None, None) => unreachable!("a field change needs an old or new value"),
            })
        })
        .collect()
}

fn compact_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "?".into())
}
pub fn layout(area: Rect) -> Layout {
    let title_extra = large_title_extra();
    let w = (area.width() - 32.0).min(720.0);
    let h = (area.height() - 32.0).min(540.0 + title_extra);
    let left = area.left + (area.width() - w) / 2.0;
    let top = area.top + (area.height() - h) / 2.0;
    let card = Rect::new(left, top, left + w, top + h);
    let field_top = top + 98.0 + title_extra;
    let field = Rect::new(
        left + 20.0,
        field_top,
        card.right - 20.0,
        (card.bottom - 102.0).max(field_top),
    );
    let mut entries = vec![
        (
            Rect::new(
                card.right - 124.0,
                top + 50.0 + title_extra,
                card.right - 20.0,
                top + 82.0 + title_extra,
            ),
            Hit::Copy,
        ),
        (
            Rect::new(
                left + 20.0,
                card.bottom - 86.0,
                left + w / 2.0 - 4.0,
                card.bottom - 54.0,
            ),
            Hit::Cancel,
        ),
        (
            Rect::new(
                left + w / 2.0 + 4.0,
                card.bottom - 86.0,
                card.right - 20.0,
                card.bottom - 54.0,
            ),
            Hit::Reject,
        ),
    ];
    if field.width() >= 120.0 && field.height() >= 80.0 {
        entries.push((
            Rect::new(
                left + 20.0,
                card.bottom - 46.0,
                card.right - 20.0,
                card.bottom - 14.0,
            ),
            Hit::Execute,
        ));
    }
    Layout {
        card,
        field,
        entries,
    }
}
pub fn paint(list: &mut DrawList, area: Rect, state: &mut State, p: &Palette) {
    let l = layout(area);
    list.rect_alpha(area, 0, 0.4);
    list.rounded_rect(l.card, 10.0, p.surface);
    list.push_clip(l.card);
    let title = if state.review_kind == ReviewKind::DocumentExport {
        "导出前确认".into()
    } else if state.review_kind == ReviewKind::Script {
        "执行脚本前确认".into()
    } else if state.files.len() > 1 {
        let path = state.files[0].path().replace('\\', "/");
        let name = path.rsplit('/').next().unwrap_or(path.as_str());
        format!("{} · {} 项修改", name, state.files.len())
    } else if state
        .file
        .as_ref()
        .is_some_and(|file| file.kind() == "block-edit")
    {
        "块修改审批".into()
    } else if state.file.is_some() {
        "文件变更前确认".into()
    } else {
        "执行命令前确认".into()
    };
    list.text(
        Rect::new(
            l.card.left + 20.0,
            l.card.top + 14.0,
            l.card.right - 20.0,
            l.card.top + 42.0 + large_title_extra(),
        ),
        title,
        TextStyle::Large,
        p.foreground,
    );
    let warning = if !state.error.is_empty() {
        state.error.as_str()
    } else if state.review_kind == ReviewKind::DocumentExport {
        "核对源文件、导出格式和输出路径后再批准导出；内容快照只读。"
    } else if state.files.len() > 1 {
        "整组修改已完成预检；批准时会再次检查文档，并一次写入全部修改。"
    } else if state
        .file
        .as_ref()
        .is_some_and(|file| file.kind() == "block-edit")
    {
        "只应用当前块；批准时再次检查原文。其余块保留当前内容。"
    } else if state.file.is_some() {
        "核对当前内容与修改内容后再批准；删除操作不可恢复。"
    } else if state.request.state == "interrupted" {
        "上次执行结果未知；重新执行前请先检查，避免重复操作。"
    } else {
        "使用当前用户权限执行，可能修改文件。下方内容只读。"
    };
    list.text(
        Rect::new(
            l.card.left + 20.0,
            l.card.top + 50.0 + large_title_extra(),
            l.card.right - 132.0,
            l.card.top + 82.0 + large_title_extra(),
        ),
        warning,
        TextStyle::Caption,
        if state.error.is_empty()
            && (state.file.is_some() || state.review_kind == ReviewKind::DocumentExport)
        {
            p.muted
        } else {
            p.danger
        },
    );
    state
        .field
        .paint_multiline_with_look(list, l.field, false, p, FieldLook::dialog(p));
    for (r, hit) in l.entries {
        let primary = hit == Hit::Execute
            && (state.review_kind == ReviewKind::DocumentExport
                || state.files.len() > 1
                || state
                    .file
                    .as_ref()
                    .is_some_and(|file| file.kind() == "block-edit"));
        if primary {
            list.glass_button(r, 5.0, p, false);
        } else {
            list.rounded_rect(
                r,
                5.0,
                if hit == Hit::Execute && state.error.is_empty() {
                    p.danger
                } else {
                    p.surface_elevated
                },
            );
        }
        list.text_aligned(
            r,
            match hit {
                Hit::Copy => {
                    if state.review_kind == ReviewKind::Script {
                        "复制脚本"
                    } else if state.review_kind == ReviewKind::DocumentExport {
                        "复制导出信息"
                    } else if state.file.is_some() {
                        "复制修改内容"
                    } else {
                        "复制命令"
                    }
                }
                Hit::Cancel => "暂不处理 · Esc",
                Hit::Reject => {
                    if state.files.len() > 1 {
                        "全部拒绝"
                    } else {
                        "拒绝"
                    }
                }
                Hit::Execute => {
                    if state.error.is_empty() {
                        if state.review_kind == ReviewKind::DocumentExport {
                            "批准导出 · Ctrl+Enter"
                        } else if state.files.len() > 1 {
                            "全部通过 · Ctrl+Enter"
                        } else {
                            "批准并执行 · Ctrl+Enter"
                        }
                    } else {
                        "有冲突，不能执行"
                    }
                }
            },
            TextStyle::Label,
            if primary {
                p.button_foreground()
            } else if hit == Hit::Execute && state.error.is_empty() {
                0xffffff
            } else {
                p.foreground
            },
            super::draw::Align::Center,
        );
    }
    list.pop_clip();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_windows_cannot_blindly_approve_commands() {
        let l = layout(Rect::new(0.0, 0.0, 240.0, 200.0));
        assert!(!l.entries.iter().any(|(_, h)| *h == Hit::Execute));
        let l = layout(Rect::new(0.0, 0.0, 1200.0, 800.0));
        assert!(l.entries.iter().any(|(_, h)| *h == Hit::Execute));
    }

    #[test]
    fn document_group_review_lists_every_change_in_one_dialog() {
        let entry = |id: &str, block: &str, before: &str, after: &str| {
            mochi_core::ai::agent_inbox::InboxEntry::from_value(&serde_json::json!({
                "id": id,
                "operation": {
                    "kind": "block-edit",
                    "path": "D:/ws/note.md",
                    "title": "note.md",
                    "summary": format!("修改 {block}"),
                    "status": "pending",
                    "blockEdit": {
                        "blockId": block,
                        "oldText": before,
                        "newText": after
                    }
                }
            }))
            .unwrap()
        };
        let mut state = State::files(
            vec![
                entry("inbox-1", "block-a", "原文一", "新文一"),
                entry("inbox-2", "block-b", "原文二", "新文二"),
            ],
            Some("原始文档".into()),
            None,
        );
        assert_eq!(state.files.len(), 2);
        assert!(state.field.text().contains("共 2 项待批准修改"));
        assert!(state.field.text().contains("块：block-a"));
        assert!(state.field.text().contains("- 原文一"));
        assert!(state.field.text().contains("+ 新文二"));

        let mut list = DrawList::new();
        paint(
            &mut list,
            Rect::new(0.0, 0.0, 1200.0, 800.0),
            &mut state,
            crate::ui::theme::tokens().palette(false),
        );
        let labels = list
            .cmds()
            .iter()
            .filter_map(|command| match command {
                crate::ui::draw::DrawCmd::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(labels.contains(&"note.md · 2 项修改"));
        assert!(labels.contains(&"全部拒绝"));
        assert!(labels.contains(&"全部通过 · Ctrl+Enter"));
    }

    #[test]
    fn document_export_review_shows_request_details_and_uses_export_action() {
        let request = crate::export_requests::Request {
            id: "export-1".into(),
            source: "notes/日报.md".into(),
            output: "exports/日报.pdf".into(),
            format: "pdf".into(),
            content: Some("# 今日记录\n\n已完成".into()),
            created_at: 0,
            state: "pending".into(),
            session_id: Some("session-1".into()),
        };
        let mut state = State::document_export(request);
        assert_eq!(state.kind(), ReviewKind::DocumentExport);
        assert!(state.file.is_none());
        assert!(state.field.text().contains("源文件：notes/日报.md"));
        assert!(state.field.text().contains("导出格式：pdf"));
        assert!(state.field.text().contains("输出路径：exports/日报.pdf"));
        assert!(state.field.text().contains("内容快照：有"));
        assert!(state.field.text().contains("# 今日记录"));

        let mut list = DrawList::new();
        paint(
            &mut list,
            Rect::new(0.0, 0.0, 1200.0, 800.0),
            &mut state,
            crate::ui::theme::tokens().palette(false),
        );
        let labels = list
            .cmds()
            .iter()
            .filter_map(|command| match command {
                crate::ui::draw::DrawCmd::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(labels.contains(&"导出前确认"));
        assert!(labels.contains(&"批准导出 · Ctrl+Enter"));
        assert!(!labels.contains(&"批准并执行 · Ctrl+Enter"));

        let no_snapshot = crate::export_requests::Request {
            id: "export-2".into(),
            source: "notes/日报.md".into(),
            output: "exports/日报.html".into(),
            format: "html".into(),
            content: None,
            created_at: 0,
            state: "pending".into(),
            session_id: None,
        };
        assert!(State::new_document_export(no_snapshot)
            .field
            .text()
            .contains("内容快照：无"));
    }

    #[test]
    fn base_group_summary_includes_deleted_and_updated_fields() {
        let mut before = mochi_core::base::create_base_document();
        let table = &mut before.tables[0];
        let deleted_field =
            mochi_core::base::create_base_field(mochi_core::base::FieldType::Text, "待删除字段");
        let updated_field =
            mochi_core::base::create_base_field(mochi_core::base::FieldType::Text, "更新字段");
        let deleted_id = deleted_field.id.clone();
        let updated_id = updated_field.id.clone();
        table.fields.push(deleted_field);
        table.fields.push(updated_field);

        let mut old_record = mochi_core::base::BaseRecord {
            id: "record-1".into(),
            ..Default::default()
        };
        old_record
            .values
            .insert(deleted_id.clone(), serde_json::json!("旧值"));
        old_record
            .values
            .insert(updated_id.clone(), serde_json::json!("旧状态"));
        table.records.push(old_record);

        let mut after = before.clone();
        let record = &mut after.tables[0].records[0];
        record.values.remove(&deleted_id);
        record
            .values
            .insert(updated_id, serde_json::json!("新状态"));

        let previous_content = mochi_core::base::serialize_base_document(&before).unwrap();
        let content = mochi_core::base::serialize_base_document(&after).unwrap();
        let entry = mochi_core::ai::agent_inbox::InboxEntry::from_value(&serde_json::json!({
            "id": "mcb-review",
            "operation": {
                "kind": "overwrite",
                "path": "data.mcb",
                "status": "pending",
                "previousContent": previous_content,
                "content": content
            }
        }))
        .unwrap();

        let summary = base_change_summary(&entry).unwrap();
        assert!(
            summary.contains("待删除字段：\"旧值\" → 已删除"),
            "{summary}"
        );
        assert!(
            summary.contains("更新字段：\"旧状态\" → \"新状态\""),
            "{summary}"
        );
    }
}
