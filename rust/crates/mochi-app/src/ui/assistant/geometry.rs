//! 计算 AI 助手面板的布局和交互区域。
use super::*;

pub fn layout(s: &State, area: Rect) -> Layout {
    let message_spacing = super::message_spacing();
    let message_padding = super::message_padding();
    let mut out = Layout {
        session_id: s.active.as_ref().map(|c| c.id.clone()),
        ..Default::default()
    };
    if area.is_empty() {
        return out;
    }
    // 独立工作区里，会话导航器位于聊天区旁边。顶部栏保持全宽，
    // 但窗口宽到有余地时，给导航器预留一条稳定的窄栏。
    let navigation_left = if s.standalone && area.width() > 900.0 {
        area.right - NAV_W
    } else {
        area.right
    };
    let extra_margin = if s.standalone {
        ((navigation_left - area.left - 1040.0) / 2.0).max(12.0)
    } else {
        0.0
    };
    let chat_left = area.left + extra_margin;
    let chat_right = navigation_left - extra_margin;
    let chat_area = Rect::new(chat_left, area.top, chat_right, area.bottom);

    // ── 头部（56px；动作顺序与 Electron AIPanel 保持一致） ──
    let head = Rect::new(area.left, area.top, area.right, area.top + HEADER_H);
    out.entries.push((head, Hit::Header));
    let title = if s.standalone {
        "墨池AI"
    } else {
        "AI 助手"
    };
    let title_w = text::measure(title, TextStyle::Search) + 12.0;
    let conv = Rect::new(
        head.left + 16.0 + title_w,
        head.top + 12.0,
        head.left + 16.0 + title_w + 32.0,
        head.top + 44.0,
    );
    out.entries.push((conv, Hit::Conversations));
    let mut x = head.right - 16.0;
    let last_action = if s.standalone {
        Hit::AgentConfig
    } else {
        Hit::Close
    };
    let mut header_actions = if area.width() < 380.0 {
        vec![last_action, Hit::Settings, Hit::Clear, Hit::NewSession]
    } else if area.width() < 460.0 {
        vec![
            last_action,
            Hit::Settings,
            Hit::Clear,
            Hit::NewSession,
            Hit::SearchToggle,
        ]
    } else {
        vec![
            last_action,
            Hit::Settings,
            Hit::Clear,
            Hit::NewSession,
            Hit::SearchToggle,
            Hit::PreviousQuestion,
        ]
    };
    // 侧栏里的「悬浮窗」紧挨着关闭。独立的墨池 AI 页本身就是整页，不需要这个按钮。
    if !s.standalone {
        header_actions.insert(1, Hit::Float);
    }
    for hit in header_actions {
        let width = if s.standalone && area.width() >= 760.0 {
            match hit {
                Hit::AgentConfig => 110.0,
                Hit::Settings => 96.0,
                _ => 32.0,
            }
        } else {
            32.0
        };
        out.entries.push((
            Rect::new(x - width, head.top + 12.0, x, head.top + 44.0),
            hit,
        ));
        x -= width + 8.0;
    }

    // 搜索是独立的 48px 行，与 Web 面板完全一致。输入框/工具栏
    // 仍固定在下方，正文只是从更低处开始。
    let search_bar = if s.search_open {
        let bar = Rect::new(chat_left, head.bottom, chat_right, head.bottom + 48.0);
        let close = Rect::new(
            bar.right - 16.0 - 24.0,
            bar.top + 12.0,
            bar.right - 16.0,
            bar.top + 36.0,
        );
        let next = Rect::new(
            close.left - 28.0,
            bar.top + 12.0,
            close.left - 4.0,
            bar.top + 36.0,
        );
        let previous = Rect::new(
            next.left - 28.0,
            bar.top + 12.0,
            next.left - 4.0,
            bar.top + 36.0,
        );
        let input = Rect::new(
            bar.left + 16.0,
            bar.top + 8.0,
            previous.left - 8.0,
            bar.bottom - 8.0,
        );
        out.entries.push((bar, Hit::SearchInput));
        out.entries.push((input, Hit::SearchInput));
        out.entries.push((previous, Hit::SearchPrevious));
        out.entries.push((next, Hit::SearchNext));
        out.entries.push((close, Hit::SearchClose));
        Some(bar)
    } else {
        None
    };
    let body_top = search_bar.map_or(head.bottom, |bar| bar.bottom);

    // ── 输入区（待发送附件、编辑器、发送/停止按钮、底部集成工具栏） ──
    let selection_h = pending_selection_height(s, chat_area);
    let images_h = pending_height(s, chat_area);
    let files_h = crate::ui::ai_images::files_height(s, chat_area);
    let pending = selection_h + images_h + files_h;
    let card_w = (chat_area.width() - INPUT_PAD_X * 2.0).max(80.0);
    let input_w = (card_w - 24.0).max(40.0);
    let text_h = composer_text_height(s, input_w);
    let pending_gap = if pending > 0.0 { 6.0 } else { 0.0 };
    let card_h = 8.0 + pending + pending_gap + text_h + 4.0 + COMPOSER_FOOTER_H + 6.0;
    let card_bottom = area.bottom - INPUT_PAD_BOTTOM;
    let card_top = card_bottom - card_h;
    let wrapper = Rect::new(
        chat_left + INPUT_PAD_X,
        card_top,
        chat_right - INPUT_PAD_X,
        card_bottom,
    );
    out.composer_rect = Some(wrapper);
    // 命中测试按逆序遍历条目：输入框背景要放在附件控件下面，
    // 它们的打开/移除/滚动操作才保持可点。
    out.entries.push((wrapper, Hit::Input));

    let mut cur_y = wrapper.top + 8.0;
    if selection_h > 0.0 {
        let selection_area = Rect::new(wrapper.left + 10.0, 0.0, wrapper.right - 10.0, 0.0);
        let selection_layout =
            context::layout_selection_chips(&s.pending_selections, selection_area, cur_y, true);
        for chip in &selection_layout.chips {
            out.entries
                .push((chip.rect, Hit::PendingSelectionLocate(chip.index)));
            if let Some(remove) = chip.remove_rect {
                out.entries
                    .push((remove, Hit::PendingSelectionRemove(chip.index)));
            }
        }
        cur_y += selection_h;
    }
    if images_h > 0.0 {
        let image_top = cur_y;
        for (i, r) in pending_rects(s, chat_area, image_top)
            .into_iter()
            .enumerate()
        {
            out.entries.push((
                Rect::from_size(r.right - 20.0, r.top + 2.0, 18.0, 18.0),
                Hit::RemoveImage(i),
            ));
        }
        cur_y += images_h;
    }
    if !s.pending_files.is_empty() {
        let top = cur_y;
        let clip = Rect::new(
            wrapper.left + 26.0,
            top + 10.0,
            wrapper.right - 26.0,
            top + 46.0,
        );
        out.files_viewport = Some(clip);
        out.files_max_scroll = crate::ui::ai_images::files_max_scroll(s, chat_area);
        for (i, r) in crate::ui::ai_images::file_rects(s, chat_area, top)
            .into_iter()
            .enumerate()
        {
            for (r, hit) in [
                (
                    Rect::new(r.left, r.top, r.right - 24.0, r.bottom),
                    Hit::OpenFile(i),
                ),
                (
                    Rect::new(r.right - 24.0, r.top, r.right, r.bottom),
                    Hit::RemoveFile(i),
                ),
            ] {
                let visible = r.intersect(&clip);
                if visible.width() > 0.0 {
                    out.entries.push((visible, hit));
                }
            }
        }
        out.entries.push((
            Rect::new(wrapper.left + 4.0, top + 10.0, clip.left, top + 46.0),
            Hit::FilesBack,
        ));
        out.entries.push((
            Rect::new(clip.right, top + 10.0, wrapper.right - 4.0, top + 46.0),
            Hit::FilesForward,
        ));
        cur_y += files_h;
    }
    if pending > 0.0 {
        cur_y += 6.0;
    }

    let input_rect = Rect::new(
        wrapper.left + 12.0,
        cur_y,
        wrapper.right - 12.0,
        cur_y + text_h,
    );
    out.entries.push((input_rect, Hit::Input));

    // 底部工具栏
    let footer_top = input_rect.bottom + 4.0;
    let footer_bottom = footer_top + 28.0;
    let footer_left = wrapper.left + 8.0;
    let footer_right = wrapper.right - 8.0;

    let send_size = 28.0;
    let send_rect = Rect::new(
        footer_right - send_size,
        footer_top,
        footer_right,
        footer_top + send_size,
    );
    out.entries.push((send_rect, Hit::Send));
    let max_left = send_rect.left - 6.0;

    let mut cur_x = footer_left;
    let name = s.agent_name.as_str();
    let name_w = crate::ui::text::measure(name, TextStyle::Caption);
    let agent_w = (name_w + 30.0).clamp(64.0, 110.0);
    if cur_x + agent_w <= max_left {
        out.entries.push((
            Rect::new(cur_x, footer_top, cur_x + agent_w, footer_bottom),
            Hit::AgentPicker,
        ));
        cur_x += agent_w + 6.0;
    }

    let approve_w = 36.0;
    let auto_w = 56.0;
    if cur_x + approve_w + auto_w <= max_left {
        out.entries.push((
            Rect::new(
                cur_x,
                footer_top + 1.0,
                cur_x + approve_w,
                footer_bottom - 1.0,
            ),
            Hit::ApproveMode,
        ));
        out.entries.push((
            Rect::new(
                cur_x + approve_w,
                footer_top + 1.0,
                cur_x + approve_w + auto_w,
                footer_bottom - 1.0,
            ),
            Hit::AutomaticMode,
        ));
        cur_x += approve_w + auto_w + 6.0;
    }

    let attach_w = 26.0;
    if cur_x + attach_w <= max_left {
        out.entries.push((
            Rect::new(
                cur_x,
                footer_top + 1.0,
                cur_x + attach_w,
                footer_bottom - 1.0,
            ),
            Hit::AttachFiles,
        ));
        cur_x += attach_w + 6.0;
    }

    let mount_w = 64.0;
    if cur_x + mount_w <= max_left {
        out.entries.push((
            Rect::new(max_left - mount_w, footer_top, max_left, footer_bottom),
            Hit::MountSession,
        ));
    } else if cur_x + 28.0 <= max_left {
        out.entries.push((
            Rect::new(max_left - 28.0, footer_top, max_left, footer_bottom),
            Hit::MountSession,
        ));
    }

    // ── 消息区 ──
    let error_h = if s.error.is_empty() { 0.0 } else { 24.0 };
    let messages_bottom = (wrapper.top - error_h - 8.0).max(body_top);
    let messages = Rect::new(chat_left, body_top, chat_right, messages_bottom);
    out.messages_rect = messages;
    out.entries.insert(0, (messages, Hit::Messages));
    let width = (messages.width() - MSG_PAD_X * 2.0).max(1.0);
    let mut y = MSG_PAD_Y;
    let visible = s.visible_messages();
    let records = s
        .active
        .iter()
        .flat_map(|c| c.messages.iter())
        .filter(|m| mochi_core::ai::locator::visible(m))
        .collect::<Vec<_>>();
    let n = visible.len() + usize::from(s.streaming.is_some());

    for (i, (role, fallback_content)) in visible.iter().enumerate() {
        let record = records.get(i).copied();
        let content = record
            .map(|message| message.content().to_owned())
            .unwrap_or_else(|| fallback_content.clone());
        let is_user = role == "user";
        let selection_values = record
            .map(context::selection_values_from_message)
            .unwrap_or_default();
        let refs = record
            .and_then(|m| m.get("images"))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let body_width = if is_user {
            let max_width = (width * 0.80).max(40.0).min(width);
            if selection_values.is_empty() && refs.is_empty() {
                user_message_body_width(&content, max_width, s.standalone)
            } else {
                max_width
            }
        } else {
            width
        };
        let body_left = if is_user {
            (messages.right - MSG_PAD_X - body_width - 12.0).max(messages.left + MSG_PAD_X)
        } else {
            messages.left + MSG_PAD_X
        };
        let trace = record.map(persisted_trace).unwrap_or_default();
        let trace_expanded = s.expanded_traces.contains(&i);
        let trace_h = if is_user {
            0.0
        } else {
            trace_height(
                &trace,
                body_width,
                trace_expanded,
                &s.expanded_trace_steps,
                i,
            )
        };
        let trace_top = y + ROLE_H + 8.0;
        let selection_top = trace_top + trace_h + if trace_h > 0.0 { 8.0 } else { 0.0 };
        let selection_chips = context::layout_selection_chips(
            &selection_values,
            Rect::new(body_left, 0.0, body_left + body_width, 0.0),
            selection_top,
            false,
        );
        let selection_h = selection_chips.height;
        let body_top = selection_top + selection_h + if selection_h > 0.0 { 8.0 } else { 0.0 };
        let body = crate::ui::ai_markdown::layout(&content, body_width, s.standalone);
        let (images, image_h) =
            image_layout(&refs, &s.image_previews, body_width, body.height + 8.0);
        let body_h = body.height + image_h;
        let pending_edit = record
            .and_then(|m| m.get("pendingEdit"))
            .filter(|edit| edit.is_object())
            .cloned();
        let pending_top = body_top + body_h + 8.0;
        let pending_edit_h = pending_edit.as_ref().map_or(0.0, |_| PENDING_EDIT_CARD_H);
        let pending_card = record
            .and_then(context::pending_card_from_message)
            .filter(|card| card.field != "pendingEdit");
        let pending_card_top = pending_top
            + pending_edit_h
            + if pending_edit_h > 0.0 && pending_card.is_some() {
                8.0
            } else {
                0.0
            };
        let pending_card_height = pending_card
            .as_ref()
            .map_or(0.0, |card| context::pending_card_height(card, body_width));
        let pending_h = pending_edit_h
            + pending_card_height
            + if pending_edit_h > 0.0 && pending_card.is_some() {
                8.0
            } else {
                0.0
            };
        let target = record
            .filter(|m| mochi_core::ai::message_ops::actionable(m))
            .and_then(|m| Some((s.active.as_ref()?.id.clone(), m.id()?.to_owned())));
        let actions_top = if pending_h > 0.0 {
            pending_top + pending_h + 2.0
        } else {
            body_top + body_h + 2.0
        };
        let actions_h = target.as_ref().map_or(0.0, |_| 40.0);
        let interrupted = record
            .and_then(|m| m.get("interrupted"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let last_assistant_idx = records
            .iter()
            .rposition(|m| m.role() == "assistant" && !m.is_hidden());
        let can_show_follow_up = Some(i) == last_assistant_idx
            && s.streaming.is_none()
            && s.follow_up_frequency != mochi_core::ai::FollowUpFrequency::Never
            && !interrupted;
        let follow_up = if can_show_follow_up {
            record
                .and_then(|m| mochi_core::ai::follow_up::get_message_follow_up(m))
                .filter(|st| {
                    st.status == mochi_core::ai::FollowUpStatus::Ready
                        && (3..=5).contains(&st.suggestions.len())
                        && record.and_then(|m| m.id()).is_some_and(|id| {
                            st.response_revision
                                == mochi_core::ai::compute_response_revision(id, &content)
                        })
                })
        } else {
            None
        };
        let laid_follow_up = follow_up.map(|st| {
            let follow_up_top = if pending_h > 0.0 {
                pending_top + pending_h + actions_h + 4.0
            } else {
                body_top + body_h + actions_h + 4.0
            };
            let inner_w = (body_width - 24.0).max(40.0);
            let mut cur_x = 0.0f32;
            let mut cur_y = 0.0f32;
            let mut row_h = 0.0f32;
            let mut buttons = Vec::new();
            for sug in &st.suggestions {
                let q_text = &sug.text;
                let single_w = text::measure(q_text, TextStyle::Small) + 20.0;
                let (btn_w, btn_h) = if single_w <= inner_w {
                    (single_w, 28.0f32)
                } else {
                    let lines_h =
                        wrapped_height(q_text, (inner_w - 20.0).max(20.0), TextStyle::Small, None);
                    (inner_w, lines_h + 10.0)
                };
                if cur_x > 0.0 && cur_x + btn_w > inner_w {
                    cur_x = 0.0;
                    cur_y += row_h + 6.0;
                    row_h = 0.0;
                }
                let btn_rect = Rect::new(cur_x, cur_y, cur_x + btn_w, cur_y + btn_h);
                buttons.push((btn_rect, q_text.clone()));
                cur_x += btn_w + 8.0;
                row_h = row_h.max(btn_h);
            }
            let total_btns_h = cur_y + row_h;
            let follow_up_h = 24.0 + 4.0 + total_btns_h + 10.0;
            LaidFollowUp {
                top: follow_up_top,
                height: follow_up_h,
                buttons,
            }
        });
        if let Some(fu) = &laid_follow_up {
            for (q_idx, (btn_rect, _)) in fu.buttons.iter().enumerate() {
                let btn_v = Rect::new(
                    body_left + 12.0 + btn_rect.left,
                    messages.top + fu.top + 26.0 + btn_rect.top - s.scroll,
                    body_left + 12.0 + btn_rect.right,
                    messages.top + fu.top + 26.0 + btn_rect.bottom - s.scroll,
                );
                out.entries.push((btn_v, Hit::FollowUpQuestion(i, q_idx)));
            }
        }
        let content_end = if let Some(fu) = &laid_follow_up {
            fu.top + fu.height
        } else if pending_h > 0.0 {
            pending_top + pending_h + actions_h
        } else {
            body_top + body_h + actions_h
        };
        let last = i + 1 == n;
        let height = (content_end - y) + if last { 0.0 } else { 20.0 + 4.0 + 1.0 };
        let bubble = is_user.then(|| {
            Rect::new(
                body_left - message_padding,
                selection_top - message_padding,
                (body_left + body_width + message_padding).min(messages.right - MSG_PAD_X),
                body_top + body_h + message_padding,
            )
        });
        let scroll_key = record
            .and_then(|m| m.id())
            .map(|id| format!("id:{id}"))
            .unwrap_or_else(|| format!("index:{i}"));
        let horizontal = s.horizontal.offsets(&out.session_id, &scroll_key);
        let metadata = record
            .and_then(|m| {
                let model = m
                    .get("model")
                    .and_then(|v| v.as_str())
                    .filter(|v| !v.is_empty());
                let interrupted = m
                    .get("interrupted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let usage = m.get("usage").and_then(|v| {
                    v.get("totalTokens")
                        .or_else(|| v.get("total_tokens"))
                        .and_then(|v| v.as_u64())
                });
                let time = m
                    .timestamp()
                    .filter(|value| *value > 0)
                    .map(crate::ui::panels::format_time);
                let mut parts = Vec::new();
                if let Some(model) = model {
                    parts.push(model.to_owned());
                }
                if let Some(usage) = usage {
                    parts.push(format!("{usage} tokens"));
                }
                if let Some(time) = time.filter(|value| !value.is_empty()) {
                    parts.push(time);
                }
                if interrupted {
                    parts.push("已停止".into());
                }
                (!parts.is_empty()).then(|| parts.join(" · "))
            })
            .unwrap_or_default();
        let trace_len = trace.len();
        out.messages.push(LaidMessage {
            images,
            scroll_key,
            horizontal,
            source: content,
            action_target: target,
            actions_top,
            role: role.clone(),
            body,
            trace,
            trace_expanded,
            trace_top,
            trace_height: trace_h,
            activity_height: 0.0,
            streaming_reasoning: None,
            active_tool: None,
            live_plan: None,
            streaming_status: None,
            selection_values,
            selection_chips,
            selection_top,
            pending_card,
            pending_card_top,
            pending_card_height,
            body_top,
            body_left,
            body_width,
            bubble,
            metadata,
            tools: Vec::new(),
            pending_edit,
            pending_top,
            follow_up: laid_follow_up,
            top: y,
            height,
            streaming: false,
        });
        if trace_len > 0 && !is_user {
            out.entries.push((
                Rect::new(
                    body_left,
                    messages.top + trace_top - s.scroll,
                    body_left + body_width,
                    messages.top + trace_top + TRACE_HEADER_H - s.scroll,
                ),
                Hit::TraceToggle(i),
            ));
            if trace_expanded {
                let row_width = (body_width - TRACE_PAD_X * 2.0).max(1.0);
                let mut trace_y =
                    messages.top + trace_top + TRACE_HEADER_H + TRACE_PAD_Y - s.scroll;
                for (step_index, value) in out.messages.last().unwrap().trace.iter().enumerate() {
                    let h = match trace_kind(value) {
                        Some("plan") => plan_height(value, row_width),
                        Some("thinking") => thinking_height(
                            value,
                            row_width,
                            s.expanded_trace_steps.contains(&(i, step_index)),
                        ),
                        Some("tool") => tool_height(
                            value,
                            row_width,
                            s.expanded_trace_steps.contains(&(i, step_index)),
                        ),
                        _ => 0.0,
                    };
                    if h <= 0.0 {
                        continue;
                    }
                    if matches!(trace_kind(value), Some("thinking" | "tool")) {
                        let row = Rect::new(
                            body_left + TRACE_PAD_X,
                            trace_y,
                            body_left + body_width - TRACE_PAD_X,
                            trace_y + h,
                        );
                        out.entries.push((row, Hit::TraceStep(i, step_index)));
                    }
                    trace_y += h + TRACE_ROW_GAP;
                }
            }
        }
        for chip in &out.messages.last().unwrap().selection_chips.chips {
            let rect = Rect::new(
                chip.rect.left,
                messages.top + chip.rect.top - s.scroll,
                chip.rect.right,
                messages.top + chip.rect.bottom - s.scroll,
            );
            out.entries
                .push((rect, Hit::SelectionLocate(i, chip.index)));
        }
        if let Some(card) = out
            .messages
            .last()
            .and_then(|message| message.pending_card.as_ref())
        {
            if pending_card_height > 0.0 {
                let card_rect = Rect::new(
                    body_left,
                    messages.top + pending_card_top - s.scroll,
                    body_left + body_width,
                    messages.top + pending_card_top + pending_card_height - s.scroll,
                );
                if let Some(review) = context::pending_card_review_rect(card, card_rect) {
                    out.entries.push((review, Hit::PendingCardReview(i)));
                }
            }
        }
        y += height + message_spacing;
    }

    if let Some(st) = &s.streaming {
        let trace = legacy_streaming_trace(st);
        let live_plan = trace
            .iter()
            .rev()
            .find(|value| trace_kind(value) == Some("plan"))
            .cloned();
        let active_tool = st
            .tools
            .iter()
            .rev()
            .find(|(_, ok, _)| ok.is_none())
            .map(|(name, _, summary)| (name.clone(), summary.clone()));
        // Thinking 轨迹结束时运行时会清空实时缓冲。
        // 后续一轮模型可能既有旧轨迹步骤又有新的推理内容，
        // 所以整轮期间都要保持可见。
        let reasoning = (!st.reasoning.is_empty()).then(|| st.reasoning.clone());
        let mut content = st.content.clone();
        let body_width = width;
        let body_left = messages.left + MSG_PAD_X;
        let activity_h = activity_height(
            &trace,
            reasoning.as_deref(),
            active_tool.as_ref(),
            live_plan.as_ref(),
            body_width,
            s.streaming_reasoning_expanded,
            (!st.status.is_empty()).then_some(st.status.as_str()),
        );
        if content.is_empty() && activity_h <= 0.0 && !st.status.is_empty() {
            content = st.status.clone();
        }
        let body = crate::ui::ai_markdown::layout(&content, body_width, s.standalone);
        let body_top = y + ROLE_H + 8.0 + activity_h + if activity_h > 0.0 { 8.0 } else { 0.0 };
        let body_h = body.height;
        let content_end = body_top + body_h;
        let height = (content_end - y).max(ROLE_H + 8.0 + activity_h);
        let mut tools = st
            .tools
            .iter()
            .map(|(name, ok, summary)| {
                let mark = match ok {
                    None => "…",
                    Some(true) => "✓",
                    Some(false) => "✗",
                };
                summary.as_ref().map_or_else(
                    || format!("{mark} {name}"),
                    |summary| format!("{mark} {name} · {summary}"),
                )
            })
            .collect::<Vec<_>>();
        tools.splice(0..0, st.plan.clone());
        let scroll_key = "streaming".to_owned();
        let horizontal = s.horizontal.offsets(&out.session_id, &scroll_key);
        out.messages.push(LaidMessage {
            images: Vec::new(),
            scroll_key,
            horizontal,
            source: content,
            action_target: None,
            actions_top: 0.0,
            role: "assistant".into(),
            body,
            trace: trace.clone(),
            trace_expanded: true,
            trace_top: y + ROLE_H + 8.0,
            trace_height: 0.0,
            activity_height: activity_h,
            streaming_reasoning: reasoning,
            active_tool,
            live_plan,
            streaming_status: (!st.status.is_empty()).then(|| {
                let base = st.status.trim_end_matches(['.', '。', '…']);
                format!("{base}{}", ".".repeat(st.animation_phase.max(1) as usize))
            }),
            selection_values: Vec::new(),
            selection_chips: context::SelectionChipsLayout::default(),
            selection_top: 0.0,
            pending_card: None,
            pending_card_top: 0.0,
            pending_card_height: 0.0,
            body_top,
            body_left,
            body_width,
            bubble: None,
            metadata: {
                let mut parts = Vec::new();
                if !st.model.is_empty() {
                    parts.push(st.model.clone());
                }
                if let Some(total) = st.usage.as_ref().and_then(|usage| {
                    usage
                        .get("totalTokens")
                        .or_else(|| usage.get("total_tokens"))
                        .and_then(serde_json::Value::as_u64)
                }) {
                    parts.push(format!("{total} tokens"));
                }
                parts.join(" · ")
            },
            tools,
            pending_edit: None,
            pending_top: 0.0,
            follow_up: None,
            top: y,
            height,
            streaming: true,
        });
        let has_streaming_reasoning = out.messages.last().is_some_and(|message| {
            message.streaming_reasoning.is_some()
                || message
                    .trace
                    .iter()
                    .any(|value| trace_kind(value) == Some("thinking"))
        });
        if has_streaming_reasoning {
            let activity_top = messages.top + y + ROLE_H + 8.0 - s.scroll;
            let row = Rect::new(
                body_left,
                activity_top,
                body_left + body_width,
                activity_top + activity_h,
            );
            out.entries.push((row, Hit::StreamingReasoningToggle));
        }
        y += height + message_spacing;
    }
    out.content_height = if out.messages.is_empty() {
        0.0
    } else {
        y - message_spacing + MSG_PAD_Y
    };
    if s.provider_missing && out.messages.is_empty() {
        let cy = messages.top + messages.height() / 2.0;
        out.entries.push((
            Rect::new(
                messages.left + messages.width() / 2.0 - 48.0,
                cy + 24.0,
                messages.left + messages.width() / 2.0 + 48.0,
                cy + 52.0,
            ),
            Hit::OpenSettings,
        ));
    }
    if out.max_scroll() > 0.0 && !s.stick_to_bottom && s.scroll < out.max_scroll() - 1.0 {
        let r = Rect::new(
            messages.right - SCROLL_BOTTOM_W - 16.0,
            messages.bottom - SCROLL_BOTTOM_H - 12.0,
            messages.right - 16.0,
            messages.bottom - 12.0,
        );
        out.entries.push((r, Hit::ScrollToBottom));
    }

    // ── 会话弹层 ──
    if s.show_conversations {
        let sessions = s.sorted_sessions();
        let rows = sessions.len().clamp(1, 8);
        let pop = Rect::new(
            conv.left,
            conv.bottom + 4.0,
            (conv.left + 320.0).min(chat_right - 8.0),
            (conv.bottom + 4.0 + 40.0 + rows as f32 * 48.0 + 8.0)
                .min(area.bottom - 8.0)
                .max(conv.bottom + 48.0),
        );
        out.popover = Some(pop);
        out.entries.push((pop, Hit::PopoverInside));
        out.entries.push((
            Rect::new(
                pop.right - 8.0 - 28.0,
                pop.top + 6.0,
                pop.right - 8.0,
                pop.top + 34.0,
            ),
            Hit::PopoverNew,
        ));
        let viewport = Rect::new(
            pop.left + 4.0,
            pop.top + 44.0,
            pop.right - 4.0,
            pop.bottom - 4.0,
        );
        out.conversations_viewport = Some(viewport);
        out.conversations_max_scroll = (sessions.len() as f32 * 48.0 - viewport.height()).max(0.0);
        let scroll = s
            .conversations_scroll
            .clamp(0.0, out.conversations_max_scroll);
        for i in 0..sessions.len() {
            let sy = viewport.top + i as f32 * 48.0 - scroll;
            let r = Rect::new(viewport.left, sy, viewport.right - 6.0, sy + 48.0);
            let visible = r.intersect(&viewport);
            if visible.height() <= 0.0 {
                continue;
            }
            out.conversation_rows.push((i, r));
            out.entries.push((visible, Hit::Session(i)));
            let delete = Rect::new(
                r.right - 8.0 - 24.0,
                r.top + 12.0,
                r.right - 8.0,
                r.top + 36.0,
            )
            .intersect(&viewport);
            if delete.height() > 0.0 {
                out.entries.push((delete, Hit::SessionDelete(i)));
            }
        }
    }
    if s.standalone && area.width() > 900.0 {
        let nav = Rect::new(navigation_left, body_top, area.right, area.bottom);
        out.navigation_rect = Some(nav);
        let navigation_viewport = Rect::new(nav.left, nav.top + 48.0, nav.right, nav.bottom);
        let navigation_items = visible
            .iter()
            .filter(|(role, _)| matches!(role.as_str(), "user" | "assistant"))
            .count();
        let navigation_content_height = navigation_items as f32 * 40.0;
        out.navigation_max_scroll =
            (navigation_content_height - navigation_viewport.height().max(0.0)).max(0.0);
        let navigation_scroll = s.navigation_scroll.clamp(0.0, out.navigation_max_scroll);
        let mut ny = navigation_viewport.top - navigation_scroll;
        out.entries.push((
            Rect::new(nav.left, nav.top, nav.right, nav.bottom),
            Hit::Header,
        ));
        for (index, (role, _content)) in visible.iter().enumerate() {
            if !matches!(role.as_str(), "user" | "assistant") {
                continue;
            }
            let row = Rect::new(nav.left + 10.0, ny, nav.right - 10.0, ny + 34.0);
            // 让导航条目与其绘制过程处于同一个裁剪坐标空间。
            // 视口外的条目仍是可滚动列表的一部分，滚动一格后即可被命中。
            let visible_row = row.intersect(&navigation_viewport);
            if visible_row.width() > 0.0 && visible_row.height() > 0.0 {
                out.entries.push((visible_row, Hit::NavigateMessage(index)));
            }
            ny += 40.0;
        }
    }
    out
}
