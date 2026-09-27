use super::*;
use crate::ui::draw::DrawCmd;
use mochi_core::ai::session::AiStoredMessage;

const AREA: Rect = Rect {
    left: 780.0,
    top: 72.0,
    right: 1200.0,
    bottom: 800.0,
};

struct ResetSettings;

impl Drop for ResetSettings {
    fn drop(&mut self) {
        crate::ui::settings_values::reset();
    }
}

fn load_test_settings(values: &[(&str, mochi_core::app_settings::SettingValue)]) -> ResetSettings {
    use mochi_core::{app_settings::AppSettings, settings::SettingsService};
    use std::sync::Arc;

    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let settings = AppSettings::new(Arc::new(SettingsService::new(Some(
        std::env::temp_dir().join(format!("mochi-assistant-ui-test-{suffix}.json")),
    ))));
    for (key, value) in values {
        let descriptor = mochi_core::app_settings::descriptor(key)
            .unwrap_or_else(|| panic!("missing test setting {key}"));
        settings.write(descriptor, value);
    }
    crate::ui::settings_values::load(&settings);
    ResetSettings
}

#[test]
fn composer_attachment_controls_take_priority_over_input_background() {
    for standalone in [false, true] {
        for width in [360.0, 800.0, 1500.0] {
            let mut state = State::default();
            state.standalone = standalone;
            state.pending_files = vec!["notes/document.md".into(); 8];
            state.pending_images.push("image.png".into());
            state.pending_selections.push(
                context::selection_from_document("notes/a.md", None, "selected text", (0, 8))
                    .unwrap()
                    .to_value(),
            );
            let area = Rect::from_size(0.0, 0.0, width, 900.0);
            let initial = layout(&state, area);
            for scroll in [0.0, initial.files_max_scroll] {
                state.files_scroll = scroll;
                let lay = layout(&state, area);
                let mut file_removals = 0;
                for (rect, hit) in &lay.entries {
                    if matches!(
                        hit,
                        Hit::OpenFile(_)
                            | Hit::RemoveFile(_)
                            | Hit::RemoveImage(_)
                            | Hit::PendingSelectionLocate(_)
                            | Hit::PendingSelectionRemove(_)
                            | Hit::FilesBack
                            | Hit::FilesForward
                    ) {
                        if matches!(hit, Hit::RemoveFile(_)) {
                            file_removals += 1;
                        }
                        // 选区移除按钮和所属小片重叠。
                        let x = if matches!(hit, Hit::PendingSelectionLocate(_)) {
                            rect.left + 2.0
                        } else {
                            (rect.left + rect.right) / 2.0
                        };
                        assert_eq!(lay.hit(x, (rect.top + rect.bottom) / 2.0), Some(*hit));
                    }
                }
                assert!(file_removals > 0);
                let input = lay.rect_of(Hit::Input).unwrap();
                assert_eq!(lay.hit(input.left + 2.0, input.top + 2.0), Some(Hit::Input));
                let wrapper = lay.composer_rect.unwrap();
                assert_eq!(
                    lay.hit(wrapper.left + 1.0, wrapper.top + 1.0),
                    Some(Hit::Input)
                );
            }
        }
    }
}

#[test]
fn conversation_history_scroll_reaches_oldest_and_clips_hit_targets() {
    let mut state = State::default();
    state.show_conversations = true;
    state.sessions = (0..25)
        .map(|i| AiSessionMeta {
            id: format!("session-{i}"),
            updated_at: 25 - i,
            ..Default::default()
        })
        .collect();
    for height in [360.0, 728.0] {
        let area = Rect::from_size(780.0, 72.0, 420.0, height);
        state.conversations_scroll = 0.0;
        let initial = layout(&state, area);
        assert!(initial.conversations_max_scroll > 0.0);
        assert!(initial.popover.unwrap().bottom <= area.bottom);
        assert!(initial.rect_of(Hit::Session(0)).is_some());
        assert!(initial.rect_of(Hit::Session(24)).is_none());
        state.conversations_scroll = initial.conversations_max_scroll + 100.0;
        let end = layout(&state, area);
        assert!(end.rect_of(Hit::Session(0)).is_none());
        let last = end.rect_of(Hit::Session(24)).unwrap();
        assert_eq!(
            end.hit(last.left + 12.0, last.top + 24.0),
            Some(Hit::Session(24))
        );
        let delete = end.rect_of(Hit::SessionDelete(24)).unwrap();
        assert_eq!(
            end.hit(delete.left + 2.0, delete.top + 2.0),
            Some(Hit::SessionDelete(24))
        );
        let viewport = end.conversations_viewport.unwrap();
        for (rect, hit) in &end.entries {
            if matches!(hit, Hit::Session(_) | Hit::SessionDelete(_)) {
                assert!(rect.top >= viewport.top && rect.bottom <= viewport.bottom);
            }
        }
        state.sessions.truncate(1);
        assert_eq!(layout(&state, area).conversations_max_scroll, 0.0);
        state.sessions = (0..25)
            .map(|i| AiSessionMeta {
                id: format!("session-{i}"),
                updated_at: 25 - i,
                ..Default::default()
            })
            .collect();
    }
}

#[test]
fn standalone_settings_and_agent_entry_keep_order_and_chat_readable() {
    for width in [360.0, 440.0, 800.0, 1500.0] {
        let mut state = State::default();
        state.standalone = true;
        state.active = Some(conversation());
        let lay = layout(&state, Rect::from_size(0.0, 0.0, width, 900.0));
        let settings = lay.rect_of(Hit::Settings).unwrap();
        let agents = lay.rect_of(Hit::AgentConfig).unwrap();
        assert!(settings.right < agents.left && agents.right < width);
        assert!(lay.rect_of(Hit::Close).is_none());
        assert!(lay.messages_rect.width() <= 1040.0);
        let composer = lay.composer_rect.unwrap();
        assert_eq!(composer.left, lay.messages_rect.left + INPUT_PAD_X);
        assert_eq!(composer.right, lay.messages_rect.right - INPUT_PAD_X);
        if let Some(navigation) = lay.navigation_rect {
            assert!(lay.messages_rect.right < navigation.left);
            assert_eq!(navigation.width(), NAV_W);
        }
    }
}

fn conversation() -> AiConversation {
    AiConversation {
        id: "c1".into(),
        title: "测试".into(),
        created_at: 0,
        updated_at: 0,
        messages: vec![
            AiStoredMessage::new("user", "进程和线程的区别？"),
            AiStoredMessage::new(
                "assistant",
                "**进程**拥有独立地址空间。\n\n线程共享进程资源。",
            ),
            {
                let mut m = AiStoredMessage::new("tool", "{\"ok\":true}");
                m.set("hidden", serde_json::json!(true));
                m
            },
        ],
    }
}

#[test]
fn user_bubble_shrinks_to_short_content_and_keeps_the_wrapping_cap() {
    let max_width = 304.0;
    let compact = user_message_body_width("正文字体大小了，改大一些", max_width, false);
    let long = user_message_body_width(
        &"这是一段需要自动换行的用户消息。".repeat(20),
        max_width,
        false,
    );

    assert!(compact >= 40.0 && compact < max_width * 0.75);
    assert!(long > max_width - 24.0 && long <= max_width);

    let mut state = State::default();
    state.active = Some(AiConversation {
        id: "compact-user-bubble".into(),
        messages: vec![AiStoredMessage::new("user", "改大一些")],
        ..Default::default()
    });
    let laid_out = layout(&state, AREA);
    let message = &laid_out.messages[0];
    let bubble = message.bubble.expect("user message bubble");
    assert!(message.body_width < max_width * 0.5);
    assert_eq!(bubble.right, laid_out.messages_rect.right - MSG_PAD_X - 2.0);
}

#[test]
fn chat_spacing_padding_and_user_bubble_color_follow_settings() {
    use mochi_core::app_settings::SettingValue;

    let _settings = load_test_settings(&[
        ("assistant.messageSpacing", SettingValue::Number(8.0)),
        ("assistant.messagePadding", SettingValue::Number(18.0)),
        ("assistant.messageRadius", SettingValue::Number(16.0)),
        (
            "assistant.userBubbleColor",
            SettingValue::Text("#123456".into()),
        ),
    ]);
    let mut state = State::default();
    state.active = Some(AiConversation {
        id: "custom-chat-surface".into(),
        messages: vec![
            AiStoredMessage::new("user", "你好"),
            AiStoredMessage::new("assistant", "收到。"),
        ],
        ..Default::default()
    });
    let area = Rect::from_size(0.0, 0.0, 720.0, 680.0);
    let layout = layout(&state, area);
    let user = &layout.messages[0];
    let next = &layout.messages[1];
    let bubble = user.bubble.unwrap();
    assert_eq!(next.top, user.top + user.height + 8.0);
    assert_eq!(bubble.left, user.body_left - 18.0);

    let mut list = DrawList::new();
    paint(
        &mut list,
        area,
        &state,
        &layout,
        false,
        theme::tokens().palette(false),
    );
    assert!(list.cmds().iter().any(|command| matches!(
        command,
        DrawCmd::RoundedRect { radius, color, .. }
            if *radius == 16.0 && *color == 0x123456
    )));
}

fn texts(list: &DrawList) -> Vec<String> {
    list.cmds()
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn trace_plain_text_reserves_explicit_line_breaks_without_overpainting() {
    let source = "The user wants me to write into the table. Let me\nfirst check the current schema and state of the file.\n\n思考后再调用工具。";
    let width = 190.0;
    let style = TextStyle::Small;
    let lines = wrapped_plain_lines(source, width, style);
    assert!(lines.len() >= source.split('\n').count());
    assert!(lines.iter().all(|line| !line.contains(['\n', '\r'])));
    assert!(
        lines.iter().any(String::is_empty),
        "blank paragraph must keep its vertical space"
    );
    assert_eq!(
        wrapped_height(source, width, style, None),
        lines.len() as f32 * style.line_height()
    );

    let mut list = DrawList::new();
    paint_wrapped_text(
        &mut list,
        source,
        Rect::new(0.0, 0.0, width, 1_000.0),
        style,
        0xFFFFFF,
        None,
    );
    let painted = list
        .cmds()
        .iter()
        .filter_map(|command| match command {
            DrawCmd::Text { rect, text, .. } => Some((*rect, text)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected_painted = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(painted.len(), expected_painted.len());
    assert!(painted.iter().all(|(_, text)| !text.contains(['\n', '\r'])));
    for ((rect, text), (line_index, expected)) in painted.iter().zip(expected_painted) {
        assert_eq!(*text, expected);
        assert_eq!(rect.top, line_index as f32 * style.line_height());
        assert_eq!(rect.bottom, rect.top + style.line_height());
    }
}

#[test]
fn hidden_and_tool_messages_are_not_visible() {
    let mut s = State::default();
    s.active = Some(conversation());
    let v = s.visible_messages();
    assert_eq!(v.len(), 2);
    assert_eq!(v[0].0, "user");
}

#[test]
fn the_header_has_the_five_actions_and_the_input_sits_at_the_bottom() {
    let s = State::default();
    let lay = layout(&s, AREA);
    for h in [
        Hit::Conversations,
        Hit::NewSession,
        Hit::Clear,
        Hit::Settings,
        Hit::Close,
    ] {
        let r = lay.rect_of(h).unwrap();
        assert_eq!(r.height(), 32.0);
        assert!(r.top >= AREA.top && r.bottom <= AREA.top + HEADER_H);
    }
    let close = lay.rect_of(Hit::Close).unwrap();
    assert_eq!(close.right, AREA.right - 16.0);
    let input = lay.rect_of(Hit::Input).unwrap();
    assert!(input.bottom <= AREA.bottom - INPUT_PAD_BOTTOM);
    let send = lay.rect_of(Hit::Send).unwrap();
    assert_eq!(send.width(), 28.0);
    assert!(send.top >= input.bottom);
    assert_eq!(lay.messages_rect.top, AREA.top + HEADER_H);
    let composer = lay.composer_rect.unwrap();
    assert_eq!(composer.bottom, AREA.bottom - INPUT_PAD_BOTTOM);
    assert_eq!(lay.messages_rect.bottom, composer.top - 8.0);
}

#[test]
fn composer_repaints_keep_text_and_caret_stationary() {
    let long_input = "内容\n".repeat(20);
    for standalone in [false, true] {
        for source in [
            "",
            "你可以执行工作流",
            "hello",
            "第一行\n第二行",
            &long_input,
        ] {
            let mut state = State::default();
            state.standalone = standalone;
            state.input.set_text(source);
            let mut previous = None;
            for frame in 0..8 {
                let lay = layout(&state, AREA);
                let input = lay.rect_of(Hit::Input).unwrap();
                // 与两种应用界面保持一致：先同步持久字段，再绘制其克隆。
                state.input.sync_multiline_scroll(input);
                let mut list = DrawList::new();
                paint(
                    &mut list,
                    AREA,
                    &state,
                    &lay,
                    true,
                    theme::tokens().palette(false),
                );
                list.set_caret_visible(frame % 2 == 0);
                let caret = list.caret_rect().unwrap();
                assert_eq!(
                    caret,
                    state.input.multiline_caret(input),
                    "paint must use the same scroll as input hit testing"
                );
                let text_rects: Vec<_> = list
                    .cmds()
                    .iter()
                    .filter_map(|cmd| match cmd {
                        DrawCmd::Text { rect, .. }
                            if rect.top >= input.top && rect.bottom <= input.bottom =>
                        {
                            Some(*rect)
                        }
                        _ => None,
                    })
                    .collect();
                let geometry = (caret, text_rects);
                if let Some(previous) = &previous {
                    assert_eq!(&geometry, previous, "frame {frame}, source {source:?}");
                }
                previous = Some(geometry);
            }
        }
    }
}

#[test]
fn composer_grows_to_fit_wrapped_text_before_scrolling() {
    for standalone in [false, true] {
        for width in [360.0, 640.0, 1200.0] {
            let area = Rect::from_size(0.0, 0.0, width, 800.0);
            let mut state = State::default();
            state.standalone = standalone;
            let input = layout(&state, area).rect_of(Hit::Input).unwrap();
            // 取旧高度估算所用宽度与实际文本视口的中间值。
            let count =
                ((input.width() - 12.0) / text::measure("中", state.input.style)).ceil() as usize;
            state.input.set_text(&"中".repeat(count));
            for composing in [false, true] {
                if composing {
                    state.input.buffer.set_composition("wen", 3);
                }
                let input = layout(&state, area).rect_of(Hit::Input).unwrap();
                assert!(input.height() < 140.0);
                let unscrolled = state.input.multiline_caret(input);
                state.input.sync_multiline_scroll(input);
                assert_eq!(
                    state.input.multiline_caret(input),
                    unscrolled,
                    "short wrapped text should fit without scrolling"
                );
            }
        }
    }
}

#[test]
fn messages_are_laid_out_top_to_bottom_with_the_streaming_one_last() {
    let mut s = State::default();
    s.active = Some(conversation());
    s.streaming = Some(Streaming {
        status: "正在思考…".into(),
        content: "线程".into(),
        tools: vec![("file_read".into(), Some(true), Some("a.md".into()))],
        ..Default::default()
    });
    let lay = layout(&s, AREA);
    assert_eq!(lay.messages.len(), 3);
    assert!(lay.messages[0].top < lay.messages[1].top && lay.messages[1].top < lay.messages[2].top);
    assert!(lay.messages[2].streaming);
    assert_eq!(lay.messages[2].tools.len(), 1);
    // 粗体标记不原样画出来
    let mut list = DrawList::new();
    paint(
        &mut list,
        AREA,
        &s,
        &lay,
        true,
        theme::tokens().palette(false),
    );
    let t = texts(&list);
    assert!(t.iter().any(|x| x == "进程"), "{t:?}");
    assert!(!t.iter().any(|x| x.contains("**")));
    assert!(t.iter().any(|x| x.contains("file_read")));
    assert!(list.finish().is_ok());
}

#[test]
fn streaming_without_reasoning_keeps_spinner_and_final_answer_visible() {
    let mut s = State::default();
    s.streaming = Some(Streaming {
        status: "正在生成".into(),
        content: "最终正文".into(),
        animation_phase: 1,
        ..Default::default()
    });
    let lay = layout(&s, AREA);
    let message = &lay.messages[0];
    assert!(message.streaming);
    assert!(message.streaming_reasoning.is_none());
    assert!(!message
        .trace
        .iter()
        .any(|value| trace_kind(value) == Some("thinking")));

    let mut first = DrawList::new();
    paint(
        &mut first,
        AREA,
        &s,
        &lay,
        false,
        theme::tokens().palette(false),
    );
    assert!(texts(&first).iter().any(|text| text.contains("最终正文")));
    let first_alpha = first
        .cmds()
        .iter()
        .filter_map(|command| match command {
            DrawCmd::RoundedRectAlpha { alpha, .. } => Some(*alpha),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(first_alpha.len(), 8, "status spinner must have eight dots");

    s.streaming.as_mut().unwrap().animation_phase = 2;
    let next = layout(&s, AREA);
    let mut second = DrawList::new();
    paint(
        &mut second,
        AREA,
        &s,
        &next,
        false,
        theme::tokens().palette(false),
    );
    let second_alpha = second
        .cmds()
        .iter()
        .filter_map(|command| match command {
            DrawCmd::RoundedRectAlpha { alpha, .. } => Some(*alpha),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_ne!(first_alpha, second_alpha, "status spinner must animate");
    assert!(first.finish().is_ok());
    assert!(second.finish().is_ok());
}

#[test]
fn an_empty_conversation_shows_the_hint_and_no_provider_shows_the_settings_button() {
    let mut s = State::default();
    let mut list = DrawList::new();
    paint(
        &mut list,
        AREA,
        &s,
        &layout(&s, AREA),
        false,
        theme::tokens().palette(false),
    );
    assert!(texts(&list).contains(&"开始与 AI 对话".to_owned()));
    s.provider_missing = true;
    let lay = layout(&s, AREA);
    assert!(lay.rect_of(Hit::OpenSettings).is_some());
    let mut list = DrawList::new();
    paint(
        &mut list,
        AREA,
        &s,
        &lay,
        false,
        theme::tokens().palette(false),
    );
    assert!(texts(&list).contains(&"还没有配置 AI 提供商".to_owned()));
}

#[test]
fn saved_conversation_stays_readable_offline_and_toolbar_does_not_overlap_messages() {
    let s = State {
        active: Some(conversation()),
        provider_missing: true,
        ..Default::default()
    };
    let lay = layout(&s, AREA);
    assert!(lay.rect_of(Hit::OpenSettings).is_none());
    for hit in [
        Hit::AgentPicker,
        Hit::MountSession,
        Hit::ApproveMode,
        Hit::AutomaticMode,
    ] {
        let r = lay.rect_of(hit).unwrap();
        assert!(r.top >= lay.messages_rect.bottom);
        assert!(r.top >= lay.rect_of(Hit::Input).unwrap().bottom);
    }
    let mut list = DrawList::new();
    paint(
        &mut list,
        AREA,
        &s,
        &lay,
        false,
        theme::tokens().palette(false),
    );
    assert!(texts(&list).iter().any(|s| s.contains("进程和线程")));
    assert!(!texts(&list).iter().any(|s| s.contains("还没有配置")));
}

#[test]
fn persisted_plan_uses_electron_trace_shape_and_is_visible_on_reopen() {
    let plan = AgentTraceStep::Plan {
        steps: vec![serde_json::json!({"title":"复核计划","status":"done"})],
        note: None,
    };
    let value = serde_json::to_value(vec![plan]).unwrap();
    assert_eq!(value[0]["kind"], "plan");
    assert!(value[0].get("note").is_none());
    let mut c = conversation();
    c.messages[1].set("trace", value);
    let s = State {
        active: Some(c),
        ..Default::default()
    };
    assert!(s.visible_messages()[1].1.contains("✓ 复核计划"));
    let tool = serde_json::to_value(AgentTraceStep::Tool {
        name: "file_read".into(),
        summary: None,
        ok: true,
        duration_ms: 5,
        result_preview: None,
    })
    .unwrap();
    assert_eq!(tool["durationMs"], 5);
    assert!(tool.get("duration_ms").is_none());
}

#[test]
fn the_conversations_popover_lists_sessions_newest_first_and_wins_hit_testing() {
    let mut s = State::default();
    s.sessions = vec![
        AiSessionMeta {
            id: "a".into(),
            title: "旧".into(),
            updated_at: 1,
            ..Default::default()
        },
        AiSessionMeta {
            id: "b".into(),
            title: "新".into(),
            updated_at: 2,
            ..Default::default()
        },
    ];
    s.show_conversations = true;
    let lay = layout(&s, AREA);
    let pop = lay.popover.unwrap();
    let first = lay.rect_of(Hit::Session(0)).unwrap();
    assert!(first.top > pop.top);
    assert_eq!(s.sorted_sessions()[0].title, "新");
    assert_eq!(
        lay.hit(first.left + 40.0, first.top + 10.0),
        Some(Hit::Session(0))
    );
    let del = lay.rect_of(Hit::SessionDelete(0)).unwrap();
    assert_eq!(
        lay.hit(del.left + 4.0, del.top + 4.0),
        Some(Hit::SessionDelete(0))
    );
    let mut list = DrawList::new();
    paint(
        &mut list,
        AREA,
        &s,
        &lay,
        false,
        theme::tokens().palette(false),
    );
    assert!(texts(&list).contains(&"新".to_owned()));
}

#[test]
fn long_replies_scroll() {
    let mut s = State::default();
    let mut c = conversation();
    for _ in 0..40 {
        c.messages
            .push(AiStoredMessage::new("assistant", "一行\n两行\n三行"));
    }
    s.active = Some(c);
    let lay = layout(&s, AREA);
    assert!(lay.max_scroll() > 0.0);
}

#[test]
fn pending_edit_message_has_stable_card_actions_and_height() {
    let mut message = AiStoredMessage::new("assistant", "建议修改：润色第二段");
    message.set("id", serde_json::json!("pending-message"));
    message.set(
        "pendingEdit",
        serde_json::json!({
            "id": "edit-1",
            "inboxId": "inbox-1",
            "path": "知识库/笔记.md",
            "startLine": 2,
            "endLine": 3,
            "summary": "润色第二段",
            "status": "pending"
        }),
    );
    let mut s = State::default();
    s.active = Some(AiConversation {
        id: "session".into(),
        messages: vec![message],
        ..Default::default()
    });
    let lay = layout(&s, AREA);
    assert!(lay.messages[0].pending_edit.is_some());
    assert!(lay.messages[0].height > lay.messages[0].body.height);
    let buttons = lay.pending_edit_buttons(0, 0.0);
    assert_eq!(buttons.len(), 2);
    for (rect, hit) in buttons {
        assert_eq!(
            lay.message_hit(
                0.0,
                (rect.left + rect.right) / 2.0,
                (rect.top + rect.bottom) / 2.0
            ),
            Some(hit)
        );
    }
    let mut list = DrawList::new();
    paint(
        &mut list,
        AREA,
        &s,
        &lay,
        false,
        theme::tokens().palette(false),
    );
    assert!(texts(&list).iter().any(|text| text == "建议修改"));
    assert!(texts(&list).iter().any(|text| text == "批准修改"));
    assert!(list.finish().is_ok());
}

#[test]
fn structured_trace_keeps_collapsed_and_expanded_rows_hit_testable() {
    let thinking = AgentTraceStep::Thinking {
        text: "核对引用并整理回答依据".into(),
        duration_ms: Some(12),
    };
    let tool = AgentTraceStep::Tool {
        name: "file_read".into(),
        summary: Some("读取说明文档".into()),
        ok: true,
        duration_ms: 48,
        result_preview: Some("README.md · 18 行".into()),
    };
    let plan = AgentTraceStep::Plan {
        steps: vec![serde_json::json!({"title":"复核引用","status":"done"})],
        note: None,
    };
    let mut answer = AiStoredMessage::new("assistant", "最终回答");
    answer.set(
        "trace",
        serde_json::to_value(vec![plan, thinking, tool]).unwrap(),
    );
    answer.set("model", serde_json::json!("fixture-model"));
    answer.set("usage", serde_json::json!({"totalTokens": 42}));
    answer.set("interrupted", serde_json::json!(true));
    let mut s = State::default();
    s.active = Some(AiConversation {
        id: "trace-session".into(),
        messages: vec![answer],
        ..Default::default()
    });

    let collapsed = layout(&s, AREA);
    assert_eq!(collapsed.messages[0].trace.len(), 3);
    assert_eq!(collapsed.messages[0].trace_height, TRACE_HEADER_H);
    assert!(collapsed.rect_of(Hit::TraceToggle(0)).is_some());
    assert!(collapsed.messages[0].metadata.contains("fixture-model"));
    assert!(collapsed.messages[0].metadata.contains("42 tokens"));
    assert!(collapsed.messages[0].metadata.contains("已停止"));

    s.expanded_traces.insert(0);
    let expanded = layout(&s, AREA);
    assert!(expanded.messages[0].trace_height > collapsed.messages[0].trace_height);
    let step = expanded
        .rect_of(Hit::TraceStep(0, 1))
        .expect("thinking row hit");
    assert_eq!(
        expanded.hit(
            (step.left + step.right) / 2.0,
            (step.top + step.bottom) / 2.0
        ),
        Some(Hit::TraceStep(0, 1))
    );
    s.expanded_trace_steps.insert((0, 1));
    let detail = layout(&s, AREA);
    assert!(detail.messages[0].trace_height >= expanded.messages[0].trace_height);

    let mut list = DrawList::new();
    paint(
        &mut list,
        AREA,
        &s,
        &detail,
        false,
        theme::tokens().palette(false),
    );
    let drawn = texts(&list);
    assert!(drawn.iter().any(|text| text.contains("思考")), "{drawn:?}");
    assert!(
        drawn.iter().any(|text| text.contains("file_read")),
        "{drawn:?}"
    );
    assert!(
        drawn.iter().any(|text| text.contains("已停止")),
        "{drawn:?}"
    );
    assert!(list.finish().is_ok());
}

#[test]
fn scrolled_trace_selection_pending_and_streaming_hits_follow_paint_origin() {
    let selection = context::selection_from_document(
        "notes.md",
        Some("Notes"),
        "first line\nsecond line",
        (0, 10),
    )
    .unwrap()
    .to_value();
    let mut user = AiStoredMessage::new("user", "请检查这段");
    user.set("selectionContexts", serde_json::json!([selection]));

    let trace = vec![
        serde_json::json!({
            "kind": "plan",
            "steps": [{"title": "先阅读上下文并给出结论", "status": "done"}]
        }),
        serde_json::json!({
            "kind": "thinking",
            "text": "这是一段足够长的思考，用来确认滚动后行命中仍与绘制位置一致。"
        }),
        serde_json::json!({
            "kind": "tool",
            "name": "file_read",
            "ok": true,
            "summary": "读取 notes.md"
        }),
    ];
    let mut traced = AiStoredMessage::new("assistant", "历史回答");
    traced.set("trace", serde_json::Value::Array(trace));

    let mut pending = AiStoredMessage::new("assistant", "需要审批");
    pending.set(
        "pendingFileOperation",
        serde_json::json!({
            "id": "operation-1",
            "operationId": "operation-1",
            "path": "notes.md",
            "summary": "写入整理后的内容",
            "status": "pending"
        }),
    );

    let mut s = State::default();
    s.active = Some(AiConversation {
        id: "scroll-geometry".into(),
        messages: vec![user, traced, pending],
        ..Default::default()
    });
    s.expanded_traces.insert(1);
    s.streaming = Some(Streaming {
        reasoning: "流式思考也必须跟随消息滚动坐标。".into(),
        content: "正在生成".into(),
        ..Default::default()
    });

    let area = AREA;
    let at_top = layout(&s, area);
    s.scroll = 37.0;
    let scrolled = layout(&s, area);
    let controls = [
        Hit::TraceToggle(1),
        Hit::TraceStep(1, 1),
        Hit::SelectionLocate(0, 0),
        Hit::PendingCardReview(2),
        Hit::StreamingReasoningToggle,
    ];
    for hit in controls {
        let before = at_top.rect_of(hit).expect("control at top");
        let after = scrolled.rect_of(hit).expect("control after scroll");
        assert!(
            (after.top - (before.top - 37.0)).abs() < 0.01,
            "{hit:?}: {before:?} -> {after:?}"
        );
        assert_eq!(
            scrolled.hit(
                (after.left + after.right) / 2.0,
                (after.top + after.bottom) / 2.0
            ),
            Some(hit),
            "scroll hit mismatch for {hit:?}"
        );
    }
}

#[test]
fn legacy_streaming_trace_does_not_duplicate_reasoning_or_pending_tools() {
    let mut s = State::default();
    s.streaming = Some(Streaming {
        model: "live-model".into(),
        usage: Some(serde_json::json!({"totalTokens": 7})),
        reasoning: "只显示一次的实时推理".into(),
        tools: vec![
            ("pending_tool".into(), None, Some("正在执行".into())),
            ("finished_tool".into(), Some(true), Some("已完成".into())),
        ],
        ..Default::default()
    });
    let lay = layout(&s, AREA);
    let message = &lay.messages[0];
    assert_eq!(message.trace.len(), 1);
    assert_eq!(message.trace[0]["name"], "finished_tool");
    assert_eq!(
        message.active_tool.as_ref().map(|(name, _)| name.as_str()),
        Some("pending_tool")
    );
    assert_eq!(
        message.streaming_reasoning.as_deref(),
        Some("只显示一次的实时推理")
    );
    assert_eq!(message.metadata, "live-model · 7 tokens");
    let mut list = DrawList::new();
    paint(
        &mut list,
        AREA,
        &s,
        &lay,
        false,
        theme::tokens().palette(false),
    );
    let drawn = texts(&list);
    assert_eq!(
        drawn
            .iter()
            .filter(|text| text.contains("只显示一次的实时推理"))
            .count(),
        1
    );
    assert!(drawn.iter().any(|text| text.contains("pending_tool")));
    assert!(drawn.iter().any(|text| text.contains("finished_tool")));
    assert!(list.finish().is_ok());
    s.streaming
        .as_mut()
        .unwrap()
        .trace
        .push(AgentTraceStep::Thinking {
            text: "上一轮已经完成的推理".into(),
            duration_ms: Some(100),
        });
    let next_round = layout(&s, AREA);
    assert_eq!(
        next_round.messages[0].streaming_reasoning.as_deref(),
        Some("只显示一次的实时推理"),
        "previous-round trace must not hide current-round reasoning"
    );
    let mut list = DrawList::new();
    paint(
        &mut list,
        AREA,
        &s,
        &next_round,
        false,
        theme::tokens().palette(false),
    );
    let drawn = texts(&list);
    assert!(drawn
        .iter()
        .any(|text| text.contains("上一轮已经完成的推理")));
    assert_eq!(
        drawn
            .iter()
            .filter(|text| text.contains("只显示一次的实时推理"))
            .count(),
        1
    );
}

#[test]
fn search_navigation_and_narrow_header_are_geometry_safe() {
    let mut s = State::default();
    s.active = Some(AiConversation {
        id: "search-session".into(),
        messages: vec![
            AiStoredMessage::new("user", "alpha question"),
            AiStoredMessage::new("assistant", "Beta answer"),
        ],
        ..Default::default()
    });
    s.search_open = true;
    s.search_query.set_text("beta");
    let wide = layout(&s, AREA);
    assert_eq!(s.search_results(), vec![1]);
    let search_input = wide.rect_of(Hit::SearchInput).expect("narrow search input");
    assert!(search_input.width() < AREA.width());
    assert_eq!(
        wide.hit(search_input.left + 4.0, search_input.top + 4.0),
        Some(Hit::SearchInput)
    );
    assert!(wide.rect_of(Hit::SearchNext).is_some());
    assert!(wide.rect_of(Hit::SearchPrevious).is_some());
    assert!(wide.rect_of(Hit::SearchClose).is_some());

    let narrow = Rect::from_size(0.0, 0.0, 340.0, 650.0);
    let compact = layout(&State::default(), narrow);
    assert!(compact.rect_of(Hit::PreviousQuestion).is_none());
    assert!(compact.rect_of(Hit::SearchToggle).is_none());
    let actions = [
        Hit::Close,
        Hit::Float,
        Hit::Settings,
        Hit::Clear,
        Hit::NewSession,
    ]
    .into_iter()
    .map(|hit| compact.rect_of(hit).unwrap())
    .collect::<Vec<_>>();
    for (i, left) in actions.iter().enumerate() {
        for right in actions.iter().skip(i + 1) {
            assert!(
                left.intersect(right).is_empty(),
                "header actions overlap: {left:?} {right:?}"
            );
        }
    }
}

#[test]
fn standalone_navigation_reserves_chat_width_and_scroll_pill_is_visible() {
    let mut s = State::default();
    s.standalone = true;
    s.stick_to_bottom = false;
    let mut c = conversation();
    for _ in 0..40 {
        c.messages
            .push(AiStoredMessage::new("assistant", "长回复\n长回复\n长回复"));
    }
    s.active = Some(c);
    let area = Rect::from_size(0.0, 0.0, 1200.0, 800.0);
    let lay = layout(&s, area);
    assert_eq!(lay.navigation_rect.unwrap().left, area.right - NAV_W);
    assert!(lay.messages_rect.right < lay.navigation_rect.unwrap().left);
    assert!(lay.rect_of(Hit::NavigateMessage(0)).is_some());
    assert!(lay.navigation_max_scroll > 0.0);
    assert!(lay.rect_of(Hit::ScrollToBottom).is_some());
    let input = lay.rect_of(Hit::Input).unwrap();
    assert!(input.right <= lay.messages_rect.right);
    let mut list = DrawList::new();
    paint(
        &mut list,
        area,
        &s,
        &lay,
        false,
        theme::tokens().palette(false),
    );
    assert!(texts(&list).iter().any(|text| text == "对话导航"));
    assert!(texts(&list).iter().any(|text| text == "回到底部"));
    assert!(list.finish().is_ok());

    // 导航器可独立滚动：即使最后一条消息放不进固定标题行下方，
    // 也要能滚动到它。
    s.navigation_scroll = lay.navigation_max_scroll;
    let bottom = layout(&s, area);
    let last = bottom
        .rect_of(Hit::NavigateMessage(41))
        .expect("last navigation row");
    assert_eq!(
        bottom.hit(
            (last.left + last.right) / 2.0,
            (last.top + last.bottom) / 2.0
        ),
        Some(Hit::NavigateMessage(41))
    );
    s.scroll = bottom.max_scroll();
    assert!(
        layout(&s, area).rect_of(Hit::ScrollToBottom).is_none(),
        "the latest message's controls must not be covered by a redundant scroll button"
    );
}

#[test]
fn hovered_controls_explain_themselves() {
    let mut s = State::default();
    s.hover_hit = Some(Hit::SearchToggle);
    let lay = layout(&s, AREA);
    let mut list = DrawList::new();
    paint(
        &mut list,
        AREA,
        &s,
        &lay,
        false,
        theme::tokens().palette(false),
    );
    assert!(texts(&list).iter().any(|text| text == "搜索会话内容"));
    assert!(list.finish().is_ok());
}
