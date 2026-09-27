//! 有状态的组件绘制，以及支持原生 IME 的输入框。
use super::*;
use crate::ui::{
    draw::{Align, TextStyle},
    layout::Edges,
};
use chrono::{Datelike, Timelike};
use mochi_core::desktop_cards::{
    studio::{Kind, Node, Trigger},
    Module,
};
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Live {
    pub now: i64,
    pub timer: String,
    pub running: bool,
    pub messages: Vec<(String, String)>,
    pub sessions: Vec<(String, String)>,
    pub streaming: bool,
    pub error: String,
    pub title: String,
}
pub fn is_studio(view: &View) -> bool {
    matches!(view.module, Some(Module::Custom | Module::Clock))
}
pub fn node_rect(n: &Node, b: Rect) -> Rect {
    Rect::from_size(
        b.left + b.width() * n.x as f32 / 10000.0,
        b.top + b.height() * n.y as f32 / 10000.0,
        b.width() * n.width as f32 / 10000.0,
        b.height() * n.height as f32 / 10000.0,
    )
}
/// 编辑器预览与桌面窗口采用相同的内容比例。
pub fn design_size(
    card: &mochi_core::desktop_cards::Card,
    page: &mochi_core::desktop_cards::Page,
) -> (f32, f32) {
    let spec = Spec {
        appearance: card.appearance.clone(),
        id: card.id.clone(),
        title: card.title.clone(),
        x: card.x,
        y: card.y,
        width: card.width,
        height: card.height,
        locked: card.locked,
        dark: false,
    };
    let view = View {
        module: Some(page.module),
        tabs: card
            .pages
            .iter()
            .map(|p| (p.id.clone(), p.title.clone()))
            .collect(),
        studio: mochi_core::desktop_cards::studio::Studio {
            height: page.studio.height,
            ..Default::default()
        },
        ..Default::default()
    };
    let r = studio_body(
        &spec,
        &view,
        Rect::from_size(0.0, 0.0, card.width as f32, card.height as f32),
        0,
    );
    (r.width().max(1.0), r.height().max(1.0))
}
pub(super) fn controls(spec: &Spec, v: &View, a: Rect, offset: usize) -> Option<Vec<(Rect, Hit)>> {
    let b = if is_studio(v) {
        studio_body(spec, v, a, offset)
    } else {
        painting::content_rect(spec, v, a)
    };
    if is_studio(v) {
        let mut hits = vec![];
        for n in v.studio.nodes.iter().rev().filter(|n| !n.hidden) {
            let mut styled = n.clone();
            let defaults = mochi_core::desktop_cards::ItemStyle {
                foreground: v.presentation.item_foreground.or(n.foreground),
                background: v.presentation.item_background.or(n.background),
            };
            let style = v
                .item_styles
                .get(&format!("node:{}", n.id))
                .cloned()
                .unwrap_or_default()
                .over(&defaults);
            styled.foreground = style.foreground;
            styled.background = style.background;
            let n = &styled;
            let r = node_rect(n, b);
            if n.kind == Kind::AiChat {
                let mut inner = v.clone();
                inner.module = Some(Module::Ai);
                inner.tabs.clear();
                hits.extend(controls(spec, &inner, r, offset).unwrap_or_default());
            } else if n.kind == Kind::Data {
                hits.push((
                    Rect::from_size(r.right - 52.0, r.top, 24.0, 24.0),
                    Hit::WidgetScroll(n.id.clone(), -1),
                ));
                hits.push((
                    Rect::from_size(r.right - 26.0, r.top, 24.0, 24.0),
                    Hit::WidgetScroll(n.id.clone(), 1),
                ));
                if let Some(rows) = v.widget_rows.get(&n.id) {
                    for row in data_entries(
                        n,
                        r,
                        rows,
                        v.widget_offsets.get(&n.id).copied().unwrap_or(0),
                    ) {
                        hits.push((row.0, Hit::WidgetRow(n.id.clone(), row.1.id.clone())));
                    }
                }
            } else {
                hits.push((r, Hit::Widget(n.id.clone(), Trigger::Click)));
            }
        }
        return Some(hits);
    }

    if v.module == Some(Module::Pomodoro) {
        let y = b.top + b.height() * 0.65;
        return Some(vec![
            (
                Rect::from_size(b.left + b.width() * 0.15, y, b.width() * 0.43, 40.0),
                Hit::Timer(false),
            ),
            (
                Rect::from_size(b.left + b.width() * 0.62, y, b.width() * 0.23, 40.0),
                Hit::Timer(true),
            ),
        ]);
    }
    if v.module == Some(Module::Ai) {
        return Some(vec![
            (
                Rect::from_size(b.right - 62.0, b.top, 62.0, 24.0),
                Hit::AiHistory,
            ),
            (super::composer::send_rect(b), Hit::AiSend),
            (super::composer::text_rect(b), Hit::AiComposer),
        ]);
    }
    None
}
pub fn chat_max(spec: &Spec, v: &View, a: Rect) -> usize {
    super::chat::max(v, chat_body(spec, v, a))
}
pub(super) fn paint(
    list: &mut DrawList,
    spec: &Spec,
    v: &View,
    a: Rect,
    offset: usize,
    hover: Option<&Hit>,
) -> bool {
    let (p, item_bg) = painting::item_palette(&painting::palette(spec), v, "page");
    let b = if is_studio(v) {
        studio_body(spec, v, a, offset)
    } else {
        painting::content_rect(spec, v, a)
    };
    if let Some(bg) = item_bg {
        list.rect(b, bg);
    }
    if is_studio(v) {
        list.push_clip(painting::content_rect(spec, v, a));
        for n in &v.studio.nodes {
            if n.hidden {
                continue;
            }
            let mut styled = n.clone();
            let defaults = mochi_core::desktop_cards::ItemStyle {
                foreground: v.presentation.item_foreground.or(n.foreground),
                background: v.presentation.item_background.or(n.background),
            };
            let style = v
                .item_styles
                .get(&format!("node:{}", n.id))
                .cloned()
                .unwrap_or_default()
                .over(&defaults);
            styled.foreground = style.foreground;
            styled.background = style.background;
            let n = &styled;
            let r = node_rect(n, b);
            if n.kind == Kind::AiChat {
                let mut inner = v.clone();
                inner.module = Some(Module::Ai);
                inner.tabs.clear();
                let mut appearance = chat_spec(spec, v);
                appearance.appearance.font_color =
                    n.foreground.or(appearance.appearance.font_color);
                appearance.appearance.background_color =
                    n.background.or(appearance.appearance.background_color);
                if let Some(bg) = n.background {
                    list.rounded_rect(r, 4.0, bg);
                }
                paint(list, &appearance, &inner, r, offset, hover);
                continue;
            }
            if n.kind == Kind::Data {
                if let Some(bg) = n.background {
                    list.rounded_rect(r, 4.0, bg);
                }
                if n.border {
                    list.rounded_border(r, 4.0, p.border);
                }
                list.icon_centered(
                    Rect::from_size(r.right - 52.0, r.top, 24.0, 24.0),
                    crate::ui::icons::Icon::CHEVRON_UP,
                    12.0,
                    p.muted,
                );
                list.icon_centered(
                    Rect::from_size(r.right - 26.0, r.top, 24.0, 24.0),
                    crate::ui::icons::Icon::CHEVRON_DOWN,
                    12.0,
                    p.muted,
                );
                list.text(
                    Rect::from_size(r.left + 6.0, r.top, r.width() - 64.0, 24.0),
                    &n.title,
                    TextStyle::Caption,
                    p.muted,
                );
                if let Some(rows) = v.widget_rows.get(&n.id) {
                    for (rr, row) in data_entries(
                        n,
                        r,
                        rows,
                        v.widget_offsets.get(&n.id).copied().unwrap_or(0),
                    ) {
                        let (row_palette, bg) = painting::item_palette(
                            &p,
                            v,
                            &mochi_core::desktop_cards::item_key(&row.id, &row.meta.path),
                        );
                        if let Some(bg) = bg {
                            list.rounded_rect(rr, 4.0, bg);
                        }
                        painting::scaled(
                            list,
                            rr,
                            &row.title,
                            n.font_size as f32,
                            node_color(n, row_palette.foreground),
                            Align::Leading,
                        );
                        list.hline(rr.left, rr.right, rr.bottom, p.border);
                    }
                }
                continue;
            }
            paint_node(
                list,
                n,
                r,
                p.foreground,
                p.border,
                &v.live,
                &v.workspace,
                hover.is_some_and(|h| matches!(h,Hit::Widget(id,_) if id==&n.id)),
            );
        }
        if v.studio.nodes.is_empty() {
            list.text_aligned(
                b,
                "拖入文件创建快捷方式\n右键 → 设置 → 设计工作台",
                TextStyle::Label,
                p.muted,
                Align::Center,
            );
        }
        list.pop_clip();
        return true;
    }
    if v.module == Some(Module::Pomodoro) {
        painting::scaled(
            list,
            Rect::new(
                b.left,
                b.top + b.height() * 0.12,
                b.right,
                b.top + b.height() * 0.5,
            ),
            &v.live.timer,
            (b.width() / 4.8).clamp(32.0, 96.0),
            p.foreground,
            Align::Center,
        );
        list.text_aligned(
            Rect::new(
                b.left,
                b.top + b.height() * 0.48,
                b.right,
                b.top + b.height() * 0.60,
            ),
            if v.live.running {
                "专注中"
            } else {
                "准备好后开始专注"
            },
            TextStyle::Label,
            p.muted,
            Align::Center,
        );
        for (r, h) in controls(spec, v, a, offset).unwrap() {
            list.rounded_border(r, 6.0, p.border);
            if hover == Some(&h) {
                list.rounded_rect(r, 6.0, p.surface_muted);
            }
            list.text_aligned(
                r,
                if h == Hit::Timer(true) {
                    "重置"
                } else if v.live.running {
                    "暂停"
                } else {
                    "开始"
                },
                TextStyle::Label,
                p.foreground,
                Align::Center,
            );
        }
        return true;
    }
    if v.module == Some(Module::Ai) {
        super::chat::paint(list, spec, v, a, offset);
        return true;
    }
    false
}
fn node_color(n: &Node, inherited: u32) -> u32 {
    n.foreground.unwrap_or_else(|| {
        n.background
            .map(|bg| {
                if ((bg >> 16) & 255) * 299 + ((bg >> 8) & 255) * 587 + (bg & 255) * 114 < 145000 {
                    0xf0f2f4
                } else {
                    0x252b32
                }
            })
            .unwrap_or(inherited)
    })
}
pub fn paint_node(
    list: &mut DrawList,
    n: &Node,
    r: Rect,
    fg: u32,
    border: u32,
    live: &Live,
    workspace: &std::path::Path,
    hover: bool,
) {
    use crate::ui::icons::Icon;
    let color = node_color(n, fg);
    if let Some(bg) = n.background {
        list.rounded_rect(r, 4.0, bg);
    }
    if n.border || hover {
        list.rounded_border(r, 4.0, border);
    }
    if n.kind == Kind::Divider {
        list.hline(r.left, r.right, (r.top + r.bottom) / 2.0, border);
        return;
    }
    if n.kind == Kind::AnalogClock {
        let now = chrono::DateTime::from_timestamp(live.now, 0)
            .filter(|_| live.now != 0)
            .map(|d| d.with_timezone(&chrono::Local))
            .unwrap_or_else(chrono::Local::now);
        let radius = (r.width().min(r.height()) / 2.0 - 10.0).max(8.0);
        let cx = (r.left + r.right) / 2.0;
        let cy = (r.top + r.bottom) / 2.0;
        let point = |turn: f32, length: f32| {
            let a = turn * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            (cx + a.cos() * length, cy + a.sin() * length)
        };
        list.polyline(
            (0..=96).map(|i| point(i as f32 / 96.0, radius)).collect(),
            border,
            1.0,
        );
        for i in 0..60 {
            list.polyline(
                vec![
                    point(i as f32 / 60.0, radius - 5.0),
                    point(
                        i as f32 / 60.0,
                        radius - if i % 5 == 0 { 12.0 } else { 8.0 },
                    ),
                ],
                if i % 5 == 0 { color } else { border },
                if i % 5 == 0 { 2.0 } else { 1.0 },
            );
        }
        if radius > 48.0 {
            for i in 1..=12 {
                let (x, y) = point(i as f32 / 12.0, radius - 25.0);
                painting::scaled(
                    list,
                    Rect::from_size(x - 14.0, y - 12.0, 28.0, 24.0),
                    &i.to_string(),
                    (radius / 8.0).clamp(10.0, 20.0),
                    color,
                    Align::Center,
                );
            }
        }
        let second = now.second() as f32;
        let minute = now.minute() as f32 + second / 60.0;
        let hour = (now.hour() % 12) as f32 + minute / 60.0;
        list.polyline(
            vec![(cx, cy), point(hour / 12.0, radius * 0.46)],
            color,
            4.0,
        );
        list.polyline(
            vec![(cx, cy), point(minute / 60.0, radius * 0.70)],
            color,
            2.5,
        );
        list.polyline(
            vec![
                point(second / 60.0, -radius * 0.15),
                point(second / 60.0, radius * 0.80),
            ],
            crate::ui::theme::mix(color, border, 0.65),
            1.2,
        );
        list.rounded_rect(Rect::from_size(cx - 3.0, cy - 3.0, 6.0, 6.0), 3.0, color);
        return;
    }
    let value = match n.kind {
        Kind::Clock | Kind::Date => {
            let format = if n.target.is_empty() {
                if n.kind == Kind::Clock {
                    "%H:%M:%S"
                } else {
                    "%Y-%m-%d"
                }
            } else {
                &n.target
            };
            let items = chrono::format::StrftimeItems::new(format);
            if items
                .clone()
                .any(|i| matches!(i, chrono::format::Item::Error))
            {
                "格式无效".into()
            } else {
                {
                    let now = chrono::DateTime::from_timestamp(live.now, 0)
                        .filter(|_| live.now != 0)
                        .map(|d| d.with_timezone(&chrono::Local))
                        .unwrap_or_else(chrono::Local::now);
                    let format = format.replace(
                        "%A",
                        [
                            "星期一",
                            "星期二",
                            "星期三",
                            "星期四",
                            "星期五",
                            "星期六",
                            "星期日",
                        ][now.weekday().num_days_from_monday() as usize],
                    );
                    now.format(&format).to_string()
                }
            }
        }
        Kind::Timer => live.timer.clone(),
        _ => n.title.clone(),
    };
    let mut text_rect = r.inset(Edges::xy(4.0, 2.0));
    if n.kind == Kind::Shortcut && n.show_icon {
        let size = (r.height() * 0.45).min(36.0);
        let ir = Rect::from_size(r.left + (r.width() - size) / 2.0, r.top + 6.0, size, size);
        if let Some(path) = super::shortcut_icon::path(workspace, &n.target) {
            list.image(ir, path, &n.title);
        } else {
            list.icon_centered(ir, Icon::FILE_TEXT, 24.0, color);
        }
        text_rect.top += r.height() * 0.45;
    }
    painting::scaled(
        list,
        text_rect,
        &value,
        n.font_size as f32,
        color,
        Align::Center,
    );
}
pub unsafe fn sync_input(hwnd: HWND) {
    let a = area(hwnd);
    let Some(s) = (unsafe { state(hwnd) }) else {
        return;
    };
    if super::tree::enabled(&s.view) {
        let _ = SetTimer(Some(hwnd), super::tree::TIMER_ID, 40, None);
    } else {
        let _ = KillTimer(Some(hwnd), super::tree::TIMER_ID);
    }
    super::shortcut_icon::watch(hwnd);
    let ai = has_chat(&s.view) && !s.spec.appearance.capsule;
    let period = if ai {
        120
    } else if s.view.module == Some(Module::Pomodoro)
        || s.view.studio.nodes.iter().any(|n| {
            matches!(
                n.kind,
                Kind::Clock | Kind::AnalogClock | Kind::Date | Kind::Timer
            )
        })
    {
        1000
    } else {
        0
    };
    if period != s.live_timer_period {
        unsafe {
            if period == 0 {
                let _ = KillTimer(Some(hwnd), LIVE_TIMER);
            } else {
                let _ = SetTimer(Some(hwnd), LIVE_TIMER, period, None);
            }
        }
        s.live_timer_period = period;
    }
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let next = if ai || s.view.module == Some(Module::Folder) {
            ex & !WS_EX_NOACTIVATE.0
        } else {
            ex | WS_EX_NOACTIVATE.0
        };
        if next != ex {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next as isize);
        }
        if ai && s.composer.is_none() {
            s.composer = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                w!(""),
                WS_CHILD
                    | WS_VISIBLE
                    | WS_TABSTOP
                    | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32),
                0,
                0,
                100,
                40,
                Some(hwnd),
                None,
                None,
                None,
            )
            .ok();
            if let Some(edit) = s.composer {
                super::composer::install(edit);
                let _ = SendMessageW(edit, 0x00c5, Some(WPARAM(16000)), Some(LPARAM(0)));
                let font = GetStockObject(DEFAULT_GUI_FONT);
                let _ = SendMessageW(
                    edit,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }
        }
        if let Some(edit) = s.composer {
            let _ = ShowWindow(edit, if ai { SW_SHOW } else { SW_HIDE });
            if ai {
                let b = chat_body(&s.spec, &s.view, a);
                let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
                let size = ((chat_spec(&s.spec, &s.view).appearance.font_size as f32)
                    .clamp(12.0, 24.0)
                    * scale)
                    .round() as i32;
                if size != s.input_font_size {
                    let mut lf = LOGFONTW {
                        lfHeight: -size,
                        ..Default::default()
                    };
                    let name: Vec<_> = "Microsoft YaHei UI".encode_utf16().collect();
                    lf.lfFaceName[..name.len()].copy_from_slice(&name);
                    let font = CreateFontIndirectW(&lf);
                    if !font.is_invalid() {
                        SendMessageW(
                            edit,
                            WM_SETFONT,
                            Some(WPARAM(font.0 as usize)),
                            Some(LPARAM(1)),
                        );
                        if !s.input_font.is_invalid() {
                            let _ = DeleteObject(s.input_font.into());
                        }
                        s.input_font = font;
                        s.input_font_size = size;
                    }
                }
                let _ = SetWindowPos(
                    edit,
                    None,
                    ((b.left + 12.0) * scale) as i32,
                    ((b.bottom - 70.0) * scale) as i32,
                    ((b.width() - 24.0) * scale).max(20.0) as i32,
                    1,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }
}
pub fn send(hwnd: HWND) {
    unsafe {
        let Some(s) = state(hwnd) else { return };
        if s.view.live.streaming {
            emit(hwnd, EventKind::AiStop);
            return;
        }
        if let Some(edit) = s.composer {
            let mut buf = vec![0u16; 16001];
            let n = GetWindowTextW(edit, &mut buf);
            let value = String::from_utf16_lossy(&buf[..n as usize]);
            if !value.trim().is_empty() {
                emit(hwnd, EventKind::AiSend(value));
            }
        }
    }
}
pub fn history(hwnd: HWND) {
    unsafe {
        let Some(s) = state(hwnd) else { return };
        if s.view.live.streaming {
            return;
        }
        let sessions = s.view.live.sessions.clone();
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, 1, w!("新对话"));
        for (i, (_, title)) in sessions.iter().enumerate() {
            let title = windows::core::HSTRING::from(title);
            let _ = AppendMenuW(menu, MF_STRING, 100 + i, &title);
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        s.dragging = true;
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        )
        .0;
        let _ = DestroyMenu(menu);
        if let Some(s) = state(hwnd) {
            s.dragging = false;
        }
        if id == 1 {
            emit(hwnd, EventKind::AiSession(None));
        } else if id >= 100 {
            if let Some((id, _)) = sessions.get((id - 100) as usize) {
                emit(hwnd, EventKind::AiSession(Some(id.clone())));
            }
        }
    }
}
pub fn drop_files(hwnd: HWND, wp: WPARAM) {
    unsafe {
        use windows::Win32::UI::Shell::*;
        let drop = HDROP(wp.0 as *mut _);
        let count = DragQueryFileW(drop, u32::MAX, None).min(256);
        let mut paths = vec![];
        for i in 0..count {
            let mut buf = vec![0u16; DragQueryFileW(drop, i, None) as usize + 1];
            let n = DragQueryFileW(drop, i, Some(&mut buf));
            if n > 0 {
                paths.push(String::from_utf16_lossy(&buf[..n as usize]));
            }
        }
        DragFinish(drop);
        if state(hwnd).is_some_and(|s| {
            (is_studio(&s.view) && s.view.studio.accept_drop)
                || matches!(s.view.module, Some(Module::Shortcuts | Module::Folder))
        }) {
            emit(hwnd, EventKind::Drop(paths));
        }
    }
}

pub fn has_chat(v: &View) -> bool {
    v.module == Some(Module::Ai)
        || is_studio(v)
            && v.studio
                .nodes
                .iter()
                .any(|n| n.kind == Kind::AiChat && !n.hidden)
}
pub(super) fn chat_body(spec: &Spec, v: &View, a: Rect) -> Rect {
    if is_studio(v) {
        if let Some(n) = v
            .studio
            .nodes
            .iter()
            .find(|n| n.kind == Kind::AiChat && !n.hidden)
        {
            let r = node_rect(n, painting::content_rect(spec, v, a));
            let mut inner = v.clone();
            inner.module = Some(Module::Ai);
            inner.tabs.clear();
            return painting::content_rect(spec, &inner, r);
        }
    }
    painting::content_rect(spec, v, a)
}
fn data_entries<'a>(n: &Node, r: Rect, rows: &'a [Row], offset: usize) -> Vec<(Rect, &'a Row)> {
    let height = n.font_size as f32 + 16.0;
    rows.iter()
        .skip(offset)
        .take(((r.height() - 24.0) / height).max(0.0) as usize)
        .enumerate()
        .map(|(i, row)| {
            (
                Rect::from_size(
                    r.left + 6.0,
                    r.top + 24.0 + i as f32 * height,
                    r.width() - 12.0,
                    height,
                ),
                row,
            )
        })
        .collect()
}

pub(super) fn studio_body(spec: &Spec, v: &View, a: Rect, offset: usize) -> Rect {
    let b = painting::content_rect(spec, v, a);
    Rect::from_size(
        b.left,
        b.top - if has_chat(v) { 0.0 } else { offset as f32 },
        b.width(),
        b.height() * v.studio.height as f32 / 100.0,
    )
}
pub(super) fn studio_max(spec: &Spec, v: &View, a: Rect) -> usize {
    let b = painting::content_rect(spec, v, a);
    (b.height() * (v.studio.height as f32 / 100.0 - 1.0)).max(0.0) as usize
}

pub(super) fn chat_spec(spec: &Spec, v: &View) -> Spec {
    let mut inner = spec.clone();
    if is_studio(v) {
        if let Some(n) = v
            .studio
            .nodes
            .iter()
            .find(|n| n.kind == Kind::AiChat && !n.hidden)
        {
            inner.appearance.font_size = n.font_size.clamp(8, 72) as u8;
            inner.appearance.font_color = n.foreground.or(inner.appearance.font_color);
            inner.appearance.background_color = n.background.or(inner.appearance.background_color);
        }
    }
    inner
}
