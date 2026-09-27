use super::*;
use crate::ui::{assistant, draw::DrawCmd};

fn script() -> String {
    (0..45)
        .map(|index| format!("print('图片引用检查 {index}：中文内容')\n"))
        .collect()
}

fn proposal(status: &str) -> Value {
    serde_json::json!({
        "id": "layout-only", "requestId": "layout-only", "status": status,
        "program": "python", "kind": "script", "command": script(),
        "script": {"code": script(), "intent": "inspect"},
        "summary": "只读检查图片引用。\n保留文档和资源的原始内容。",
        "runtime": "D:/应用程序/墨池/runtime/python/python.exe",
        "cwd": "D:/知识库/墨池"
    })
}

#[test]
fn multiline_approval_card_emits_only_single_visual_lines() {
    let card = pending_card_from_value(
        "pendingShellCommand",
        PendingCardKind::ShellCommand,
        proposal("pending"),
        None,
    )
    .unwrap();
    let rect = Rect::from_size(16.0, 100.0, 420.0, pending_card_height(&card, 420.0));
    let mut list = DrawList::new();
    let review =
        paint_pending_card(&mut list, rect, &card, theme::tokens().palette(false)).unwrap();
    for cmd in list.cmds() {
        if let DrawCmd::Text {
            text,
            rect: line,
            style,
            ..
        } = cmd
        {
            assert!(
                !text.contains(['\n', '\r', '\u{2028}', '\u{2029}', '\u{85}']),
                "multiline text leaked into a single row: {text:?}"
            );
            assert!(line.top >= rect.top && line.bottom <= rect.bottom);
            assert!(
                line.height() >= style.line_height(),
                "row must reserve its actual font height"
            );
            if text != card.review_label() {
                assert!(line.bottom <= review.top);
            }
        }
    }
    assert_eq!(
        card.raw["command"],
        script(),
        "rendering must preserve the complete approval payload"
    );
    assert!(list.finish().is_ok());
}

#[test]
#[cfg(debug_assertions)]
fn multiline_approval_card_native_rendering_keeps_neighboring_messages_separate() {
    use mochi_core::ai::session::AiConversation;
    use windows::Win32::Foundation::HWND;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let mut renderer = crate::gfx::Renderer::new().unwrap();
    for (width, dpi, dark) in [
        (420.0, 144.0, false),
        (280.0, 192.0, true),
        (640.0, 96.0, false),
    ] {
        let snapshot = renderer
            .prepare_snapshot(
                (width * dpi / 96.0) as u32,
                (950.0 * dpi / 96.0) as u32,
                dpi,
            )
            .unwrap();
        let mut request = AiStoredMessage::new(
            "assistant",
            "已准备只读检查脚本，可在审批窗口查看完整内容。",
        );
        request.set("pendingShellCommand", proposal("pending"));
        let mut state = assistant::State::default();
        state.active = Some(AiConversation {
            id: "layout-only".into(), title: "图片引用检查".into(), created_at: 0, updated_at: 0,
            messages: vec![
                AiStoredMessage::new("assistant", "文档中的中文、图片引用和代码应当清晰可读。\n\n审批卡片会显示脚本预览，完整脚本保留在审批窗口中。"),
                request,
                AiStoredMessage::new("assistant", "这条后续消息应与审批卡片保持间距，文字不会互相覆盖。")
            ],
        });
        let area = Rect::from_size(0.0, 0.0, width, 950.0);
        let layout = assistant::layout(&state, area);
        let mut list = DrawList::new();
        let p = theme::tokens().palette(dark);
        assistant::paint(&mut list, area, &state, &layout, false, p);
        assert!(list.finish().is_ok());
        renderer
            .present(HWND::default(), p.background, &list)
            .unwrap();
        if let Some(output) = std::env::var_os("MOCHI_AI_CARD_QA_DIR") {
            let output = std::path::PathBuf::from(output);
            std::fs::create_dir_all(&output).unwrap();
            renderer
                .save_snapshot(
                    &snapshot,
                    &output.join(format!("ai-approval-{width}-{dpi}.png")),
                )
                .unwrap();
        }
        let message = &layout.messages[1];
        assert!(message.pending_card_top + message.pending_card_height < layout.messages[2].top);
        let review = layout
            .rect_of(assistant::Hit::PendingCardReview(1))
            .unwrap();
        assert_eq!(
            layout.hit(review.left + 10.0, review.top + 10.0),
            Some(assistant::Hit::PendingCardReview(1))
        );
        for cmd in list.cmds() {
            if let DrawCmd::Text { text, .. } = cmd {
                assert!(
                    !text.contains(['\n', '\r']),
                    "approval text must be laid out before painting"
                );
            }
        }
    }
}

#[test]
fn multiline_approval_card_bounds_hold_for_long_titles_errors_and_completed_results() {
    let p = theme::tokens().palette(false);
    for width in [180.0, 280.0, 420.0] {
        for status in ["pending", "applied", "error"] {
            let mut card = pending_card_from_value(
                "pendingShellCommand",
                PendingCardKind::ShellCommand,
                proposal(status),
                None,
            )
            .unwrap();
            card.title = "很长的脚本标题\r\n第二行标题\u{2028}第三行".repeat(12);
            card.summary = "第一行摘要\r\n第二行摘要\r第三行\u{85}第四行\u{2029}第五行".into();
            card.error = Some("脚本错误：中文路径和 emoji 🖼️\n详细错误\n更多信息".repeat(40));
            card.details
                .push(("标准输出".into(), "多行结果\n".repeat(200)));
            let rect = Rect::from_size(10.0, 20.0, width, pending_card_height(&card, width));
            assert!(rect.height() <= PENDING_CARD_MAX_HEIGHT);
            let mut list = DrawList::new();
            let review = paint_pending_card(&mut list, rect, &card, p);
            assert_eq!(review.is_some(), status == "pending");
            let mut text_rects: Vec<Rect> = Vec::new();
            let mut has_error = false;
            let mut truncated = false;
            for cmd in list.cmds() {
                if let DrawCmd::Text {
                    text,
                    rect: row,
                    color,
                    ..
                } = cmd
                {
                    assert!(!text.contains(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}']));
                    assert!(row.top >= rect.top && row.bottom <= rect.bottom);
                    assert!(row.left >= rect.left && row.right <= rect.right);
                    assert!(
                        text_rects.iter().all(|previous| previous.bottom <= row.top
                            || previous.top >= row.bottom
                            || previous.right <= row.left
                            || previous.left >= row.right),
                        "preview rows must not overlap"
                    );
                    text_rects.push(*row);
                    has_error |= *color == p.danger;
                    truncated |= text.ends_with('…');
                }
            }
            assert!(
                has_error && truncated,
                "errors stay visible within bounded previews"
            );
            assert!(list.finish().is_ok());
        }
    }
}

#[test]
fn multiline_approval_card_fixed_height_labels_flatten_breaks_without_touching_payloads() {
    let label = "中文 🖼️\r\n第二行\r第三行\u{2028}第四行\u{85}第五行";
    assert_eq!(
        single_line_preview(label, TextStyle::Small, 2000.0),
        "中文 🖼️ 第二行 第三行 第四行 第五行"
    );
    assert_eq!(
        single_line_preview("路径 C:\\new\\notes", TextStyle::Small, 2000.0),
        "路径 C:\\new\\notes"
    );
}
