//! 文件名标题的行内编辑，只调用文件重命名，不重写 Markdown。
use super::*;
pub(super) struct Editing {
    pub path: PathBuf,
    pub field: TextField,
}
impl App {
    pub(super) fn begin_title_edit(&mut self) {
        // 快速笔记在工作区中有固定路径，其正文仍可编辑。
        if self.state.view == WorkspaceView::QuickNote {
            return;
        }
        let Some(path) = self.active_file_path() else {
            return;
        };
        let mut field = TextField::new("标题");
        field.style = TextStyle::DocumentTitle;
        field.set_text(&path.file_stem().unwrap_or_default().to_string_lossy());
        field.select_all();
        self.title_editing = Some(Editing { path, field });
        self.focus = Focus::DocumentTitle;
        self.editor_engaged = false;
        self.editor_ai.invalidate();
    }
    pub(super) fn commit_title(&mut self) -> bool {
        let Some(edit) = self.title_editing.take() else {
            return true;
        };
        let value = edit.field.text().trim();
        if value.is_empty() {
            self.focus = Focus::Main;
            return true;
        }
        self.ai_cancel();
        let extension = edit
            .path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_else(|| ".mc".into());
        let name = if value.ends_with(&extension) {
            value.to_owned()
        } else {
            format!("{value}{extension}")
        };
        let renamed = sidebar::finalize_name(&name, false)
            .map_err(|e| e.to_string())
            .and_then(|name| {
                self.shell
                    .rename(&edit.path, &name)
                    .map_err(|e| e.to_string())
            });
        match renamed {
            Ok(path) => {
                if let Some(icon) = self.file_icons.remove(&edit.path) {
                    self.file_icons.insert(path.clone(), icon.clone());
                    self.settings
                        .remove(&format!("file.icon:{}", edit.path.display()));
                    self.settings
                        .set(&format!("file.icon:{}", path.display()), &icon);
                    let _ = self.settings.flush();
                }
                if self.split.other.as_deref() == Some(edit.path.as_path()) {
                    self.split.other = Some(path);
                }
                self.focus = Focus::Main;
                self.remember_split_active();
                self.invalidate_main();
                self.sync_state();
                true
            }
            Err(error) => {
                self.state.status_text = error;
                self.title_editing = Some(edit);
                false
            }
        }
    }
    pub(super) fn paint_title_edit(&mut self, area: Rect, p: &Palette) {
        if let Some(edit) = self.title_editing.as_mut() {
            let r = self.doc.title_rect(area, self.shell.active_scroll());
            self.list.push_clip(area);
            edit.field.paint(
                &mut self.list,
                r,
                self.focus == Focus::DocumentTitle,
                p,
                FieldLook::bare(),
            );
            self.list.pop_clip();
        }
    }
}
