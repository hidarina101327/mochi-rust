//! 绘制 AI 助手面板及其消息内容。
use super::*;

pub fn paint(list: &mut DrawList, area: Rect, s: &State, lay: &Layout, focused: bool, p: &Palette) {
    if area.is_empty() {
        return;
    }
    list.push_clip(area);
    // ── 头部 ──
    let head = Rect::new(area.left, area.top, area.right, area.top + HEADER_H);
    list.text_run(
        Rect::new(head.left + 16.0, head.top, head.left + 200.0, head.bottom),
        if s.standalone {
            "墨池AI"
        } else {
            "AI 助手"
        },
        TextStyle::Search,
        p.foreground,
        Align::Leading,
        Emphasis::Bold,
    );
    let action = |list: &mut DrawList, hit: Hit, icon: Icon, active: bool| {
        if let Some(r) = lay.rect_of(hit) {
            let hovered = s.hover_hit == Some(hit);
            if active || hovered {
                list.rounded_rect(
                    r,
                    6.0,
                    if active {
                        theme::mix(p.accent, p.area_assistant_default, 0.10)
                    } else {
                        p.surface_muted
                    },
                );
            }
            let label = if r.width() > 40.0 {
                match hit {
                    Hit::Settings => Some("AI 设置"),
                    Hit::AgentConfig => Some("Agent配置"),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(label) = label {
                list.text(
                    Rect::new(r.left + 32.0, r.top, r.right - 8.0, r.bottom),
                    label,
                    TextStyle::Caption,
                    p.foreground,
                );
            }
            list.icon_centered(
                if label.is_some() {
                    Rect::new(r.left + 8.0, r.top, r.left + 24.0, r.bottom)
                } else {
                    r
                },
                icon,
                16.0,
                if active {
                    p.accent
                } else if hovered {
                    p.foreground
                } else {
                    p.muted
                },
            );
        }
    };
    action(
        list,
        Hit::Conversations,
        Icon::MESSAGE_SQUARE,
        s.show_conversations,
    );
    action(list, Hit::PreviousQuestion, Icon::ARROW_UP, false);
    action(list, Hit::SearchToggle, Icon::SEARCH, s.search_open);
    action(list, Hit::NewSession, Icon::PLUS, false);
    action(list, Hit::Clear, Icon::TRASH2, false);
    action(list, Hit::Settings, Icon::SETTINGS, false);
    action(list, Hit::AgentConfig, Icon::SPARKLES, false);
    action(list, Hit::Float, Icon::MAXIMIZE2, false);
    action(list, Hit::Close, Icon::X, false);
    list.border_bottom(head, p.border);

    if let Some(bar) = lay
        .entries
        .iter()
        .find(|(_, hit)| *hit == Hit::SearchInput)
        .map(|(rect, _)| *rect)
        .filter(|rect| rect.top >= head.bottom && rect.height() >= 40.0)
    {
        list.rect(bar, p.surface);
        list.hline(bar.left, bar.right, bar.bottom - 1.0, p.border);
        if let Some(input) = lay
            .entries
            .iter()
            .find(|(rect, hit)| *hit == Hit::SearchInput && rect.height() < 40.0)
            .map(|(rect, _)| *rect)
        {
            let mut query = s.search_query.clone();
            // 结果计数要留在输入框内，且不能让长查询/光标画到它底下。
            // FieldLook 的右内边距才是实际文本视口；外边框仍覆盖整个控件。
            let mut look = FieldLook::search(p);
            look.padding_right += 74.0;
            query.paint(list, input, s.search_focused, p, look);
            let results = s.search_results();
            let label = if query.text().trim().is_empty() {
                String::new()
            } else if results.is_empty() {
                "无匹配".into()
            } else {
                format!(
                    "{} / {}",
                    (s.search_result_index.max(0) as usize + 1).min(results.len()),
                    results.len()
                )
            };
            if !label.is_empty() {
                list.text_aligned(
                    Rect::new(
                        input.right - 74.0,
                        input.top,
                        input.right - 8.0,
                        input.bottom,
                    ),
                    label,
                    TextStyle::Tiny,
                    p.muted,
                    Align::Trailing,
                );
            }
        }
        for (hit, icon) in [
            (Hit::SearchPrevious, Icon::CHEVRON_UP),
            (Hit::SearchNext, Icon::CHEVRON_DOWN),
            (Hit::SearchClose, Icon::X),
        ] {
            action(list, hit, icon, false);
        }
    }

    // ── 消息 ──
    let mr = lay.messages_rect;
    list.push_clip(mr);
    if s.provider_missing && lay.messages.is_empty() {
        let cy = mr.top + mr.height() / 2.0;
        list.text_aligned(
            Rect::new(mr.left, cy - 40.0, mr.right, cy - 16.0),
            "还没有配置 AI 提供商",
            TextStyle::Search,
            p.muted,
            Align::Center,
        );
        list.text_aligned(
            Rect::new(mr.left, cy - 12.0, mr.right, cy + 8.0),
            "在设置 → AI 配置里填写接口地址、模型和密钥",
            TextStyle::Body,
            p.muted,
            Align::Center,
        );
        if let Some(r) = lay.rect_of(Hit::OpenSettings) {
            crate::ui::workspace_ui::button(
                list,
                r,
                "打开设置",
                None,
                true,
                s.hover_hit == Some(Hit::OpenSettings),
                p,
            );
        }
    } else if lay.messages.is_empty() {
        let cy = mr.top + mr.height() / 2.0;
        let title_top = cy - 38.0;
        let title_height = if s.standalone {
            TextStyle::Display.line_height().max(40.0)
        } else {
            40.0
        };
        let subtitle_top = (cy + 10.0).max(title_top + title_height + 8.0);
        list.text_aligned(
            Rect::new(
                mr.left + 16.0,
                title_top,
                mr.right - 16.0,
                title_top + title_height,
            ),
            if s.standalone {
                "一起理清思路"
            } else {
                "开始与 AI 对话"
            },
            if s.standalone {
                TextStyle::Display
            } else {
                TextStyle::Search
            },
            p.foreground,
            Align::Center,
        );
        list.text_aligned(
            Rect::new(mr.left, subtitle_top, mr.right, subtitle_top + 20.0),
            if s.standalone && mr.width() < 500.0 {
                "从一个问题或一篇笔记开始。"
            } else if s.standalone {
                "讨论想法、整理笔记，或把计划变成下一步行动。"
            } else {
                "Ctrl+J 显示或隐藏右侧面板"
            },
            TextStyle::Caption,
            p.muted,
            Align::Center,
        );
    } else {
        let lh = TextStyle::Ai {
            kind: 0,
            standalone: s.standalone,
        }
        .line_height();
        let located = s.located.as_ref().and_then(|id| {
            s.message_ids()
                .iter()
                .position(|value| value.as_ref() == Some(id))
        });
        let search_results = s.search_results();
        for (i, m) in lay.messages.iter().enumerate() {
            let top = mr.top + m.top - s.scroll;
            if top > mr.bottom || top + m.height < mr.top {
                continue;
            }
            let left = m.body_left;
            let right = m.body_left + m.body_width;
            let role_left = if m.role == "user" {
                left
            } else {
                mr.left + MSG_PAD_X
            };
            let role_right = right;
            if located == Some(i) || (located == Some(i + 1) && m.role == "user") {
                let rect = Rect::new(
                    mr.left + MSG_PAD_X - 8.0,
                    top - 4.0,
                    mr.right - MSG_PAD_X + 8.0,
                    top + m.height + 4.0,
                );
                list.rounded_rect(
                    rect,
                    8.0,
                    theme::mix(p.accent, p.area_assistant_default, 0.16),
                );
                list.rounded_border(
                    rect,
                    8.0,
                    theme::mix(p.accent, p.area_assistant_default, 0.38),
                );
            }
            if search_results.contains(&i) && !s.search_query.is_empty() {
                let rect = Rect::new(
                    left - 12.0,
                    top + ROLE_H + 2.0,
                    right + 12.0,
                    top + m.height + 3.0,
                );
                list.rounded_border(rect, 8.0, theme::mix(p.accent, p.surface, 0.65));
            }
            // assistant 头像与角色信息。用户消息保持干净的右对齐气泡，
            // 与 Electron 聊天界面轮廓一致。
            if m.role == "user" {
                list.text_run(
                    Rect::new(role_left, top, role_right, top + ROLE_H),
                    "你",
                    TextStyle::Caption,
                    p.muted,
                    Align::Trailing,
                    Emphasis::Bold,
                );
                if !m.metadata.is_empty() {
                    list.text_aligned(
                        Rect::new(role_left, top, role_right - 26.0, top + ROLE_H),
                        m.metadata.as_str(),
                        TextStyle::Tiny,
                        p.muted,
                        Align::Trailing,
                    );
                }
                if let Some(bubble) = m.bubble {
                    let b = Rect::new(
                        bubble.left,
                        mr.top + bubble.top - s.scroll,
                        bubble.right,
                        mr.top + bubble.bottom - s.scroll,
                    );
                    let bubble_color = super::user_bubble_color(p);
                    let radius = super::message_radius();
                    list.rounded_rect(b, radius, bubble_color);
                    list.rounded_border(b, radius, theme::mix(bubble_color, p.surface, 0.24));
                }
            } else {
                let avatar =
                    Rect::from_size(mr.left + MSG_PAD_X, top - 2.0, AVATAR_SIZE, AVATAR_SIZE);
                list.rounded_rect(avatar, 12.0, theme::mix(p.accent, p.surface, 0.14));
                list.icon_centered(avatar, Icon::BOT, 15.0, p.accent);
                list.text_run(
                    Rect::new(
                        role_left + AVATAR_SIZE + AVATAR_GAP,
                        top,
                        role_right,
                        top + ROLE_H,
                    ),
                    "AI",
                    TextStyle::Caption,
                    p.accent,
                    Align::Leading,
                    Emphasis::Bold,
                );
                if !m.metadata.is_empty() {
                    list.text(
                        Rect::new(
                            role_left + AVATAR_SIZE + AVATAR_GAP + 22.0,
                            top,
                            role_right,
                            top + ROLE_H,
                        ),
                        m.metadata.as_str(),
                        TextStyle::Tiny,
                        p.muted,
                    );
                }
            }
            let trace_top = mr.top + m.trace_top - s.scroll;
            if !m.streaming && m.trace_height > 0.0 {
                paint_trace_card(
                    list,
                    m,
                    Rect::new(left, trace_top, right, trace_top + m.trace_height),
                    s,
                    p,
                    i,
                );
            }
            if m.streaming && m.activity_height > 0.0 {
                paint_activity(
                    list,
                    m,
                    Rect::new(left, trace_top, right, trace_top + m.activity_height),
                    s,
                    p,
                );
            }
            if !m.selection_chips.chips.is_empty() {
                context::paint_selection_chips(
                    list,
                    &m.selection_values,
                    Rect::new(left, 0.0, right, 0.0),
                    mr.top + m.selection_top - s.scroll,
                    false,
                    p,
                );
            }
            let body_top = mr.top + m.body_top - s.scroll;
            let region = s
                .horizontal
                .hover
                .as_ref()
                .filter(|(key, _)| key == &m.scroll_key)
                .map(|(_, id)| *id);
            // 原生拖拽选区画在 Markdown run 底下，保证字形清晰，
            // 高亮也能跟随换行/带样式的文本。正文裁剪同时防止
            // 超出视口的选区盖住轨迹或输入工具栏。
            let body_clip = Rect::new(left, mr.top, right, mr.bottom);
            let selection_rects = s.selection_rects(lay, i, s.scroll);
            if !selection_rects.is_empty() {
                list.push_clip(body_clip);
                for rect in selection_rects {
                    list.rect_alpha(rect, p.accent, 0.28);
                }
                list.pop_clip();
            }
            m.body.paint_scrolled(
                list,
                (left, body_top),
                Rect::new(left, mr.top, right, mr.bottom),
                p,
                s.hover_content
                    .filter(|(message, _)| *message == i)
                    .map(|(_, action)| action),
                s.hover_content_hit,
                Some(&m.horizontal),
                region,
            );
            for (image_rect, reference) in &m.images {
                let r = Rect::new(
                    left + image_rect.left,
                    body_top + image_rect.top,
                    left + image_rect.right,
                    body_top + image_rect.bottom,
                );
                if let Some(preview) = s.image_previews.get(reference) {
                    list.image_thumbnail(r, &preview.path, "图片");
                } else {
                    list.text(r, "图片暂不可用", TextStyle::Small, p.muted);
                }
                list.rounded_border(r, 8.0, p.border);
            }
            if let Some(edit) = &m.pending_edit {
                let card_top = mr.top + m.pending_top - s.scroll;
                let card = Rect::new(left, card_top, right, card_top + PENDING_EDIT_CARD_H);
                let status = edit["status"].as_str().unwrap_or("pending");
                let summary = edit["summary"].as_str().unwrap_or("局部修改建议");
                let path = edit["path"].as_str().unwrap_or("目标文档");
                let start = edit["startLine"].as_u64().unwrap_or(0);
                let end = edit["endLine"].as_u64().unwrap_or(start);
                let radius = super::card_radius();
                list.rounded_rect(card, radius, theme::mix(p.surface_muted, p.surface, 0.5));
                list.rounded_border(card, radius, p.border);
                list.icon_centered(
                    Rect::new(
                        card.left + 10.0,
                        card.top + 9.0,
                        card.left + 26.0,
                        card.top + 25.0,
                    ),
                    Icon::PENCIL_LINE,
                    13.0,
                    p.muted,
                );
                list.text(
                    Rect::new(
                        card.left + 32.0,
                        card.top + 6.0,
                        card.right - 12.0,
                        card.top + 26.0,
                    ),
                    "建议修改",
                    TextStyle::Label,
                    p.foreground,
                );
                list.text(
                    Rect::new(
                        card.left + 12.0,
                        card.top + 29.0,
                        card.right - 12.0,
                        card.top + 48.0,
                    ),
                    context::single_line_preview(
                        &format!("{path} · 第 {start}-{end} 行 · {summary}"),
                        TextStyle::Small,
                        card.width() - 24.0,
                    ),
                    TextStyle::Small,
                    p.foreground,
                );
                let status_text = match status {
                    "applied" => "已应用",
                    "rejected" => "已拒绝",
                    "conflict" => "内容已变化",
                    "error" => "处理失败",
                    _ => "等待批准",
                };
                list.text(
                    Rect::new(
                        card.left + 12.0,
                        card.top + 49.0,
                        card.right - 12.0,
                        card.top + 68.0,
                    ),
                    status_text,
                    TextStyle::Small,
                    if status == "pending" {
                        p.muted
                    } else {
                        p.accent
                    },
                );
                if status == "pending" {
                    for (button, hit) in lay.pending_edit_buttons(i, s.scroll) {
                        let approve = matches!(hit, Hit::PendingEditApprove(_));
                        if approve {
                            list.glass_button(button, 5.0, p, s.hover_hit == Some(hit));
                        } else {
                            list.rounded_rect(button, 5.0, p.background);
                        }
                        list.text_aligned(
                            button,
                            if approve { "批准修改" } else { "拒绝" },
                            TextStyle::Small,
                            if approve {
                                p.button_foreground()
                            } else {
                                p.foreground
                            },
                            Align::Center,
                        );
                    }
                }
            }
            if let Some(card) = &m.pending_card {
                let card_rect = Rect::new(
                    left,
                    mr.top + m.pending_card_top - s.scroll,
                    right,
                    mr.top + m.pending_card_top + m.pending_card_height - s.scroll,
                );
                let _ = context::paint_pending_card(list, card_rect, card, p);
            }
            if !m.streaming {
                for (r, action) in lay.message_buttons(i, s.scroll) {
                    let hovered = s.hover_message == Some(i);
                    if !hovered {
                        continue;
                    }
                    list.rounded_rect(
                        r,
                        7.0,
                        theme::mix(
                            p.surface_muted,
                            p.surface,
                            if hovered { 0.82 } else { 0.58 },
                        ),
                    );
                    let (icon, label) = match action {
                        MessageAction::Mount => (Icon::LINK2, "挂载"),
                        MessageAction::Copy => (Icon::COPY, "复制"),
                        MessageAction::Delete => (Icon::TRASH2, "删除"),
                        MessageAction::Insert => (Icon::CROSSHAIR, "插入"),
                    };
                    let color = if action == MessageAction::Delete && s.is_streaming() {
                        theme::mix(p.muted, p.area_assistant_default, 0.4)
                    } else if action == MessageAction::Copy && !hovered {
                        theme::mix(p.muted, p.foreground, 0.22)
                    } else {
                        p.muted
                    };
                    list.icon_centered(
                        Rect::new(r.left + 6.0, r.top, r.left + 20.0, r.bottom),
                        icon,
                        14.0,
                        color,
                    );
                    list.text(
                        Rect::new(r.left + 24.0, r.top, r.right - 4.0, r.bottom),
                        label,
                        TextStyle::Small,
                        color,
                    );
                }
            }
            if let Some(fu) = &m.follow_up {
                let container = Rect::new(
                    left,
                    mr.top + fu.top - s.scroll,
                    right,
                    mr.top + fu.top + fu.height - s.scroll,
                );
                let radius = super::card_radius();
                list.rounded_rect(
                    container,
                    radius,
                    theme::mix(p.surface_muted, p.surface, 0.5),
                );
                list.rounded_border(container, radius, p.border);
                let title_rect = Rect::new(
                    container.left + 12.0,
                    container.top + 7.0,
                    container.right - 12.0,
                    container.top + 24.0,
                );
                list.text(title_rect, "智能追问", TextStyle::Caption, p.muted);
                for (q_idx, (btn_rect, text)) in fu.buttons.iter().enumerate() {
                    let r = Rect::new(
                        container.left + 12.0 + btn_rect.left,
                        container.top + 26.0 + btn_rect.top,
                        container.left + 12.0 + btn_rect.right,
                        container.top + 26.0 + btn_rect.bottom,
                    );
                    let is_hovered = s.hover_hit == Some(Hit::FollowUpQuestion(i, q_idx));
                    let btn_bg = if is_hovered {
                        p.surface_elevated
                    } else {
                        p.surface
                    };
                    list.rounded_rect(r, 6.0, btn_bg);
                    list.rounded_border(r, 6.0, if is_hovered { p.accent } else { p.border });
                    let text_rect =
                        Rect::new(r.left + 10.0, r.top + 5.0, r.right - 10.0, r.bottom - 5.0);
                    paint_wrapped_text(list, text, text_rect, TextStyle::Small, p.foreground, None);
                }
            }
            if m.streaming {
                let cx = (left + m.body.tail.0 + 2.0).min(right - 2.0);
                let cy = body_top + m.body.tail.1;
                list.rect(Rect::new(cx, cy + 3.0, cx + 2.0, cy + lh - 3.0), p.accent);
            }
            if i + 1 < lay.messages.len() {
                list.hline(
                    mr.left + MSG_PAD_X,
                    mr.right - MSG_PAD_X,
                    top + m.height - 1.0,
                    p.border,
                );
            }
        }
    }
    if let Some(r) = lay.rect_of(Hit::ScrollToBottom) {
        let fill = if s.hover_hit == Some(Hit::ScrollToBottom) {
            theme::mix(p.accent, p.surface, 0.24)
        } else {
            theme::mix(p.accent, p.surface, 0.14)
        };
        list.rounded_rect(r, 15.0, fill);
        list.rounded_border(r, 15.0, theme::mix(p.accent, p.surface, 0.35));
        list.icon_centered(
            Rect::new(r.left + 8.0, r.top, r.left + 28.0, r.bottom),
            Icon::CHEVRON_DOWN,
            13.0,
            p.accent,
        );
        list.text(
            Rect::new(r.left + 32.0, r.top, r.right - 8.0, r.bottom),
            "回到底部",
            TextStyle::Tiny,
            p.accent,
        );
    }
    list.pop_clip();

    // 独立工作区导航器（窄面板宽度下隐藏）
    if let Some(nav) = lay.navigation_rect {
        list.rect(nav, p.surface);
        list.border_bottom(nav, p.border);
        list.text(
            Rect::new(
                nav.left + 16.0,
                nav.top + 16.0,
                nav.right - 12.0,
                nav.top + 38.0,
            ),
            "对话导航",
            TextStyle::Label,
            p.foreground,
        );
        let navigation_messages = s.visible_messages();
        let navigation_viewport = Rect::new(nav.left, nav.top + 48.0, nav.right, nav.bottom);
        list.push_clip(navigation_viewport);
        let active_nav_index = lay.entries.iter().find_map(|(_, hit)| {
            if let Hit::NavigateMessage(idx) = hit {
                if lay.messages.get(*idx).is_some_and(|m| {
                    mr.top + m.top <= mr.bottom + s.scroll
                        && mr.top + m.top + m.height >= mr.top + s.scroll
                }) {
                    return Some(*idx);
                }
            }
            None
        });
        for (r, hit) in &lay.entries {
            let Hit::NavigateMessage(index) = hit else {
                continue;
            };
            let row = *r;
            let active = Some(*index) == active_nav_index;
            if active {
                list.rounded_rect(row, 7.0, theme::mix(p.accent, p.surface, 0.10));
            }
            list.rounded_rect(
                Rect::from_size(row.left + 8.0, row.top + 12.0, 6.0, 6.0),
                3.0,
                if active { p.accent } else { p.border },
            );
            let text = navigation_messages
                .get(*index)
                .map(|(_, text)| {
                    text.lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or("AI 回复")
                })
                .unwrap_or("消息");
            list.text(
                Rect::new(
                    row.left + 22.0,
                    row.top + 5.0,
                    row.right - 8.0,
                    row.bottom - 5.0,
                ),
                context::single_line_preview(text, TextStyle::Tiny, (row.width() - 30.0).max(1.0)),
                TextStyle::Tiny,
                if active { p.foreground } else { p.muted },
            );
        }
        list.pop_clip();
    }

    // ── 一体化输入卡片容器（待发送附件、编辑器、底部工具栏、发送按钮） ──
    let chat_right = lay.messages_rect.right;
    let chat_area = Rect::new(lay.messages_rect.left, area.top, chat_right, area.bottom);
    let wrapper = lay.composer_rect.unwrap_or_else(|| {
        Rect::new(
            area.left + INPUT_PAD_X,
            area.bottom - 100.0,
            chat_right - INPUT_PAD_X,
            area.bottom - INPUT_PAD_BOTTOM,
        )
    });

    list.rect(
        Rect::new(area.left, wrapper.top - 6.0, chat_right, area.bottom),
        p.surface,
    );

    list.rounded_rect(wrapper, 9.0, p.surface);
    list.rounded_border(
        wrapper,
        9.0,
        if focused {
            theme::mix(p.accent, p.surface, 0.45)
        } else {
            p.border
        },
    );

    // 1. 上下文附件徽章区
    if !s.pending_selections.is_empty() {
        context::paint_selection_chips(
            list,
            &s.pending_selections,
            Rect::new(wrapper.left + 10.0, 0.0, wrapper.right - 10.0, 0.0),
            wrapper.top + 8.0,
            true,
            p,
        );
    }
    let selection_h = pending_selection_height(s, chat_area);
    let image_top = wrapper.top + 8.0 + selection_h;
    for (i, r) in pending_rects(s, chat_area, image_top)
        .into_iter()
        .enumerate()
    {
        list.push_clip(r);
        if let Some(preview) = s.image_previews.get(&s.pending_images[i]) {
            let scale = (56.0 / preview.width).max(56.0 / preview.height);
            let w = preview.width * scale;
            let h = preview.height * scale;
            list.image_thumbnail(
                Rect::from_size(r.left + (56.0 - w) / 2.0, r.top + (56.0 - h) / 2.0, w, h),
                &preview.path,
                "图片",
            );
        } else {
            list.text(r, "图片", TextStyle::Small, p.muted);
        }
        list.pop_clip();
        list.rounded_border(r, 8.0, p.border);
        if let Some(close) = lay.rect_of(Hit::RemoveImage(i)) {
            list.rounded_rect_alpha(close, 9.0, 0, 0.55);
            list.icon_centered(close, Icon::X, 12.0, 0xffffff);
        }
    }
    if let Some(clip) = lay.files_viewport {
        let images_h = pending_height(s, chat_area);
        let files_top = image_top + images_h;
        list.push_clip(clip);
        for (i, r) in crate::ui::ai_images::file_rects(s, chat_area, files_top)
            .into_iter()
            .enumerate()
        {
            let is_hover =
                s.hover_hit == Some(Hit::OpenFile(i)) || s.hover_hit == Some(Hit::RemoveFile(i));
            list.rounded_rect(
                r,
                6.0,
                if is_hover {
                    p.surface_muted
                } else {
                    p.background
                },
            );
            list.rounded_border(r, 6.0, p.border);
            let name = s.pending_files[i]
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            list.text_aligned(
                Rect::new(r.left + 10.0, r.top, r.right - 30.0, r.bottom),
                context::single_line_preview(
                    &name,
                    TextStyle::Caption,
                    (r.width() - 40.0).max(0.0),
                ),
                TextStyle::Caption,
                p.foreground,
                Align::Leading,
            );
            list.icon_centered(
                Rect::new(r.right - 24.0, r.top, r.right, r.bottom),
                Icon::X,
                14.0,
                if s.hover_hit == Some(Hit::RemoveFile(i)) {
                    p.foreground
                } else {
                    p.muted
                },
            );
        }
        list.pop_clip();
        for (hit, icon) in [
            (Hit::FilesBack, Icon::CHEVRON_LEFT),
            (Hit::FilesForward, Icon::CHEVRON_RIGHT),
        ] {
            if let Some(r) = lay.rect_of(hit) {
                list.icon_centered(r, icon, 14.0, p.muted);
            }
        }
    }

    let has_pending = !s.pending_selections.is_empty()
        || !s.pending_images.is_empty()
        || !s.pending_files.is_empty();
    if has_pending {
        if let Some(input_r) = lay.rect_of(Hit::Input) {
            list.hline(
                wrapper.left + 8.0,
                wrapper.right - 8.0,
                input_r.top - 4.0,
                theme::mix(p.border, p.surface, 0.4),
            );
        }
    }

    // 2. 多行编辑器
    if let Some(r) = lay.rect_of(Hit::Input) {
        let mut f = s.input.clone();
        f.paint_multiline_with_look(list, r, focused, p, FieldLook::bare());
    }

    // 3. 底部集成工具栏
    if let Some(send_r) = lay.rect_of(Hit::Send) {
        list.hline(
            wrapper.left + 8.0,
            wrapper.right - 8.0,
            send_r.top - 4.0,
            theme::mix(p.border, p.surface, 0.6),
        );
    }

    if let Some(r) = lay.rect_of(Hit::AgentPicker) {
        let is_hover = s.hover_hit == Some(Hit::AgentPicker);
        if is_hover {
            list.rounded_rect(r, 6.0, p.surface_muted);
        }
        list.text_aligned(
            Rect::new(r.left + 8.0, r.top, r.right - 20.0, r.bottom),
            context::single_line_preview(
                s.agent_name.as_str(),
                TextStyle::Caption,
                (r.width() - 28.0).max(0.0),
            ),
            TextStyle::Caption,
            if is_hover { p.foreground } else { p.muted },
            Align::Leading,
        );
        list.icon_centered(
            Rect::new(r.right - 18.0, r.top, r.right - 4.0, r.bottom),
            Icon::CHEVRON_DOWN,
            11.0,
            p.muted,
        );
    }

    if let (Some(r_app), Some(r_auto)) = (
        lay.rect_of(Hit::ApproveMode),
        lay.rect_of(Hit::AutomaticMode),
    ) {
        let seg_r = Rect::new(r_app.left, r_app.top, r_auto.right, r_app.bottom);
        list.rounded_rect(seg_r, 5.0, theme::mix(p.surface_muted, p.surface, 0.6));
        let is_auto = s.automatic_edits;
        let active_r = if is_auto { r_auto } else { r_app };
        list.rounded_rect(active_r, 4.0, p.surface);
        list.rounded_border(active_r, 4.0, p.border);
        list.text_aligned(
            r_app,
            "批准",
            TextStyle::Caption,
            if !is_auto { p.foreground } else { p.muted },
            Align::Center,
        );
        list.text_aligned(
            r_auto,
            "自动执行",
            TextStyle::Caption,
            if is_auto { p.foreground } else { p.muted },
            Align::Center,
        );
    }

    if let Some(r) = lay.rect_of(Hit::AttachFiles) {
        let is_hover = s.hover_hit == Some(Hit::AttachFiles);
        if is_hover {
            list.rounded_rect(r, 5.0, p.surface_muted);
        }
        list.icon_centered(
            r,
            Icon::PAPERCLIP,
            15.0,
            if is_hover { p.foreground } else { p.muted },
        );
    }

    if let Some(r) = lay.rect_of(Hit::MountSession) {
        let enabled = s.active.as_ref().is_some_and(|c| !c.messages.is_empty());
        let is_hover = s.hover_hit == Some(Hit::MountSession) && enabled;
        if is_hover {
            list.rounded_rect(r, 5.0, p.surface_muted);
        }
        if r.width() >= 50.0 {
            list.text_aligned(
                r,
                "挂载会话",
                TextStyle::Caption,
                if enabled {
                    if is_hover {
                        p.accent
                    } else {
                        p.muted
                    }
                } else {
                    theme::mix(p.muted, p.surface, 0.4)
                },
                Align::Center,
            );
        } else {
            list.icon_centered(
                r,
                Icon::LINK,
                13.0,
                if enabled {
                    if is_hover {
                        p.accent
                    } else {
                        p.muted
                    }
                } else {
                    theme::mix(p.muted, p.surface, 0.4)
                },
            );
        }
    }

    if let (Some(r_auto), Some(send_r)) = (lay.rect_of(Hit::AutomaticMode), lay.rect_of(Hit::Send))
    {
        let left = lay
            .rect_of(Hit::MountSession)
            .or_else(|| lay.rect_of(Hit::AttachFiles))
            .map(|r| r.right)
            .unwrap_or(r_auto.right)
            + 8.0;
        let right = send_r.left - 8.0;
        if right > left + 30.0 && !s.loaded_skills.is_empty() {
            list.text(
                Rect::new(left, send_r.top, right, send_r.bottom),
                context::single_line_preview(
                    &format!("已加载：{}", s.loaded_skills.join("、")),
                    TextStyle::Caption,
                    right - left,
                ),
                TextStyle::Caption,
                p.muted,
            );
        }
    }

    if let Some(r) = lay.rect_of(Hit::Send) {
        let enabled = s.is_streaming()
            || !s.input.text().trim().is_empty()
            || !s.pending_images.is_empty()
            || !s.pending_files.is_empty();
        let start = list.cmds().len();
        list.glass_button(r, 7.0, p, enabled && s.hover_hit == Some(Hit::Send));
        list.icon_centered(
            r,
            if s.is_streaming() {
                Icon::SQUARE
            } else {
                Icon::SEND
            },
            14.0,
            p.button_foreground(),
        );
        if !enabled {
            list.fade_since(start, r, 0.45);
        }
    }

    if !s.error.is_empty() {
        list.text(
            Rect::new(
                area.left + INPUT_PAD_X,
                wrapper.top - 20.0,
                chat_right - INPUT_PAD_X,
                wrapper.top,
            ),
            context::single_line_preview(
                &s.error,
                TextStyle::Caption,
                (chat_right - area.left - 40.0).max(0.0),
            ),
            TextStyle::Caption,
            p.danger,
        );
    }

    // ── 会话弹层 ──
    if let Some(pop) = lay.popover {
        list.rounded_rect(pop, 10.0, p.surface);
        list.rounded_border(pop, 10.0, p.border);
        list.text_run(
            Rect::new(
                pop.left + 12.0,
                pop.top + 6.0,
                pop.right - 48.0,
                pop.top + 34.0,
            ),
            "会话",
            TextStyle::Label,
            p.foreground,
            Align::Leading,
            Emphasis::Bold,
        );
        if let Some(r) = lay.rect_of(Hit::PopoverNew) {
            list.icon_centered(r, Icon::PLUS, 14.0, p.muted);
        }
        let sessions = s.sorted_sessions();
        if sessions.is_empty() {
            list.text_aligned(
                Rect::new(pop.left, pop.top + 44.0, pop.right, pop.top + 84.0),
                "还没有会话",
                TextStyle::Caption,
                p.muted,
                Align::Center,
            );
        }
        let viewport = lay.conversations_viewport.unwrap_or(pop);
        list.push_clip(viewport);
        for &(i, r) in &lay.conversation_rows {
            let Some(meta) = sessions.get(i) else {
                continue;
            };
            let active = s.active.as_ref().map(|c| c.id == meta.id).unwrap_or(false);
            if active {
                list.rounded_rect(r, 6.0, theme::mix(p.accent, p.surface, 0.10));
            }
            list.icon_centered(
                Rect::new(r.left + 8.0, r.top, r.left + 22.0, r.bottom),
                Icon::MESSAGE_SQUARE,
                14.0,
                if active { p.accent } else { p.muted },
            );
            list.text(
                Rect::new(r.left + 30.0, r.top + 6.0, r.right - 36.0, r.top + 26.0),
                context::single_line_preview(&meta.title, TextStyle::Label, r.width() - 70.0),
                TextStyle::Label,
                p.foreground,
            );
            list.text(
                Rect::new(r.left + 30.0, r.top + 26.0, r.right - 36.0, r.top + 42.0),
                crate::ui::panels::format_time(meta.updated_at),
                TextStyle::Caption,
                p.muted,
            );
            let d = Rect::new(r.right - 32.0, r.top + 12.0, r.right - 8.0, r.top + 36.0);
            list.icon_centered(d, Icon::X, 12.0, p.muted);
        }
        if lay.conversations_max_scroll > 0.0 {
            let height = (viewport.height() * viewport.height()
                / (viewport.height() + lay.conversations_max_scroll))
                .max(16.0)
                .min(viewport.height());
            let top = viewport.top
                + (viewport.height() - height)
                    * s.conversations_scroll
                        .clamp(0.0, lay.conversations_max_scroll)
                    / lay.conversations_max_scroll;
            list.rounded_rect(
                Rect::new(viewport.right - 3.0, top, viewport.right, top + height),
                1.5,
                p.muted,
            );
        }
        list.pop_clip();
    }
    // tooltip 文本刻意延迟到所有面板画完之后再画，保证提示位于
    // 消息/输入裁剪之上，并跟随 root 在指针移动期间维护的同一语义命中。
    for hit in [
        Hit::Conversations,
        Hit::PreviousQuestion,
        Hit::SearchToggle,
        Hit::NewSession,
        Hit::Clear,
        Hit::Settings,
        Hit::AgentConfig,
        Hit::Float,
        Hit::Close,
        Hit::SearchPrevious,
        Hit::SearchNext,
        Hit::SearchClose,
        Hit::AttachFiles,
        Hit::Send,
        Hit::ScrollToBottom,
    ] {
        if s.hover_hit != Some(hit) {
            continue;
        }
        let Some(rect) = lay.rect_of(hit) else {
            continue;
        };
        let Some(label) = control_tooltip(hit, s.is_streaming()) else {
            continue;
        };
        paint_control_tooltip(list, area, rect, label, p);
    }
    list.pop_clip();
}
