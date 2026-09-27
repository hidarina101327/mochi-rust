//! 行内标记和页边气泡共用现有的评论数据文件与面板。
use super::*;

#[derive(Default)]
pub(super) struct Cache {
    revision: Option<u64>,
    entries: std::collections::HashMap<String, CachedAnchor>,
    index: crate::ui::comment_anchors::IndexCache,
    #[cfg(test)]
    resolutions: usize,
}

struct CachedAnchor {
    anchor: sidecars::CommentAnchor,
    range: Option<std::ops::Range<usize>>,
}

fn preview_range(
    range: std::ops::Range<usize>,
    change: Option<(usize, usize, usize)>,
) -> std::ops::Range<usize> {
    let Some((start, old_end, new_end)) = change else {
        return range;
    };
    if range.end <= start {
        return range;
    }
    let shift = |offset: usize| new_end + offset.saturating_sub(old_end);
    let from = if range.start >= old_end {
        shift(range.start)
    } else {
        range.start.min(start)
    };
    let end = if range.end >= old_end {
        shift(range.end)
    } else {
        new_end
    };
    from..end.max(from)
}

impl Cache {
    pub(super) fn range_for(&self, id: &str) -> Option<&std::ops::Range<usize>> {
        self.entries.get(id)?.range.as_ref()
    }

    fn refresh(
        &mut self,
        buffer: &crate::ui::editor::TextBuffer,
        parsed: &crate::ui::document::Parsed,
        comments: &[sidecars::DocumentComment],
    ) {
        // 选区、滚动、渐进布局、悬停都不会改变内容修订号。
        // 对这些操作绝不做全文锚点搜索。
        let anchored = || {
            comments.iter().filter_map(|comment| {
                if comment.parent_id.is_some() {
                    return None;
                }
                comment
                    .anchor
                    .as_ref()
                    .map(|anchor| (comment.id.as_str(), anchor))
            })
        };
        let same_revision = self.revision == Some(buffer.revision());
        if same_revision
            && self.entries.len() == anchored().count()
            && anchored().all(|(id, anchor)| {
                self.entries
                    .get(id)
                    .is_some_and(|entry| entry.anchor == *anchor)
            })
        {
            return;
        }
        // 渲染解析结果里包含未提交的输入法预览。锚点仍指向已提交的缓冲区，
        // 与加这个缓存之前一致。
        let committed;
        let parsed = if buffer.composition().is_some() {
            committed = crate::ui::document::parse_ranged(buffer.text());
            &committed
        } else {
            parsed
        };
        let mut resolver = crate::ui::comment_anchors::Resolver::with_cache(
            buffer.text(),
            parsed,
            &mut self.index,
        );
        let mut entries = std::collections::HashMap::new();
        for (id, anchor) in anchored() {
            if same_revision {
                if let Some(previous) = self
                    .entries
                    .remove(id)
                    .filter(|entry| entry.anchor == *anchor)
                {
                    entries.insert(id.to_owned(), previous);
                    continue;
                }
            }
            let range = resolver.resolve(anchor);
            #[cfg(test)]
            {
                self.resolutions += 1;
            }
            entries.insert(
                id.to_owned(),
                CachedAnchor {
                    anchor: anchor.clone(),
                    range,
                },
            );
        }
        self.entries = entries;
        self.revision = Some(buffer.revision());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{comment_anchors, document, editor::TextBuffer};

    fn comment(source: &str, selected: &str) -> sidecars::DocumentComment {
        let start = source.find(selected).unwrap();
        sidecars::DocumentComment {
            resolved: false,
            id: "anchor-1".into(),
            parent_id: None,
            target_type: "text".into(),
            author: "test".into(),
            content: "批注".into(),
            created_at: String::new(),
            updated_at: None,
            attachments: Vec::new(),
            anchor: comment_anchors::create(source, start..start + selected.len(), "text"),
        }
    }

    #[test]
    fn comment_ranges_are_cached_across_frames_and_cursor_moves() {
        let source = "# 标题\n\n前文😀 **狂热** 后文\n\n另一个段落";
        let mut buffer = TextBuffer::new(source);
        let comments = vec![comment(source, "狂热")];
        let parsed = document::parse_ranged(source);
        let mut cache = Cache::default();
        for step in 0..100 {
            buffer.set_cursor(if step % 2 == 0 { 0 } else { source.len() }, false);
            cache.refresh(&buffer, &parsed, &comments);
        }
        assert_eq!(
            cache.resolutions, 1,
            "repaint must not resolve anchors again"
        );
        let range = cache.entries["anchor-1"].range.clone().unwrap();
        assert_eq!(&source[range], "狂热");
    }

    #[test]
    fn comment_cache_tracks_edits_undo_and_sidecar_anchor_changes() {
        let source = "# 标题\n\n前文😀 **狂热** 后文\n\n另一个段落";
        let mut buffer = TextBuffer::new(source);
        let mut comments = vec![comment(source, "狂热")];
        let mut cache = Cache::default();
        let update =
            |cache: &mut Cache, buffer: &TextBuffer, comments: &[sidecars::DocumentComment]| {
                cache.refresh(buffer, &document::parse_ranged(buffer.text()), comments);
                cache.entries["anchor-1"].range.clone().unwrap()
            };
        let original = update(&mut cache, &buffer, &comments);
        buffer.set_cursor(0, false);
        buffer.insert("移动后的前文\n\n");
        let moved = update(&mut cache, &buffer, &comments);
        assert_eq!(&buffer.text()[moved.clone()], "狂热");
        assert!(moved.start > original.start);
        buffer.undo();
        assert_eq!(update(&mut cache, &buffer, &comments), original);
        buffer.replace_range(original.clone(), "平静");
        let replaced = update(&mut cache, &buffer, &comments);
        assert_eq!(
            &buffer.text()[replaced],
            "平静",
            "equal-length edits need a new revision"
        );
        assert_eq!(cache.resolutions, 4);
        comments[0].content = "只改评论正文".into();
        update(&mut cache, &buffer, &comments);
        assert_eq!(cache.resolutions, 4);
        comments[0] = comment(buffer.text(), "另一个段落");
        let changed = update(&mut cache, &buffer, &comments);
        assert_eq!(&buffer.text()[changed], "另一个段落");
        assert_eq!(cache.resolutions, 5);
        cache.refresh(&buffer, &document::parse_ranged(buffer.text()), &[]);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn ime_preview_moves_comment_geometry_without_re_resolving_committed_anchors() {
        let source = "前文😀 **狂热** 后文";
        let mut buffer = TextBuffer::new(source);
        let comments = vec![comment(source, "狂热")];
        let mut cache = Cache::default();
        cache.refresh(&buffer, &document::parse_ranged(source), &comments);
        let original = cache.range_for("anchor-1").unwrap().clone();
        buffer.set_cursor(0, false);
        buffer.set_composition("组合😀", 0);
        let shown = buffer.display_text().0;
        cache.refresh(&buffer, &document::parse_ranged(&shown), &comments);
        let mapped = preview_range(
            original.clone(),
            Some(crate::ui::editor::changed_bounds(source, &shown)),
        );
        assert_eq!(&shown[mapped], "狂热");
        assert_eq!(cache.resolutions, 1);
        drop(shown);
        buffer.cancel_composition();
        cache.refresh(&buffer, &document::parse_ranged(source), &comments);
        assert_eq!(cache.range_for("anchor-1"), Some(&original));
        assert_eq!(cache.resolutions, 1);
    }
}

impl App {
    pub(super) fn load_editor_comments(&mut self) {
        let path = self.active_file_path();
        let state = &mut self.panels.comments;
        if state.source != path {
            state.pending = None;
            state.scroll = 0.0;
        }
        state.source = path.clone();
        state.comments = path
            .as_deref()
            .map(|p| sidecars::load_comments(&p.to_string_lossy()).comments)
            .unwrap_or_default();
    }
    pub(super) fn begin_selection_comment(&mut self) {
        let Some(buffer) = self.shell.active().and_then(|t| t.buffer()) else {
            return;
        };
        let (a, b) = buffer.selection();
        let anchor = crate::ui::comment_anchors::create(buffer.text(), a..b, "text");
        let Some(anchor) = anchor else { return };
        self.state.ai_panel_open = true;
        self.invalidate_main();
        self.set_right_panel(RightPanel::Comments);
        self.panels
            .comments
            .start_compose(None, anchor.selected_text.clone());
        self.panels.comments.pending.as_mut().unwrap().anchor = Some(anchor);
        self.focus = Focus::CommentCompose;
    }
    pub(super) fn paint_editor_comments(&mut self, palette: &Palette) {
        self.comment_bubbles.clear();
        if self.content() != MainContent::Document {
            return;
        }
        let Some(buffer) = self.shell.active().and_then(|t| t.buffer()) else {
            return;
        };
        if self.panels.comments.comments.is_empty() {
            self.comment_ranges = Default::default();
            return;
        }
        self.comment_ranges
            .refresh(buffer, self.doc.parsed(), &self.panels.comments.comments);
        // 先解析已提交的锚点，再把它们的几何经输入法预览做变换——每帧一次，
        // 而不是每条评论一次。
        let preview_change = buffer
            .composition()
            .map(|_| crate::ui::editor::changed_bounds(buffer.text(), self.doc.display_source()));
        self.list.push_clip(self.editor_area);
        let mut last_y = f32::NEG_INFINITY;
        for comment in self
            .panels
            .comments
            .comments
            .iter()
            .filter(|c| c.parent_id.is_none())
        {
            let Some(range) = self
                .comment_ranges
                .entries
                .get(&comment.id)
                .and_then(|entry| entry.range.as_ref())
            else {
                continue;
            };
            let rects = self.doc.mark_rects(
                self.editor_area,
                preview_range(range.clone(), preview_change),
                self.shell.active_scroll(),
                palette,
            );
            for rect in &rects {
                self.list.rect_alpha(*rect, 0xf2c94c, 0.25);
            }
            let Some(last) = rects.last() else { continue };
            let y = last.top.max(last_y + 26.0);
            let bubble = Rect::from_size(self.editor_area.right - 28.0, y, 24.0, 24.0);
            self.list.rounded_rect(bubble, 6.0, palette.surface_muted);
            self.list
                .icon_centered(bubble, Icon::MESSAGE_SQUARE, 14.0, palette.accent);
            self.comment_bubbles.push((bubble, comment.id.clone()));
            last_y = y;
        }
        self.list.pop_clip();
    }
    pub(super) fn click_comment_bubble(&mut self, x: f32, y: f32) -> bool {
        let Some(id) = self
            .comment_bubbles
            .iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, id)| id.clone())
        else {
            return false;
        };
        self.state.ai_panel_open = true;
        self.invalidate_main();
        self.set_right_panel(RightPanel::Comments);
        self.panels.comments.scroll_to(&id);
        true
    }
}
