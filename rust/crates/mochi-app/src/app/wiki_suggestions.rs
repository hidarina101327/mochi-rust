//! 原生版 WikiLinkMenu：输入 [[，选择建议项，然后继续编辑。
use super::*;

pub(super) struct Suggestion {
    path: PathBuf,
    range: std::ops::Range<usize>,
    query: String,
    pub menu: Menu<String>,
}

fn query(source: &str, cursor: usize) -> Option<(std::ops::Range<usize>, String)> {
    let before = source.get(..cursor)?;
    let start = before.rfind("[[")?;
    if start > 0 && source.as_bytes()[start - 1] == b'\\' {
        return None;
    }
    let value = &before[start + 2..];
    if value.contains(['\r', '\n', ']', '[', '|', '#']) || value.chars().count() > 200 {
        return None;
    }
    let parsed = crate::ui::document::parse_ranged(source);
    if parsed.block_at(cursor).is_some_and(|i| {
        matches!(
            parsed.blocks[i].block,
            crate::ui::document::Block::Code { .. } | crate::ui::document::Block::Math(_)
        )
    }) {
        return None;
    }
    Some((start..cursor, value.to_owned()))
}

impl App {
    pub(super) fn refresh_wiki_suggestion(&mut self) {
        if self.content() != MainContent::Document
            || self.focus != Focus::Main
            || !self.editor_engaged
            || self.dialog.is_some()
            || self.menu.is_some()
            || self.table_picker.is_some()
        {
            self.wiki_suggestion = None;
            return;
        }
        let Some(tab) = self.shell.active() else {
            return;
        };
        let Some(buffer) = tab.buffer() else { return };
        if buffer.has_selection() {
            self.wiki_suggestion = None;
            return;
        }
        let Some((range, query)) = query(buffer.text(), buffer.cursor()) else {
            self.wiki_suggestion = None;
            self.wiki_dismissed = None;
            return;
        };
        let Some(path) = self.active_file_path() else {
            return;
        };
        if self.wiki_dismissed.as_ref() == Some(&(path.clone(), range.end, query.clone())) {
            self.wiki_suggestion = None;
            return;
        }
        if self
            .wiki_suggestion
            .as_ref()
            .is_some_and(|s| s.path == path && s.range == range && s.query == query)
        {
            return;
        }
        let Some(workspace) = self.shell.workspace() else {
            return;
        };
        let notes = workspace.index.search_notes_by_title(&query, 12);
        let mut items: Vec<_> = notes
            .iter()
            .map(|n| MenuItem::new(&n.title, n.title.clone()).icon(Icon::FILE_TEXT))
            .collect();
        let trimmed = query.trim();
        if !trimmed.is_empty()
            && !notes
                .iter()
                .any(|n| n.title.to_lowercase() == trimmed.to_lowercase())
        {
            items.push(
                MenuItem::new(format!("新建「{trimmed}」"), trimmed.to_owned()).icon(Icon::PLUS),
            );
        }
        if items.is_empty() {
            self.wiki_suggestion = None;
            return;
        }
        let caret = self
            .doc
            .caret_rect(self.editor_area, buffer, tab.scroll)
            .unwrap_or(self.editor_area);
        let mut menu = Menu::open_at(
            items,
            caret.left,
            caret.bottom + 4.0,
            self.renderer.viewport(),
        );
        menu.hover = Some(0);
        self.wiki_suggestion = Some(Suggestion {
            path,
            range,
            query,
            menu,
        });
    }

    fn apply_wiki_suggestion(&mut self, index: usize) {
        let Some(s) = self.wiki_suggestion.take() else {
            return;
        };
        if self.active_file_path().as_ref() != Some(&s.path) {
            return;
        }
        let Some(item) = s.menu.items.get(index) else {
            return;
        };
        let Some(buffer) = self.shell.active_buffer_mut() else {
            return;
        };
        if buffer.text().get(s.range.clone()) != Some(format!("[[{}", s.query).as_str()) {
            return;
        }
        buffer.replace_range(s.range, &format!("[[{}]]", item.action));
        self.after_doc_edit(true);
    }

    pub(super) fn wiki_key(&mut self, key: u16) -> bool {
        let Some(s) = self.wiki_suggestion.as_mut() else {
            return false;
        };
        match key {
            0x1b => {
                self.wiki_dismissed = Some((s.path.clone(), s.range.end, s.query.clone()));
                self.wiki_suggestion = None;
            }
            0x26 | 0x28 => {
                s.menu.move_selection(if key == 0x26 { -1 } else { 1 });
            }
            0x09 | 0x0d => {
                let index = s.menu.hover.unwrap_or(0);
                self.apply_wiki_suggestion(index);
            }
            _ => return false,
        }
        true
    }

    pub(super) fn wiki_click(&mut self, x: f32, y: f32) -> bool {
        if self
            .wiki_suggestion
            .as_mut()
            .is_some_and(|s| s.menu.begin_scrollbar_drag(x, y))
        {
            return true;
        }
        let Some(s) = self.wiki_suggestion.as_ref() else {
            return false;
        };
        if let Some(index) = s.menu.hit(x, y) {
            self.apply_wiki_suggestion(index);
            return true;
        }
        self.wiki_dismissed = Some((s.path.clone(), s.range.end, s.query.clone()));
        self.wiki_suggestion = None;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::query;
    #[test]
    fn suggestion_trigger_respects_markdown_and_unicode_boundaries() {
        for source in ["[[中文", "链接 [[", "![[示例"] {
            assert!(query(source, source.len()).is_some());
        }
        for source in [
            "[[closed]]",
            "\\[[escaped",
            "[[two\nlines",
            "```md\n[[code",
            "[[note#heading",
            "[[note|alias",
        ] {
            assert!(query(source, source.len()).is_none(), "{source}");
        }
    }
}
