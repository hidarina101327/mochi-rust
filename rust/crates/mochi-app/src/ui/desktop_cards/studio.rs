//! 桌面组合编辑器。画布和检查器共用语义化的命中几何。
use super::*;
use crate::ui::{
    draw::{Align, DrawList, TextStyle},
    layout::Edges,
    theme::Palette,
    widgets::FieldLook,
};
use mochi_core::desktop_cards::studio::{
    Action as EventAction, Binding, Kind, Node, Studio, Trigger,
};
#[derive(Debug, Default)]
pub struct Editor {
    pub selected: Option<usize>,
    pub event: usize,
    pub picker: Option<Picker>,
    pub picker_scroll: usize,
    pub drag: Option<(f32, f32, Node, bool)>,
    undo: Vec<Studio>,
    redo: Vec<Studio>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    Title,
    Target,
    X,
    Y,
    Width,
    Height,
    Font,
    Foreground,
    Background,
    EventTarget,
    EventValue,
    Seconds,
    Id,
    EventName,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Picker {
    Trigger,
    Action,
    Event,
    Source,
    Target,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    PickTrigger(Trigger),
    PickAction(EventAction),
    PickEvent(usize),
    PickSource(usize),
    PickTarget(usize),
    Target,
    DismissPicker,
    Color(bool),
    ResetColor(bool),
    Add(Kind),
    Select(usize),
    Resize(usize),
    Delete,
    Duplicate,
    Undo,
    Redo,
    Up,
    Down,
    Snap,
    Border,
    Icon,
    Drop,
    EventAdd,
    EventDelete,
    EventNext,
    Trigger,
    Action,
    Source,
}
pub fn enabled(s: &State) -> bool {
    s.selected_page_ref()
        .is_some_and(|p| matches!(p.module, ModuleKind::Custom | ModuleKind::Clock))
}
fn palette_columns(body: Rect) -> usize {
    if body.width() - 248.0 >= 380.0 {
        5
    } else {
        3
    }
}
fn palette_bottom(body: Rect) -> f32 {
    body.top + 106.0 + Kind::ALL.len().div_ceil(palette_columns(body)) as f32 * 34.0
}
pub fn canvas(s: &State, body: Rect) -> Rect {
    let r = Rect::new(
        body.left,
        palette_bottom(body) + 10.0,
        body.right - 248.0,
        body.bottom - 38.0,
    );
    let (w, h) = design_size(s);
    let scale = (r.width().max(40.0) / w).min(r.height().max(40.0) / h);
    Rect::from_size(
        r.left + (r.width() - w * scale).max(0.0) / 2.0,
        r.top,
        w * scale,
        h * scale,
    )
}
fn design_size(s: &State) -> (f32, f32) {
    s.selected_card_ref()
        .zip(s.selected_page_ref())
        .map(|(c, p)| crate::desktop_window::widgets::design_size(c, p))
        .unwrap_or((448.0, 410.0))
}
pub fn inspector(body: Rect) -> Rect {
    Rect::new(body.right - 232.0, body.top + 86.0, body.right, body.bottom)
}
fn selected(s: &State) -> Option<&Node> {
    s.selected_page_ref()?
        .studio
        .nodes
        .get(s.studio_editor.selected?)
}
fn fields(s: &State, body: Rect) -> Vec<(Rect, Property, &'static str)> {
    let Some(n) = selected(s) else {
        return vec![];
    };
    let r = inspector(body).inset(Edges::xy(12.0, 0.0));
    let mut y = r.top + 32.0 - s.options_scroll;
    let mut out = vec![];
    let mut full = |prop, label| {
        out.push((
            Rect::from_size(r.left, y + 17.0, r.width(), 28.0),
            prop,
            label,
        ));
        y += 54.0;
    };
    full(Property::Title, "名称 / 文字");
    if matches!(
        n.kind,
        Kind::Clock | Kind::Date | Kind::Shortcut | Kind::Data
    ) {
        full(
            Property::Target,
            if matches!(n.kind, Kind::Clock | Kind::Date) {
                "时间格式"
            } else {
                "目标"
            },
        );
    }
    for pair in [
        [(Property::X, "X (%)"), (Property::Y, "Y (%)")],
        [(Property::Width, "宽 (%)"), (Property::Height, "高 (%)")],
    ] {
        for (i, (prop, label)) in pair.into_iter().enumerate() {
            out.push((
                Rect::from_size(
                    r.left + i as f32 * (r.width() + 8.0) / 2.0,
                    y + 17.0,
                    (r.width() - 8.0) / 2.0,
                    28.0,
                ),
                prop,
                label,
            ));
        }
        y += 54.0;
    }
    for (prop, label) in [
        (Property::Font, "字号"),
        (Property::Foreground, "文字颜色"),
        (Property::Background, "背景颜色"),
    ] {
        out.push((
            Rect::from_size(r.left, y + 17.0, r.width(), 28.0),
            prop,
            label,
        ));
        y += 54.0;
    }
    y += 182.0;
    if let Some(b) = n.events.get(s.studio_editor.event) {
        for (prop, label, show) in [
            (Property::EventTarget, "事件目标", true),
            (
                Property::EventValue,
                "事件值",
                matches!(b.action, EventAction::SetText),
            ),
            (
                Property::Seconds,
                "间隔（秒）",
                b.trigger == Trigger::Interval,
            ),
            (
                Property::EventName,
                "自定义事件名",
                b.trigger == Trigger::Custom,
            ),
        ] {
            if show {
                out.push((
                    Rect::from_size(r.left, y + 17.0, r.width(), 28.0),
                    prop,
                    label,
                ));
                y += 54.0;
            }
        }
    }
    out
}
fn event_top(s: &State, body: Rect) -> f32 {
    fields(s, body)
        .iter()
        .find(|(_, p, _)| *p == Property::Background)
        .map_or(inspector(body).top + 20.0, |(r, _, _)| r.bottom + 16.0)
}
const SOURCES: [(&str, &str); 10] = [
    ("favorites", "收藏"),
    ("recent", "最近"),
    ("schedule", "日程"),
    ("inbox", "收件箱"),
    ("knowledge", "知识库"),
    ("base", "多维表格"),
    ("canvas", "画布"),
    ("exam", "试卷"),
    ("automations", "自动化"),
    ("home", "工作台"),
];
fn targets(s: &State) -> Vec<(String, String)> {
    let Some(n) = selected(s) else { return vec![] };
    let Some(b) = n.events.get(s.studio_editor.event) else {
        return vec![];
    };
    match b.action {
        EventAction::OpenModule => ModuleKind::ALL
            .iter()
            .map(|m| (m.wire_name().into(), m.label().into()))
            .collect(),
        EventAction::SwitchPage => s
            .selected_card_ref()
            .map(|c| {
                c.pages
                    .iter()
                    .map(|p| (p.id.clone(), p.title.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        EventAction::SetText | EventAction::ToggleVisible => s
            .selected_page_ref()
            .unwrap()
            .studio
            .nodes
            .iter()
            .map(|n| (n.id.clone(), n.title.clone()))
            .collect(),
        _ => vec![],
    }
}
fn choices(s: &State) -> Vec<(String, Command)> {
    match s.studio_editor.picker {
        Some(Picker::Trigger) => Trigger::ALL
            .iter()
            .map(|t| (t.label().into(), Command::PickTrigger(*t)))
            .collect(),
        Some(Picker::Action) => EventAction::ALL
            .iter()
            .map(|a| (a.label().into(), Command::PickAction(*a)))
            .collect(),
        Some(Picker::Event) => selected(s)
            .map(|n| {
                n.events
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        (
                            format!("{} · {} → {}", i + 1, b.trigger.label(), b.action.label()),
                            Command::PickEvent(i),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        Some(Picker::Source) => SOURCES
            .iter()
            .enumerate()
            .map(|(i, (_, label))| (label.to_string(), Command::PickSource(i)))
            .collect(),
        Some(Picker::Target) => targets(s)
            .iter()
            .enumerate()
            .map(|(i, (_, label))| (label.clone(), Command::PickTarget(i)))
            .collect(),
        None => vec![],
    }
}
fn chosen(s: &State, c: Command) -> bool {
    let n = selected(s);
    let b = n.and_then(|n| n.events.get(s.studio_editor.event));
    match c {
        Command::PickTrigger(t) => b.is_some_and(|b| b.trigger == t),
        Command::PickAction(a) => b.is_some_and(|b| b.action == a),
        Command::PickEvent(i) => i == s.studio_editor.event,
        Command::PickSource(i) => n.is_some_and(|n| n.target == SOURCES[i].0),
        Command::PickTarget(i) => targets(s)
            .get(i)
            .is_some_and(|(t, _)| b.is_some_and(|b| b.target == *t)),
        _ => false,
    }
}
pub fn picker_key(s: &mut State, key: u16, shift: bool) {
    let rows = choices(s);
    if rows.is_empty() {
        if key == 27 {
            s.studio_editor.picker = None;
        }
        return;
    }
    let at = rows
        .iter()
        .position(|(_, c)| Some(Hit::Studio(*c)) == s.focused)
        .or_else(|| rows.iter().position(|(_, c)| chosen(s, *c)))
        .unwrap_or(0);
    match key {
        27 => s.studio_editor.picker = None,
        13 | 32 => command(s, rows[at].1),
        9 | 38 | 40 => {
            let previous = key == 38 || (key == 9 && shift);
            let index = if previous {
                (at + rows.len() - 1) % rows.len()
            } else {
                (at + 1) % rows.len()
            };
            s.focused = Some(Hit::Studio(rows[index].1));
            s.studio_editor.picker_scroll = index.saturating_sub(2);
        }
        _ => {}
    }
}
fn popup(s: &State, body: Rect) -> Rect {
    let i = inspector(body);
    let height = (choices(s).len().max(1) as f32 * 32.0 + 44.0).min(i.height());
    Rect::from_size(i.left - 64.0, i.top, i.width() + 64.0, height)
}
pub fn controls(s: &State, body: Rect) -> Vec<(Rect, Hit)> {
    if s.studio_editor.picker.is_some() {
        let r = popup(s, body);
        let mut out = vec![(body, Hit::Studio(Command::DismissPicker))];
        for (i, (_, c)) in choices(s)
            .into_iter()
            .skip(s.studio_editor.picker_scroll)
            .enumerate()
        {
            let rr = Rect::from_size(
                r.left + 8.0,
                r.top + 36.0 + i as f32 * 32.0,
                r.width() - 16.0,
                30.0,
            );
            if rr.bottom <= r.bottom {
                out.push((rr, Hit::Studio(c)));
            }
        }
        return out;
    }
    let mut out = vec![];
    let cols = palette_columns(body);
    let width = (body.width() - 248.0) / cols as f32;
    for (i, kind) in Kind::ALL.into_iter().enumerate() {
        out.push((
            Rect::from_size(
                body.left + (i % cols) as f32 * width,
                body.top + 106.0 + (i / cols) as f32 * 34.0,
                width - 6.0,
                28.0,
            ),
            Hit::Studio(Command::Add(kind)),
        ));
    }
    let r = canvas(s, body);
    if let Some(p) = s.selected_page_ref() {
        for (i, n) in p.studio.nodes.iter().enumerate() {
            let nr = crate::desktop_window::widgets::node_rect(n, r);
            out.push((nr, Hit::Studio(Command::Select(i))));
            if s.studio_editor.selected == Some(i) {
                out.push((
                    Rect::from_size(nr.right - 10.0, nr.bottom - 10.0, 12.0, 12.0),
                    Hit::Studio(Command::Resize(i)),
                ));
            }
        }
    }
    for (i, c) in [
        Command::Undo,
        Command::Redo,
        Command::Duplicate,
        Command::Delete,
        Command::Up,
        Command::Down,
    ]
    .into_iter()
    .enumerate()
    {
        out.push((
            Rect::from_size(
                body.left + i as f32 * (body.width() - 248.0) / 6.0,
                body.bottom - 28.0,
                (body.width() - 248.0) / 6.0 - 3.0,
                26.0,
            ),
            Hit::Studio(c),
        ));
    }
    if selected(s).is_none() {
        return out;
    }
    for (r, prop, _) in fields(s, body) {
        let c = match prop {
            Property::Foreground => Some(Command::Color(false)),
            Property::Background => Some(Command::Color(true)),
            _ => None,
        };
        if let Some(c) = c {
            out.push((
                Rect::new(r.left, r.top, r.right - 30.0, r.bottom),
                Hit::Studio(c),
            ));
            out.push((
                Rect::from_size(r.right - 28.0, r.top, 28.0, r.height()),
                Hit::Studio(Command::ResetColor(prop == Property::Background)),
            ));
        } else {
            out.push((r, Hit::EditPreference(Field::Studio(prop))));
        }
    }
    let i = inspector(body).inset(Edges::xy(12.0, 0.0));
    let top = event_top(s, body);
    for (n, c) in [Command::Border, Command::Icon, Command::Snap, Command::Drop]
        .into_iter()
        .enumerate()
    {
        out.push((
            Rect::from_size(
                i.left + (n % 2) as f32 * (i.width() + 8.0) / 2.0,
                top + (n / 2) as f32 * 30.0,
                (i.width() - 8.0) / 2.0,
                26.0,
            ),
            Hit::Studio(c),
        ));
    }
    out.push((
        Rect::from_size(i.left, top + 64.0, i.width() - 58.0, 28.0),
        Hit::Studio(Command::EventNext),
    ));
    out.push((
        Rect::from_size(i.right - 54.0, top + 64.0, 26.0, 28.0),
        Hit::Studio(Command::EventAdd),
    ));
    out.push((
        Rect::from_size(i.right - 26.0, top + 64.0, 26.0, 28.0),
        Hit::Studio(Command::EventDelete),
    ));
    if selected(s).is_some_and(|n| !n.events.is_empty()) {
        for (n, c) in [Command::Trigger, Command::Action].into_iter().enumerate() {
            out.push((
                Rect::from_size(i.left, top + 98.0 + n as f32 * 34.0, i.width(), 28.0),
                Hit::Studio(c),
            ));
        }
    }
    if selected(s).is_some_and(|n| n.kind == Kind::Data) {
        out.push((
            Rect::from_size(body.right - 248.0 - 100.0, body.top + 80.0, 100.0, 22.0),
            Hit::Studio(Command::Source),
        ));
    }
    if !targets(s).is_empty() {
        if let Some((r, _, _)) = fields(s, body)
            .into_iter()
            .find(|(_, p, _)| *p == Property::EventTarget)
        {
            out.push((
                Rect::from_size(r.right - 26.0, r.top, 26.0, r.height()),
                Hit::Studio(Command::Target),
            ));
        }
    }
    out
}
pub fn value(s: &State, prop: Property) -> String {
    let Some(n) = s
        .selected_page_ref()
        .and_then(|p| p.studio.nodes.get(s.studio_editor.selected?))
    else {
        return String::new();
    };
    let b = n.events.get(s.studio_editor.event);
    match prop {
        Property::Title => n.title.clone(),
        Property::Target => n.target.clone(),
        Property::Id => n.id.clone(),
        Property::X => format!("{:.2}", n.x as f32 / 100.0),
        Property::Y => format!("{:.2}", n.y as f32 / 100.0),
        Property::Width => format!("{:.2}", n.width as f32 / 100.0),
        Property::Height => format!("{:.2}", n.height as f32 / 100.0),
        Property::Font => n.font_size.to_string(),
        Property::Foreground => n
            .foreground
            .map(|c| format!("#{c:06X}"))
            .unwrap_or_default(),
        Property::Background => n
            .background
            .map(|c| format!("#{c:06X}"))
            .unwrap_or_default(),
        Property::EventTarget => b.map(|b| b.target.clone()).unwrap_or_default(),
        Property::EventValue => b.map(|b| b.value.clone()).unwrap_or_default(),
        Property::EventName => b.map(|b| b.name.clone()).unwrap_or_default(),
        Property::Seconds => b.map(|b| b.seconds.to_string()).unwrap_or_default(),
    }
}
pub fn checkpoint(s: &mut State) {
    if let Some(p) = s.selected_page_ref() {
        let v = p.studio.clone();
        if s.studio_editor.undo.last() != Some(&v) {
            s.studio_editor.undo.push(v);
            if s.studio_editor.undo.len() > 60 {
                s.studio_editor.undo.remove(0);
            }
            s.studio_editor.redo.clear();
        }
    }
}
pub fn set(s: &mut State, prop: Property, text: &str) {
    let selected = s.studio_editor.selected;
    let event = s.studio_editor.event;
    let Some(n) = s
        .selected_page_mut()
        .and_then(|p| p.studio.nodes.get_mut(selected?))
    else {
        return;
    };
    let pos = || {
        text.parse::<f32>()
            .ok()
            .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 100.0)
            .map(|v| (v * 100.0).round() as u16)
    };
    match prop {
        Property::Title => n.title = text.chars().take(4096).collect(),
        Property::Target => n.target = text.chars().take(4096).collect(),
        Property::Id => return,
        Property::X => {
            if let Some(v) = pos() {
                n.x = v;
            }
        }
        Property::Y => {
            if let Some(v) = pos() {
                n.y = v;
            }
        }
        Property::Width => {
            if let Some(v) = pos() {
                n.width = v;
            }
        }
        Property::Height => {
            if let Some(v) = pos() {
                n.height = v;
            }
        }
        Property::Font => {
            if let Ok(v) = text.parse::<u16>() {
                n.font_size = v.clamp(8, 240);
            }
        }
        Property::Foreground | Property::Background => {
            let color = if text.trim().is_empty() {
                None
            } else {
                let Ok(v) = u32::from_str_radix(text.trim_start_matches('#'), 16) else {
                    return;
                };
                if v > 0xffffff {
                    return;
                }
                Some(v)
            };
            if prop == Property::Foreground {
                n.foreground = color;
            } else {
                n.background = color;
            }
        }
        Property::EventTarget | Property::EventValue | Property::Seconds | Property::EventName => {
            if let Some(b) = n.events.get_mut(event) {
                match prop {
                    Property::EventName => b.name = text.into(),
                    Property::EventTarget => b.target = text.into(),
                    Property::EventValue => b.value = text.into(),
                    Property::Seconds => {
                        if let Ok(v) = text.parse::<u32>() {
                            b.seconds = v.clamp(1, 86400);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    n.clamp();
    s.dirty = true;
}
pub fn command(s: &mut State, c: Command) {
    if c == Command::DismissPicker {
        s.studio_editor.picker = None;
        return;
    }
    let picker = match c {
        Command::Trigger => Some(Picker::Trigger),
        Command::Action => Some(Picker::Action),
        Command::EventNext => Some(Picker::Event),
        Command::Source => Some(Picker::Source),
        Command::Target => Some(Picker::Target),
        _ => None,
    };
    if let Some(picker) = picker {
        s.studio_editor.picker_scroll = 0;
        s.studio_editor.picker = Some(picker);
        s.focus_field = None;
        return;
    }
    let target = if let Command::PickTarget(i) = c {
        targets(s).get(i).map(|v| v.0.clone())
    } else {
        None
    };
    s.studio_editor.picker = None;

    if matches!(c, Command::Select(_) | Command::Resize(_)) {
        if let Command::Select(i) | Command::Resize(i) = c {
            s.studio_editor.selected = Some(i);
            s.options_scroll = 0.0;
            s.studio_editor.event = 0;
        }
        return;
    }
    if matches!(c, Command::Undo | Command::Redo) {
        let next = if c == Command::Undo {
            s.studio_editor.undo.pop()
        } else {
            s.studio_editor.redo.pop()
        };
        if let Some(next) = next {
            if let Some(p) = s.selected_page_mut() {
                let old = std::mem::replace(&mut p.studio, next);
                if c == Command::Undo {
                    s.studio_editor.redo.push(old);
                } else {
                    s.studio_editor.undo.push(old);
                }
                s.dirty = true;
                s.studio_editor.selected = None;
            }
        }
        return;
    }
    checkpoint(s);
    let selected = s.studio_editor.selected;
    let event = s.studio_editor.event;
    let Some(p) = s.selected_page_mut() else {
        return;
    };
    let nodes = &mut p.studio.nodes;
    match c {
        Command::Add(kind) => {
            if nodes.len() < 256
                && !(kind == Kind::AiChat && nodes.iter().any(|n| n.kind == Kind::AiChat))
            {
                let mut n = Node::new(kind);
                n.x = ((nodes.len() % 6) * 500) as u16;
                n.y = ((nodes.len() % 6) * 500) as u16;
                n.clamp();
                nodes.push(n);
                s.studio_editor.selected = Some(nodes.len() - 1);
            }
        }
        Command::Delete => {
            if let Some(i) = selected.filter(|i| *i < nodes.len()) {
                nodes.remove(i);
                s.studio_editor.selected = None;
            }
        }
        Command::Duplicate => {
            if let Some(i) = selected.filter(|i| *i < nodes.len()) {
                if nodes.len() < 256 && nodes[i].kind != Kind::AiChat {
                    let mut n = nodes[i].clone();
                    n.id = Node::default().id;
                    n.x = n.x.saturating_add(250);
                    n.y = n.y.saturating_add(250);
                    n.clamp();
                    nodes.push(n);
                    s.studio_editor.selected = Some(nodes.len() - 1);
                }
            }
        }
        Command::Up | Command::Down => {
            if let Some(i) = selected {
                let j = if c == Command::Up {
                    i.saturating_add(1)
                } else {
                    i.saturating_sub(1)
                };
                if j < nodes.len() && i < nodes.len() {
                    nodes.swap(i, j);
                    s.studio_editor.selected = Some(j);
                }
            }
        }
        Command::Snap => p.studio.snap = if p.studio.snap == 0 { 250 } else { 0 },
        Command::Drop => p.studio.accept_drop = !p.studio.accept_drop,
        _ => {
            if let Some(n) = selected.and_then(|i| nodes.get_mut(i)) {
                match c {
                    Command::ResetColor(background) => {
                        if background {
                            n.background = None;
                        } else {
                            n.foreground = None;
                        }
                    }
                    Command::PickSource(i) => n.target = SOURCES[i.min(SOURCES.len() - 1)].0.into(),
                    Command::PickEvent(i) => {
                        s.studio_editor.event = i.min(n.events.len().saturating_sub(1))
                    }
                    Command::PickTrigger(trigger) => {
                        if let Some(b) = n.events.get_mut(event) {
                            b.trigger = trigger;
                        }
                    }
                    Command::PickAction(action) => {
                        if let Some(b) = n.events.get_mut(event) {
                            b.action = action;
                        }
                    }
                    Command::PickTarget(_) => {
                        if let (Some(b), Some(target)) = (n.events.get_mut(event), target) {
                            b.target = target;
                        }
                    }
                    Command::Border => n.border = !n.border,
                    Command::Icon => n.show_icon = !n.show_icon,
                    Command::EventAdd => {
                        if n.events.len() < 32 {
                            n.events.push(Binding::default());
                            s.studio_editor.event = n.events.len() - 1;
                        }
                    }
                    Command::EventDelete => {
                        if event < n.events.len() {
                            n.events.remove(event);
                            s.studio_editor.event = event.saturating_sub(1);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    s.dirty = true;
}
pub fn begin(s: &mut State, layout: &Layout, x: f32, y: f32) -> bool {
    if s.editor_tab != 2 {
        return false;
    }
    let hit = layout.hit(x, y);
    if let Some(Hit::Studio(c @ (Command::Select(i) | Command::Resize(i)))) = hit {
        s.commit_focused_field();
        s.focus_field = None;
        checkpoint(s);
        command(s, c);
        let n = s.selected_page_ref().unwrap().studio.nodes[i].clone();
        s.studio_editor.drag = Some((x, y, n, matches!(c, Command::Resize(_))));
        return true;
    }
    false
}
pub fn drag(s: &mut State, layout: &Layout, x: f32, y: f32) -> bool {
    let Some((sx, sy, n, resize)) = s.studio_editor.drag.clone() else {
        return false;
    };
    let r = canvas(s, layout.right_body);
    let selected = s.studio_editor.selected;
    let Some(p) = s.selected_page_mut() else {
        return false;
    };
    let snap = p.studio.snap as i32;
    let quant = |v: i32| {
        let v = if snap > 0 {
            ((v as f32 / snap as f32).round() as i32) * snap
        } else {
            v
        };
        v.clamp(0, 10000) as u16
    };
    let dx = ((x - sx) / r.width() * 10000.0) as i32;
    let dy = ((y - sy) / r.height() * 10000.0) as i32;
    if let Some(node) = selected.and_then(|i| p.studio.nodes.get_mut(i)) {
        if resize {
            node.width = quant(n.width as i32 + dx).max(250);
            node.height = quant(n.height as i32 + dy).max(100);
        } else {
            node.x = quant(n.x as i32 + dx);
            node.y = quant(n.y as i32 + dy);
        }
        node.clamp();
        s.dirty = true;
    }
    true
}
fn label(s: &State, c: Command) -> String {
    let n = selected(s);
    let b = n.and_then(|n| n.events.get(s.studio_editor.event));
    match c {
        Command::Add(k) => k.label().into(),
        Command::Delete => "删除".into(),
        Command::Duplicate => "复制".into(),
        Command::Undo => "撤销".into(),
        Command::Redo => "重做".into(),
        Command::Up => "上移".into(),
        Command::Down => "下移".into(),
        Command::Border => format!(
            "{} 边框",
            if n.is_some_and(|n| n.border) {
                "✓"
            } else {
                "○"
            }
        ),
        Command::Icon => format!(
            "{} 图标",
            if n.is_some_and(|n| n.show_icon) {
                "✓"
            } else {
                "○"
            }
        ),
        Command::Snap => format!(
            "{} 吸附",
            if s.selected_page_ref().unwrap().studio.snap > 0 {
                "✓"
            } else {
                "○"
            }
        ),
        Command::Drop => format!(
            "{} 接收拖入",
            if s.selected_page_ref().unwrap().studio.accept_drop {
                "✓"
            } else {
                "○"
            }
        ),
        Command::EventAdd => "＋".into(),
        Command::EventDelete => "−".into(),
        Command::EventNext => b
            .map(|_| {
                format!(
                    "事件 {} / {}",
                    s.studio_editor.event + 1,
                    n.unwrap().events.len()
                )
            })
            .unwrap_or_else(|| "添加交互事件".into()),
        Command::Trigger => format!(
            "触发 · {}",
            b.map(|b| b.trigger.label()).unwrap_or("请选择")
        ),
        Command::Action => format!("动作 · {}", b.map(|b| b.action.label()).unwrap_or("请选择")),
        Command::Source => "选择数据源".into(),
        Command::Target => "⌄".into(),
        _ => String::new(),
    }
}
pub fn paint(list: &mut DrawList, s: &mut State, body: Rect, p: &Palette) {
    use crate::ui::{icons::Icon, text, theme};
    let r = canvas(s, body);
    let i = inspector(body);
    list.text(
        Rect::from_size(body.left, body.top + 80.0, 180.0, 22.0),
        "组件",
        TextStyle::Caption,
        p.muted,
    );
    list.rounded_rect(
        Rect::new(
            body.left - 4.0,
            r.top - 4.0,
            body.right - 244.0,
            body.bottom - 34.0,
        ),
        10.0,
        p.surface_muted,
    );
    let background = s
        .selected_card_ref()
        .and_then(|c| c.appearance.background_color)
        .unwrap_or(p.surface);
    list.rounded_rect(r, 8.0, background);
    list.rounded_border(r, 8.0, p.border);
    let foreground = s
        .selected_card_ref()
        .and_then(|c| c.appearance.font_color)
        .unwrap_or_else(|| {
            if ((background >> 16) & 255) * 299
                + ((background >> 8) & 255) * 587
                + (background & 255) * 114
                < 145000
            {
                0xf0f2f4
            } else {
                0x252b32
            }
        });
    list.push_clip(r);
    if let Some(page) = s.selected_page_ref() {
        if page.studio.snap > 0 {
            for x in (500..10000).step_by(500) {
                for y in (500..10000).step_by(500) {
                    list.rounded_rect(
                        Rect::from_size(
                            r.left + r.width() * x as f32 / 10000.0,
                            r.top + r.height() * y as f32 / 10000.0,
                            1.3,
                            1.3,
                        ),
                        0.65,
                        theme::mix(foreground, background, 0.12),
                    );
                }
            }
        }
        for (index, n) in page.studio.nodes.iter().enumerate() {
            let nr = crate::desktop_window::widgets::node_rect(n, r);
            let mut preview = n.clone();
            preview.font_size = (n.font_size as f32 * r.width() / design_size(s).0)
                .round()
                .max(1.0) as u16;
            crate::desktop_window::widgets::paint_node(
                list,
                &preview,
                nr,
                foreground,
                p.border,
                &crate::desktop_window::widgets::Live {
                    now: chrono::Local::now().timestamp(),
                    timer: "25:00".into(),
                    ..Default::default()
                },
                &s.icon_workspace,
                false,
            );
            if s.studio_editor.selected == Some(index) {
                list.rounded_border(nr, 4.0, p.muted);
                let handle = Rect::from_size(nr.right - 4.0, nr.bottom - 4.0, 8.0, 8.0);
                list.rounded_rect(handle, 2.0, p.surface);
                list.rounded_border(handle, 2.0, p.foreground);
            }
        }
        if page.studio.nodes.is_empty() {
            list.text_aligned(
                r,
                "点击上方组件开始设计",
                TextStyle::Label,
                p.muted,
                Align::Center,
            );
        }
    }
    list.pop_clip();
    list.rounded_rect(i, 10.0, theme::mix(p.surface_muted, p.surface, 0.35));
    list.rounded_border(i, 10.0, p.border);
    list.push_clip(i);
    list.text(
        Rect::from_size(i.left + 12.0, i.top + 8.0, i.width() - 24.0, 22.0),
        selected(s).map(|n| n.kind.label()).unwrap_or("组件属性"),
        TextStyle::Title,
        p.foreground,
    );
    if selected(s).is_none() {
        list.text_aligned(
            Rect::new(i.left + 20.0, i.top + 64.0, i.right - 20.0, i.top + 160.0),
            "选择画布中的组件
编辑外观与交互",
            TextStyle::Caption,
            p.muted,
            Align::Center,
        );
    }
    list.push_clip(Rect::new(i.left, i.top + 32.0, i.right, i.bottom));
    for (field, prop, title) in fields(s, body) {
        list.text(
            Rect::from_size(field.left, field.top - 17.0, field.width(), 17.0),
            title,
            TextStyle::Caption,
            p.muted,
        );
        if matches!(prop, Property::Foreground | Property::Background) {
            list.rounded_rect(field, 6.0, p.surface);
            list.rounded_border(field, 6.0, p.border);
            let color = selected(s).and_then(|n| {
                if prop == Property::Background {
                    n.background
                } else {
                    n.foreground
                }
            });
            let swatch = Rect::from_size(field.left + 7.0, field.top + 6.0, 16.0, 16.0);
            list.rounded_rect(
                swatch,
                4.0,
                color.unwrap_or(if prop == Property::Background {
                    background
                } else {
                    foreground
                }),
            );
            list.rounded_border(swatch, 4.0, p.border);
            list.text(
                Rect::new(
                    field.left + 32.0,
                    field.top,
                    field.right - 26.0,
                    field.bottom,
                ),
                color
                    .map(|c| format!("#{c:06X}"))
                    .unwrap_or_else(|| "跟随卡片".into()),
                TextStyle::Caption,
                p.foreground,
            );
            list.icon_centered(
                Rect::from_size(field.right - 26.0, field.top, 26.0, field.height()),
                Icon::ROTATE_CCW,
                12.0,
                p.muted,
            );
        } else if s.focus_field == Some(Field::Studio(prop)) {
            s.preference_text
                .paint(list, field, true, p, FieldLook::dialog(p));
        } else {
            list.rounded_rect(field, 6.0, p.surface);
            list.rounded_border(field, 6.0, p.border);
            list.text(
                field.inset(Edges::xy(8.0, 0.0)),
                text::ellipsize(&value(s, prop), TextStyle::Caption, field.width() - 16.0),
                TextStyle::Caption,
                p.foreground,
            );
        }
    }
    list.pop_clip();
    list.pop_clip();
    // 选择器覆盖其上时，常规控件仍照常绘制。
    let picker = s.studio_editor.picker.take();
    let base = controls(s, body);
    s.studio_editor.picker = picker;
    for (r, h) in base {
        let Hit::Studio(c) = h else { continue };
        if matches!(
            c,
            Command::Select(_) | Command::Resize(_) | Command::Color(_) | Command::ResetColor(_)
        ) {
            continue;
        }
        let inside = r.left >= i.left;
        if inside {
            list.push_clip(Rect::new(i.left, i.top + 32.0, i.right, i.bottom));
        }
        list.rounded_rect(
            r,
            6.0,
            if s.hover == Some(h) {
                p.surface_muted
            } else {
                p.surface
            },
        );
        list.rounded_border(r, 6.0, p.border);
        let is_picker = matches!(
            c,
            Command::Trigger | Command::Action | Command::EventNext | Command::Source
        );
        let tr = if is_picker {
            Rect::new(r.left + 8.0, r.top, r.right - 22.0, r.bottom)
        } else {
            r
        };
        list.text_aligned(
            tr,
            text::ellipsize(&label(s, c), TextStyle::Caption, tr.width() - 4.0),
            TextStyle::Caption,
            p.foreground,
            if is_picker {
                Align::Leading
            } else {
                Align::Center
            },
        );
        if is_picker {
            list.icon_centered(
                Rect::from_size(r.right - 21.0, r.top, 18.0, r.height()),
                Icon::CHEVRON_DOWN,
                12.0,
                p.muted,
            );
        }
        if inside {
            list.pop_clip();
        }
    }
    if s.studio_editor.picker.is_some() {
        let r = popup(s, body);
        list.rounded_rect_alpha(
            Rect::new(r.left - 3.0, r.top + 3.0, r.right + 3.0, r.bottom + 6.0),
            10.0,
            0,
            0.12,
        );
        list.rounded_rect(r, 10.0, p.surface);
        list.rounded_border(r, 10.0, p.border);
        list.text(
            Rect::from_size(r.left + 12.0, r.top + 6.0, r.width() - 24.0, 24.0),
            "选择一项",
            TextStyle::Title,
            p.foreground,
        );
        list.push_clip(r);
        for (index, (label, c)) in choices(s)
            .into_iter()
            .skip(s.studio_editor.picker_scroll)
            .enumerate()
        {
            let rr = Rect::from_size(
                r.left + 8.0,
                r.top + 36.0 + index as f32 * 32.0,
                r.width() - 16.0,
                30.0,
            );
            if s.hover == Some(Hit::Studio(c)) || s.focused == Some(Hit::Studio(c)) {
                list.rounded_rect(rr, 6.0, p.surface_muted);
            }
            if chosen(s, c) {
                list.icon_centered(
                    Rect::from_size(rr.right - 24.0, rr.top, 22.0, rr.height()),
                    Icon::CHECK,
                    13.0,
                    p.foreground,
                );
            }
            list.text(
                Rect::new(rr.left + 8.0, rr.top, rr.right - 26.0, rr.bottom),
                text::ellipsize(&label, TextStyle::Caption, rr.width() - 34.0),
                TextStyle::Caption,
                p.foreground,
            );
        }
        list.pop_clip();
    }
}

pub fn scroll_picker(s: &mut State, body: Rect, pixels: f32) -> bool {
    if s.studio_editor.picker.is_none() {
        return false;
    }
    let capacity = ((popup(s, body).height() - 44.0) / 32.0).floor().max(1.0) as usize;
    let max = choices(s).len().saturating_sub(capacity);
    s.studio_editor.picker_scroll = if pixels > 0.0 {
        s.studio_editor.picker_scroll.saturating_sub(1)
    } else {
        (s.studio_editor.picker_scroll + 1).min(max)
    };
    true
}
pub fn content_height(s: &State, body: Rect) -> f32 {
    let bottom = fields(s, body)
        .iter()
        .map(|(r, _, _)| r.bottom)
        .fold(event_top(s, body) + 164.0, f32::max);
    if selected(s).is_some() {
        bottom + s.options_scroll - inspector(body).top + 16.0
    } else {
        0.0
    }
}
