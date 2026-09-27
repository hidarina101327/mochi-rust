//! 展示保存冲突，并提供冲突内容的复制操作。
use super::*;
impl App {
    pub(super) fn show_save_conflict(&mut self, path: PathBuf) {
        self.dialog = Some(Dialog {
            title: "文件保存冲突".into(),
            description: "磁盘文件已有变化，本地修改仍然保留。".into(),
            field: None,
            error: String::new(),
            note: Some("用磁盘：舍弃本地改动。\n用本地：覆盖磁盘；副本：保留两者。".into()),
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "用磁盘".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::ConflictReload(path.clone()),
                },
                DialogButton {
                    label: "用本地".into(),
                    kind: ButtonKind::Danger,
                    action: DialogAction::ConflictOverwrite(path.clone()),
                },
                DialogButton {
                    label: "存副本".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::ConflictCopy(path),
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }
    pub(super) fn conflict_copy(&mut self, path: &Path) {
        let Some(content) = self
            .shell
            .tabs()
            .iter()
            .find(|t| t.path() == Some(path))
            .and_then(|t| match &t.kind {
                TabKind::Viewer {
                    content: viewer::Content::Base(state),
                    ..
                } => mochi_core::base::serialize_base_document(&state.document).ok(),
                _ => t.buffer().map(|b| b.text().to_owned()),
            })
        else {
            return;
        };
        let name = format!(
            "{}-本地副本.{}",
            path.file_stem().unwrap_or_default().to_string_lossy(),
            path.extension().unwrap_or_default().to_string_lossy()
        );
        let Some(target) = platform::save_file(HWND(self.hwnd_raw as *mut _), &name) else {
            return;
        };
        if target
            .to_string_lossy()
            .replace('\\', "/")
            .eq_ignore_ascii_case(&path.to_string_lossy().replace('\\', "/"))
        {
            self.state.status_text = "副本需要使用不同的文件名".into();
            return;
        }
        match mochi_core::files::FileService::new().write_file_safe(&target, &content) {
            Ok(()) => {
                self.close_dialog();
                self.shell.forget_tabs_under(path);
                self.shell.open_file(path);
                self.shell.open_file(&target);
                self.remember_split_active();
                self.invalidate_main();
                self.sync_state();
                self.state.status_text = "本地副本已保存，原磁盘版本未被覆盖".into();
            }
            Err(error) => self.state.status_text = format!("副本保存失败：{error}"),
        }
    }
}
