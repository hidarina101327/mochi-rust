//! 墨池 AI 独立工作区的会话侧栏。正文复用 assistant 的对话状态和运行时。
use super::{
    draw::{DrawList, TextStyle},
    icons::Icon,
    layout::Rect,
    text,
    theme::Palette,
    widgets::{FieldLook, TextField},
};
use chrono::{Local, TimeZone};
use mochi_core::ai::session::{AiProject, AiSessionMeta};
pub struct State {
    pub query: TextField,
    pub scroll: f32,
    pub hover: Option<String>,
    pub projects: Vec<AiProject>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            query: TextField::new("搜索会话"),
            scroll: 0.0,
            hover: None,
            projects: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    New,
    Query,
    Open(String),
    Delete(String),
}
#[derive(Default)]
pub struct Layout {
    pub entries: Vec<(Rect, Hit)>,
    pub list: Rect,
    pub height: f32,
    pub rows: Vec<(Rect, AiSessionMeta)>,
}
impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, h)| {
                r.contains(x, y) && (matches!(h, Hit::New | Hit::Query) || self.list.contains(x, y))
            })
            .map(|(_, h)| h.clone())
    }
    pub fn max_scroll(&self) -> f32 {
        (self.height - self.list.height()).max(0.0)
    }
    pub fn query(&self) -> Option<Rect> {
        self.entries
            .iter()
            .find(|(_, h)| *h == Hit::Query)
            .map(|(r, _)| *r)
    }
}
pub fn layout(s: &State, sessions: &[AiSessionMeta], area: Rect) -> Layout {
    let mut lay = Layout {
        list: Rect::new(area.left, area.top + 86.0, area.right, area.bottom),
        ..Default::default()
    };
    lay.entries.push((
        Rect::new(
            area.right - 42.0,
            area.top + 6.0,
            area.right - 12.0,
            area.top + 36.0,
        ),
        Hit::New,
    ));
    lay.entries.push((
        Rect::new(
            area.left + 10.0,
            area.top + 42.0,
            area.right - 10.0,
            area.top + 76.0,
        ),
        Hit::Query,
    ));
    let query = s.query.text().trim().to_lowercase();
    let mut sessions = sessions
        .iter()
        .filter(|m| {
            m.title.to_lowercase().contains(&query)
                || m.project_id.as_ref().is_some_and(|id| {
                    s.projects
                        .iter()
                        .any(|p| &p.id == id && p.name.to_lowercase().contains(&query))
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    sessions.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.updated_at.cmp(&a.updated_at))
    });
    lay.height = 16.0 + sessions.len() as f32 * 48.0;
    for (i, m) in sessions.into_iter().enumerate() {
        let y = lay.list.top + 4.0 + i as f32 * 48.0 - s.scroll;
        let r = Rect::new(area.left + 8.0, y, area.right - 8.0, y + 48.0);
        lay.entries.push((r, Hit::Open(m.id.clone())));
        lay.entries.push((
            Rect::new(r.right - 34.0, r.top + 10.0, r.right - 6.0, r.top + 38.0),
            Hit::Delete(m.id.clone()),
        ));
        lay.rows.push((r, m));
    }
    lay
}
pub fn paint(
    list: &mut DrawList,
    area: Rect,
    s: &mut State,
    lay: &Layout,
    active: Option<&str>,
    focused: bool,
    streaming: bool,
    p: &Palette,
) {
    list.push_clip(area);
    list.rect(area, p.area_sidebar_default);
    list.icon_centered(
        Rect::new(
            area.left + 14.0,
            area.top + 8.0,
            area.left + 34.0,
            area.top + 34.0,
        ),
        Icon::BOT,
        17.0,
        p.foreground,
    );
    list.text(
        Rect::new(
            area.left + 42.0,
            area.top + 10.0,
            area.right - 48.0,
            area.top + 34.0,
        ),
        "墨池 AI",
        TextStyle::Label,
        p.foreground,
    );
    for (r, h) in &lay.entries {
        if *h == Hit::New {
            list.rounded_border(*r, 6.0, p.border);
            list.icon_centered(
                *r,
                Icon::PLUS,
                16.0,
                if streaming { p.border } else { p.muted },
            );
        }
    }
    if let Some(r) = lay.query() {
        s.query.paint(list, r, focused, p, FieldLook::search(p));
    }
    list.push_clip(lay.list);
    if lay.rows.is_empty() {
        list.text(
            Rect::new(
                area.left + 18.0,
                lay.list.top + 18.0,
                area.right - 18.0,
                lay.list.top + 42.0,
            ),
            if s.query.is_empty() {
                "还没有会话"
            } else {
                "没有匹配的会话"
            },
            TextStyle::Caption,
            p.muted,
        );
    }
    for (r, m) in &lay.rows {
        if r.bottom < lay.list.top || r.top > lay.list.bottom {
            continue;
        }
        if active == Some(m.id.as_str()) {
            list.rounded_rect(*r, 6.0, p.surface);
            list.rect(
                Rect::from_size(r.left, r.top + 12.0, 2.0, 24.0),
                p.foreground,
            );
        } else if s.hover.as_deref() == Some(&m.id) {
            list.rounded_rect(*r, 6.0, p.surface_muted);
        }
        let color = m
            .color
            .as_ref()
            .and_then(|color| u32::from_str_radix(color.trim_start_matches('#'), 16).ok())
            .unwrap_or(p.muted);
        list.icon_centered(
            Rect::new(r.left + 6.0, r.top, r.left + 22.0, r.bottom),
            if m.pinned {
                Icon::PIN
            } else {
                Icon::MESSAGE_SQUARE
            },
            14.0,
            color,
        );
        let timestamp = Local
            .timestamp_millis_opt(m.updated_at)
            .single()
            .map(|t| {
                if t.date_naive() == Local::now().date_naive() {
                    t.format("%H:%M").to_string()
                } else {
                    t.format("%m/%d").to_string()
                }
            })
            .unwrap_or_default();
        let project = m
            .project_id
            .as_ref()
            .and_then(|id| s.projects.iter().find(|p| &p.id == id))
            .map(|p| format!(" · {}", p.name))
            .unwrap_or_default();
        let meta = format!(
            "{timestamp}{}{project}",
            if m.message_count > 0 {
                format!(" · {} 条", m.message_count)
            } else {
                String::new()
            }
        );
        list.text(
            Rect::new(r.left + 30.0, r.top + 6.0, r.right - 40.0, r.top + 26.0),
            text::ellipsize(&m.title, TextStyle::Label, (r.width() - 70.0).max(0.0)),
            TextStyle::Label,
            p.foreground,
        );
        list.text(
            Rect::new(r.left + 30.0, r.top + 28.0, r.right - 40.0, r.bottom - 4.0),
            text::ellipsize(&meta, TextStyle::Tiny, (r.width() - 70.0).max(0.0)),
            TextStyle::Tiny,
            p.muted,
        );
        if !streaming && s.hover.as_deref() == Some(&m.id) {
            list.icon_centered(
                Rect::new(r.right - 34.0, r.top + 10.0, r.right - 6.0, r.top + 38.0),
                Icon::TRASH2,
                13.0,
                p.muted,
            );
        }
    }
    list.pop_clip();
    list.pop_clip();
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_filters_and_delete_wins() {
        let mut s = State::default();
        s.query.set_text("墨池");
        let sessions = vec![
            AiSessionMeta {
                id: "a".into(),
                title: "墨池讨论".into(),
                ..Default::default()
            },
            AiSessionMeta {
                id: "b".into(),
                title: "其他".into(),
                ..Default::default()
            },
        ];
        let lay = layout(&s, &sessions, Rect::new(0.0, 0.0, 260.0, 700.0));
        assert_eq!(lay.rows.len(), 1);
        assert_eq!(lay.hit(230.0, 106.0), Some(Hit::Delete("a".into())));
        let mut list = DrawList::new();
        paint(
            &mut list,
            Rect::new(0.0, 0.0, 260.0, 700.0),
            &mut s,
            &lay,
            None,
            false,
            false,
            super::super::theme::tokens().palette(false),
        );
        assert!(list.finish().is_ok());
    }
}
