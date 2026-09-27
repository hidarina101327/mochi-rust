use super::*;
use crate::ui::{
    draw::{DrawCmd, DrawList},
    icons::Icon,
    layout::Rect,
    theme,
};

fn populated() -> State {
    let mut s = State::default();
    for i in 0..60 {
        s.history.push(
            if i % 2 == 0 {
                Category::Document
            } else {
                Category::Assistant
            },
            &format!("消息 {i}"),
            &format!("第 {i} 条消息，中文内容。"),
            i * 6000,
        );
    }
    s.open(Mode::Center);
    s
}

#[test]
fn automation_category_wraps_without_covering_notification_rows() {
    let s = populated();
    for width in [400., 1280.] {
        let lay = layout(&s, Rect::from_size(0., 0., width, 800.), Rect::ZERO);
        for (rect, hit) in &lay.controls {
            if matches!(hit, Hit::Filter(_)) {
                assert!(rect.bottom <= lay.body.top);
            }
        }
        let (row, id) = lay.rows[0];
        assert_eq!(lay.hit(row.left + 5., row.top + 5.), Some(Hit::Notice(id)));
    }
}

#[test]
fn notifications_bound_history_merge_repeats_and_preserve_read_state() {
    let mut h = History::default();
    let id = h.push(Category::System, "标题", "重复消息", 10).unwrap();
    assert!(h.mark_read(id));
    assert_eq!(h.unread(), 0);
    assert_eq!(h.push(Category::System, "标题", "重复消息", 100), Some(id));
    assert_eq!(h.entries[0].occurrences, 2);
    assert_eq!(h.unread(), 1);
    let other = h.push(Category::Document, "标题", "重复消息", 101).unwrap();
    assert_ne!(id, other);
    assert!(h.mark_read(other));
    let raw = serde_json::to_string(&h.entries).unwrap();
    let restored = History::restore(Some(&raw));
    assert_eq!(restored.entries, h.entries);
    assert_eq!(restored.unread(), 1);
    for n in 0..350 {
        h.push(Category::System, "提示", &n.to_string(), n * 6000);
    }
    assert_eq!(h.entries.len(), model::CAPACITY);
    assert_eq!(h.entries[0].message, "349");
    assert!(h.mark_all_read());
    assert!(!h.mark_all_read());
}

#[test]
fn notifications_reject_corrupt_history_and_bound_unicode_messages() {
    assert!(History::restore(Some("{broken")).entries.is_empty());
    let mut h = History::default();
    assert_eq!(h.push(Category::System, "", " \n ", 0), None);
    h.push(Category::System, &"字".repeat(200), &"中🦀".repeat(2000), 0);
    assert!(h.entries[0].title.chars().count() <= 81);
    assert_eq!(h.entries[0].message.chars().count(), 2049);
    let raw = serde_json::to_string(&h.entries).unwrap();
    assert_eq!(History::restore(Some(&raw)).entries, h.entries);
}

#[test]
fn notifications_filter_and_read_actions_do_not_consume_unseen_messages() {
    let mut s = populated();
    let count = s.history.unread();
    s.open(Mode::Popover);
    assert_eq!(s.history.unread(), count);
    s.activate(Hit::More);
    let id = s.history.entries[0].id;
    assert!(s.activate(Hit::Notice(id)));
    assert_eq!(s.history.unread(), count - 1);
    assert_eq!(s.selected_notice().unwrap().id, id);
    s.activate(Hit::Back);
    s.activate(Hit::Filter(Filter::Category(Category::Document)));
    let lay = layout(&s, Rect::from_size(0.0, 0.0, 1280.0, 800.0), Rect::ZERO);
    assert_eq!(lay.rows.len(), 30);
    s.activate(Hit::Filter(Filter::Unread));
    let lay = layout(&s, Rect::from_size(0.0, 0.0, 1280.0, 800.0), Rect::ZERO);
    assert_eq!(lay.rows.len(), count - 1);
    assert!(!lay.rows.iter().any(|(_, n)| *n == id));
}

#[test]
fn notifications_popup_is_anchored_and_small_windows_keep_controls_separate() {
    let mut s = populated();
    for (w, h) in [(1280.0, 800.0), (480.0, 380.0), (320.0, 260.0)] {
        let viewport = Rect::from_size(0.0, 0.0, w, h);
        for mode in [Mode::Popover, Mode::Center] {
            s.open(mode);
            let lay = layout(&s, viewport, Rect::from_size(w - 206.0, 0.0, 34.0, 34.0));
            assert!(lay.frame.left >= viewport.left && lay.frame.right <= viewport.right);
            assert!(lay.frame.top >= viewport.top && lay.frame.bottom <= viewport.bottom);
            assert!(lay.body.height() >= 0.0);
            let close = lay
                .controls
                .iter()
                .find(|(_, h)| *h == Hit::Close)
                .unwrap()
                .0;
            let read = lay
                .controls
                .iter()
                .find(|(_, h)| *h == Hit::MarkAllRead)
                .unwrap()
                .0;
            assert!(read.right < close.left);
            assert_eq!(lay.rows.len(), if mode == Mode::Popover { 5 } else { 60 });
        }
    }
}

#[test]
fn notifications_keyboard_scroll_reaches_last_row_and_scrollbar_is_overlay() {
    let mut s = populated();
    let vp = Rect::from_size(0.0, 0.0, 1280.0, 800.0);
    for _ in 0..70 {
        let lay = layout(&s, vp, Rect::ZERO);
        s.navigate(&lay, false);
    }
    assert!(s.scroll > 0.0);
    let lay = layout(&s, vp, Rect::ZERO);
    assert_eq!(lay.scroll, lay.max_scroll);
    let body = lay.body;
    assert_eq!(lay.bars()[0].1.thumb.width(), 6.0);
    let bars = lay.bars();
    s.scrollbar.pointer(&bars, body.right - 1.0, body.top + 2.0);
    assert_eq!(layout(&s, vp, Rect::ZERO).body, body);
    let first = lay.rows[0].0;
    assert!(lay.hit(first.left + 1.0, first.top + 2.0).is_none());
}

#[test]
fn notifications_paint_blue_unread_dots_without_reflow_in_both_themes() {
    for dark in [false, true] {
        let mut s = populated();
        let vp = Rect::from_size(0.0, 0.0, 1280.0, 800.0);
        let p = theme::configured_palette(dark);
        let before = layout(&s, vp, Rect::ZERO);
        let mut list = DrawList::new();
        paint(&mut list, &s, &before, vp, &p);
        assert!(list.finish().is_ok());
        assert!(list.cmds().iter().any(|c| matches!(c, DrawCmd::RoundedRect { color, radius, .. } if *color == blue(&p) && *radius == 3.0)));
        assert!(list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Icon { icon, .. } if *icon == Icon::BOT)));
        s.history.mark_all_read();
        list.clear();
        let after = layout(&s, vp, Rect::ZERO);
        paint(&mut list, &s, &after, vp, &p);
        assert_eq!(before.body, after.body);
        assert!(!list.cmds().iter().any(|c| matches!(c, DrawCmd::RoundedRect { color, radius, .. } if *color == blue(&p) && *radius == 3.0)));
    }
}

#[test]
fn notifications_details_wrap_and_remain_scrollable() {
    let mut s = State::default();
    let id = s
        .history
        .push(
            Category::Document,
            "导入结果",
            &"中文 English 🦀 内容\n".repeat(100),
            0,
        )
        .unwrap();
    s.activate(Hit::Notice(id));
    let vp = Rect::from_size(0.0, 0.0, 480.0, 380.0);
    let lay = layout(&s, vp, Rect::ZERO);
    assert!(lay.max_scroll > 0.0 && lay.detail_lines.len() >= 100);
    assert!(lay.rows.is_empty());
    assert_eq!(s.history.unread(), 0);
}

#[test]
fn notifications_category_controls_are_hit_testable_and_render_at_high_dpi() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
    }
    let mut renderer = crate::gfx::Renderer::new().unwrap();
    for (width, dpi, dark) in [(320., 192., true), (800., 144., false)] {
        let snapshot = renderer
            .prepare_snapshot((width * dpi / 96.) as u32, (640. * dpi / 96.) as u32, dpi)
            .unwrap();
        let viewport = Rect::from_size(0., 0., width, 640.);
        let mut state = State::default();
        let id = state
            .history
            .push(
                Category::Document,
                "文档导入完成",
                "Markdown 文档和引用图片已加入知识库。\n关闭此类消息提醒后，记录仍保留在通知中心。",
                0,
            )
            .unwrap();
        state.activate(Hit::Notice(id));
        for muted in [false, true] {
            state.muted = if muted {
                vec![Category::Document]
            } else {
                vec![]
            };
            let lay = layout(&state, viewport, Rect::ZERO);
            for (index, (rect, hit)) in lay.controls.iter().enumerate() {
                assert_eq!(
                    lay.hit((rect.left + rect.right) / 2., (rect.top + rect.bottom) / 2.),
                    Some(*hit)
                );
                for (other, _) in &lay.controls[index + 1..] {
                    assert!(
                        rect.right <= other.left
                            || other.right <= rect.left
                            || rect.bottom <= other.top
                            || other.bottom <= rect.top
                    );
                }
            }
            let mut list = DrawList::new();
            let palette = theme::configured_palette(dark);
            paint(&mut list, &state, &lay, viewport, &palette);
            assert!(list.finish().is_ok());
            assert!(list.cmds().iter().any(|c| matches!(c, DrawCmd::Text { text, .. } if text == if muted { "恢复此类消息提醒" } else { "不再提示此类消息" })));
            renderer
                .present(
                    windows::Win32::Foundation::HWND::default(),
                    palette.background,
                    &list,
                )
                .unwrap();
            if let Some(output) = std::env::var_os("MOCHI_NOTIFICATION_QA_DIR") {
                let output = std::path::PathBuf::from(output);
                std::fs::create_dir_all(&output).unwrap();
                renderer
                    .save_snapshot(
                        &snapshot,
                        &output.join(format!("notifications-{width}-{muted}.png")),
                    )
                    .unwrap();
            }
        }
    }
}
