//! 大纲下方的链接区，对照 BacklinksPanel.tsx；几何与命中共用同一份布局。
use super::document::Heading;
use super::draw::{DrawList, TextStyle};
use super::icons::Icon;
use super::layout::Rect;
use super::theme::Palette;
use super::{outline, text, theme};
use mochi_core::metadata_index::{NoteLink, UnlinkedMention};
use std::path::PathBuf;

#[derive(Default)]
pub struct Data {
    pub backlinks: Vec<NoteLink>,
    pub outgoing: Vec<NoteLink>,
    pub mentions: Vec<UnlinkedMention>,
}

pub struct State {
    pub source: Option<PathBuf>,
    pub data: Data,
    pub collapsed: [bool; 3],
    pub scroll: f32,
    pub loading: bool,
    pub error: String,
    pub hover: Option<Rect>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            source: None,
            data: Data::default(),
            collapsed: [false, true, true],
            scroll: 0.0,
            loading: false,
            error: String::new(),
            hover: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Heading(usize),
    Refresh,
    Toggle(usize),
    Open(PathBuf),
}

#[derive(Default)]
pub struct Layout {
    pub entries: Vec<(Rect, Hit)>,
    pub list: Rect,
    pub outline: Rect,
    pub content_height: f32,
    rows: Vec<Row>,
    divider_y: f32,
}

struct Row {
    rect: Rect,
    label: String,
    context: Option<String>,
    icon: Option<Icon>,
    missing: bool,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        if !self.list.contains(x, y) {
            return None;
        }
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| h.clone())
    }
    #[cfg(test)]
    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.list.height()).max(0.0)
    }
    pub fn hover_at(&self, x: f32, y: f32) -> Option<Rect> {
        if !self.list.contains(x, y) {
            return None;
        }
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(r, _)| *r)
    }
}

/// px/py/gap/mt/pt 来自 Electron 的 Tailwind 类：mt-10 / pt-4 / px-2 / py-1.5。
pub fn layout(s: &State, headings: &[Heading], area: Rect) -> Layout {
    let mut lay = Layout {
        list: area,
        ..Layout::default()
    };
    if area.is_empty() {
        return lay;
    }
    let top = area.top - s.scroll;
    let outline_height = 32.0
        + if headings.is_empty() {
            8.0 + theme::ROW_HEIGHT
        } else {
            headings.len() as f32 * theme::ROW_HEIGHT
        };
    lay.outline = Rect::new(area.left, top, area.right, top + outline_height);
    for i in 0..headings.len() {
        let y = top + 32.0 + i as f32 * theme::ROW_HEIGHT;
        lay.entries.push((
            Rect::new(area.left, y, area.right, y + theme::ROW_HEIGHT),
            Hit::Heading(i),
        ));
    }
    let mut y = top + outline_height;
    if s.source.is_some() {
        y += 40.0;
        lay.divider_y = y;
        y += 17.0;
        let left = area.left + 12.0;
        let right = area.right - 12.0;
        let header = Rect::new(left, y, right, y + 20.0);
        lay.rows.push(Row {
            rect: header,
            label: "链接".into(),
            context: None,
            icon: Some(Icon::LINK2),
            missing: false,
        });
        lay.entries
            .push((Rect::new(right - 20.0, y, right, y + 20.0), Hit::Refresh));
        y += 24.0;
        if !s.error.is_empty() || s.loading {
            lay.rows.push(Row {
                rect: Rect::new(left + 4.0, y, right, y + 32.0),
                label: if s.loading {
                    "正在加载链接…".into()
                } else {
                    "链接加载失败，点击刷新重试".into()
                },
                context: None,
                icon: None,
                missing: false,
            });
            y += 32.0;
        } else if s.data.backlinks.is_empty()
            && s.data.outgoing.is_empty()
            && s.data.mentions.is_empty()
        {
            let title = s
                .source
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            // 三行空态；长文件名在最后一行截断，不溢出侧栏。
            for label in [
                "还没有笔记链接到这里。".to_owned(),
                "在其他笔记中输入".into(),
                format!("[[{title}]] 即可建立关联。"),
            ] {
                lay.rows.push(Row {
                    rect: Rect::new(left + 4.0, y, right, y + 20.0),
                    label,
                    context: None,
                    icon: None,
                    missing: false,
                });
                y += 20.0;
            }
            y += 8.0;
        }
        for (section, label, count) in [
            (0, "反向链接", s.data.backlinks.len()),
            (1, "出链", s.data.outgoing.len()),
            (2, "未链接的提及", s.data.mentions.len()),
        ] {
            if count == 0 {
                continue;
            }
            let rect = Rect::new(left, y, right, y + 28.0);
            lay.entries.push((rect, Hit::Toggle(section)));
            lay.rows.push(Row {
                rect,
                label: format!("{label}  {count}"),
                context: None,
                icon: Some(if s.collapsed[section] {
                    Icon::CHEVRON_RIGHT
                } else {
                    Icon::CHEVRON_DOWN
                }),
                missing: false,
            });
            y += 28.0;
            if s.collapsed[section] {
                continue;
            }
            for i in 0..count {
                let (label, context, path) = match section {
                    0 => {
                        let l = &s.data.backlinks[i];
                        (
                            l.source_title.clone(),
                            Some(l.context.clone()),
                            Some(l.source_path.clone()),
                        )
                    }
                    1 => {
                        let l = &s.data.outgoing[i];
                        (l.target_raw.clone(), None, l.target_path.clone())
                    }
                    _ => {
                        let l = &s.data.mentions[i];
                        (
                            l.source_title.clone(),
                            Some(l.context.clone()),
                            Some(l.source_path.clone()),
                        )
                    }
                };
                let rect = Rect::new(
                    left + 8.0,
                    y,
                    right - 8.0,
                    y + if context.is_some() { 44.0 } else { 28.0 },
                );
                let missing = path.is_none();
                if let Some(path) = path {
                    lay.entries.push((rect, Hit::Open(path)));
                }
                lay.rows.push(Row {
                    rect,
                    label,
                    context,
                    icon: (section == 1).then_some(Icon("CornerDownRight")),
                    missing,
                });
                y = rect.bottom + 2.0;
            }
            y += 8.0;
        }
    }
    lay.content_height = y - top;
    lay
}

/// 编辑器正文后的链接区（Electron 的 EditorContent 后继节点）。
pub fn layout_tail(s: &State, area: Rect, clip: Rect) -> Layout {
    let mut lay = layout(s, &[], area);
    let offset = lay.outline.height();
    let move_up = |r: &mut Rect| {
        r.top -= offset;
        r.bottom -= offset;
    };
    for (r, _) in &mut lay.entries {
        move_up(r);
    }
    for row in &mut lay.rows {
        move_up(&mut row.rect);
    }
    lay.divider_y -= offset;
    lay.content_height -= offset;
    lay.outline = Rect::ZERO;
    lay.list = clip;
    lay
}

pub fn paint(
    list: &mut DrawList,
    s: &State,
    headings: &[Heading],
    active: Option<usize>,
    lay: &Layout,
    p: &Palette,
) {
    if lay.list.is_empty() {
        return;
    }
    list.push_clip(lay.list);
    outline::paint(list, lay.outline, headings, active, 0, p);
    if let Some(hover) = s.hover {
        if lay
            .entries
            .iter()
            .any(|(r, h)| *r == hover && matches!(h, Hit::Open(_)))
        {
            list.rect(hover, theme::mix(p.accent, p.area_assistant_default, 0.10));
        }
    }
    if s.source.is_some() {
        list.rect(
            Rect::new(
                lay.list.left + 12.0,
                lay.divider_y,
                lay.list.right - 12.0,
                lay.divider_y + 1.0,
            ),
            p.border,
        );
    }
    for row in &lay.rows {
        if row.rect.bottom <= lay.list.top || row.rect.top >= lay.list.bottom {
            continue;
        }
        let mut x = row.rect.left;
        if let Some(icon) = row.icon {
            list.icon_centered(
                Rect::new(
                    x,
                    row.rect.top,
                    x + 13.0,
                    row.rect.top + 28.0_f32.min(row.rect.height()),
                ),
                icon,
                13.0,
                p.muted,
            );
            x += 19.0;
        }
        let right = row.rect.right - if row.missing { 62.0 } else { 0.0 };
        let height = if row.context.is_some() {
            22.0
        } else {
            row.rect.height()
        };
        let muted = row.missing
            || row.context.is_none()
                && row.icon != Some(Icon("CornerDownRight"))
                && row.label != "链接"
                && s.hover != Some(row.rect);
        list.text(
            Rect::new(x, row.rect.top, right, row.rect.top + height),
            text::ellipsize(&row.label, TextStyle::Caption, (right - x).max(0.0)),
            TextStyle::Caption,
            if muted { p.muted } else { p.foreground },
        );
        if row.missing {
            list.icon_centered(
                Rect::new(right, row.rect.top, right + 14.0, row.rect.bottom),
                Icon("Link2Off"),
                11.0,
                p.muted,
            );
            list.text(
                Rect::new(right + 16.0, row.rect.top, row.rect.right, row.rect.bottom),
                "未创建",
                TextStyle::Tiny,
                p.muted,
            );
        }
        if let Some(context) = &row.context {
            list.text(
                Rect::new(x, row.rect.top + 22.0, row.rect.right, row.rect.bottom),
                text::ellipsize(context, TextStyle::Tiny, (row.rect.right - x).max(0.0)),
                TextStyle::Tiny,
                p.muted,
            );
        }
    }
    for (rect, hit) in &lay.entries {
        if matches!(hit, Hit::Refresh) {
            list.icon_centered(*rect, Icon("RefreshCw"), 12.0, p.muted);
        }
    }
    list.pop_clip();
}

#[cfg(test)]
mod tests {
    use super::super::draw::DrawCmd;
    use super::*;
    fn area() -> Rect {
        Rect::new(800.0, 80.0, 1100.0, 700.0)
    }
    fn state() -> State {
        State {
            source: Some("目标.md".into()),
            ..State::default()
        }
    }
    #[test]
    fn empty_state_and_clipping_are_visible_in_both_themes() {
        let s = state();
        let lay = layout(&s, &[], area());
        for dark in [false, true] {
            let mut list = DrawList::new();
            paint(
                &mut list,
                &s,
                &[],
                None,
                &lay,
                theme::tokens().palette(dark),
            );
            assert!(list
                .cmds()
                .iter()
                .any(|c| matches!(c, DrawCmd::Text { text, .. } if text.contains("[[目标]]"))));
            assert!(list.finish().is_ok());
        }
    }
    #[test]
    fn collapsed_mentions_reveal_clickable_source_and_stay_inside_viewport() {
        let mut s = state();
        s.data.mentions.push(UnlinkedMention {
            source_path: "来源.md".into(),
            source_title: "来源".into(),
            line: 1,
            context: "正文".into(),
        });
        assert!(!layout(&s, &[], area())
            .entries
            .iter()
            .any(|(_, h)| matches!(h, Hit::Open(_))));
        s.collapsed[2] = false;
        let lay = layout(&s, &[], area());
        let (r, h) = lay
            .entries
            .iter()
            .find(|(_, h)| matches!(h, Hit::Open(_)))
            .unwrap();
        assert_eq!(lay.hit(r.left + 1.0, r.top + 1.0), Some(h.clone()));
        assert!(lay.hit(area().left - 1.0, r.top).is_none());
    }
    #[test]
    fn missing_outgoing_target_is_labelled_but_not_clickable() {
        let mut s = state();
        s.collapsed[1] = false;
        s.data.outgoing.push(NoteLink {
            source_path: "目标.md".into(),
            source_title: "目标".into(),
            target_raw: "未建立".into(),
            target_path: None,
            kind: "wiki".into(),
            heading: None,
            block_id: None,
            alias: None,
            line: 1,
            context: String::new(),
        });
        let lay = layout(&s, &[], area());
        assert!(!lay.entries.iter().any(|(_, h)| matches!(h, Hit::Open(_))));
        let mut list = DrawList::new();
        paint(
            &mut list,
            &s,
            &[],
            None,
            &lay,
            theme::tokens().palette(false),
        );
        assert!(list
            .cmds()
            .iter()
            .any(|c| matches!(c, DrawCmd::Text { text, .. } if text == "未创建")));
        assert!(list.finish().is_ok());
    }
    #[test]
    fn long_outline_and_links_share_scroll_and_reject_clipped_hits() {
        let mut s = state();
        let headings = (0..50)
            .map(|i| Heading {
                level: 1,
                text: i.to_string(),
                y: i as f32 * 100.0,
            })
            .collect::<Vec<_>>();
        let first = layout(&s, &headings, area());
        s.scroll = first.max_scroll();
        let lay = layout(&s, &headings, area());
        let (r, _) = lay
            .entries
            .iter()
            .find(|(_, h)| matches!(h, Hit::Refresh))
            .unwrap();
        assert_eq!(lay.hit(r.left + 1.0, r.top + 1.0), Some(Hit::Refresh));
        assert!(lay.hit(area().left + 2.0, area().top - 1.0).is_none());
    }
    #[test]
    fn heading_hit_follows_scrolled_geometry_and_excludes_header() {
        let mut s = state();
        let heads = (0..30)
            .map(|i| Heading {
                level: 1,
                text: format!("标题{i}"),
                y: i as f32 * 100.0,
            })
            .collect::<Vec<_>>();
        let lay = layout(&s, &heads, area());
        assert_eq!(lay.hit(810.0, area().top + 5.0), None);
        assert_eq!(lay.hit(810.0, area().top + 33.0), Some(Hit::Heading(0)));
        s.scroll = theme::ROW_HEIGHT * 3.0;
        let lay = layout(&s, &heads, area());
        assert_eq!(lay.hit(810.0, area().top + 33.0), Some(Hit::Heading(3)));
    }
}
