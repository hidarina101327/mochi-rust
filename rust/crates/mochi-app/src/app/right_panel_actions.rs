//! 处理右侧栏中的版本、批注和番茄钟操作。
use super::*;

impl App {
    pub(super) fn on_version_click(&mut self, x: f32, y: f32) {
        let Some(hit) = self.panels.version_layout.hit(x, y) else {
            if self.focus == Focus::VersionMessage {
                self.focus = Focus::Main;
            }
            return;
        };
        match hit {
            version::Hit::Close => {
                self.state.ai_panel_open = false;
                self.invalidate_main();
            }
            version::Hit::Message => {
                self.focus = Focus::VersionMessage;
                if let Some(r) = self.panels.version_layout.rect_of(version::Hit::Message) {
                    self.panels
                        .version
                        .message
                        .click(x - (r.left + 12.0), false);
                }
            }
            version::Hit::Refresh => {
                let src = self.panels.version.source.clone();
                self.load_version_history(src);
            }
            version::Hit::Record => {
                let Some(git) = self.shell.git() else { return };
                if !self.panels.version.enabled {
                    return;
                }
                // 先把当前缓冲区落盘，再登记、提交（TSX：recordFileChange + commit(msg, 'manual')）
                if !self.save_active() {
                    self.panels.version.error = self.shell.status().into();
                    return;
                }
                let Some(src) = self.active_file_path() else {
                    return;
                };
                git.mark_write(src.clone());
                let msg = self.panels.version.message.text().trim().to_owned();
                let msg = if msg.is_empty() {
                    "手动记录版本".to_owned()
                } else {
                    msg
                };
                match git.commit(&msg, "manual") {
                    Ok(Some(_)) => self.panels.version.message.clear(),
                    Ok(None) => self.panels.version.error = "当前文档没有可记录的变更".to_owned(),
                    Err(e) => self.panels.version.error = e.to_string(),
                }
                let err = std::mem::take(&mut self.panels.version.error);
                self.load_version_history(Some(src));
                self.panels.version.error = err;
            }
            version::Hit::Open(i) => {
                // 把该版本的内容写到临时文件并开成标签（TSX 的 openVersion）
                let Some(git) = self.shell.git() else { return };
                let Some(src) = self.panels.version.source.clone() else {
                    return;
                };
                let versions = self.panels.version.versions();
                let Some(c) = versions.get(i).cloned().cloned() else {
                    return;
                };
                let content = match git.file_at_commit(&c.oid, &src.to_string_lossy()) {
                    Ok(t) => t,
                    Err(e) => {
                        self.panels.version.error = e.to_string();
                        return;
                    }
                };
                let dir = std::env::temp_dir().join("mochi-version-previews");
                let _ = std::fs::create_dir_all(&dir);
                let stem = src
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "version".into());
                let ext = src
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_else(|| ".mc".into());
                let safe: String = stem
                    .chars()
                    .map(|ch| {
                        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
                            || (ch as u32) < 0x20
                        {
                            '_'
                        } else {
                            ch
                        }
                    })
                    .take(80)
                    .collect();
                let preview = dir.join(format!("{safe}-v{}-{}{ext}", i + 1, c.short_oid));
                if std::fs::write(&preview, content).is_ok() && self.open_file_from_ui(&preview) {
                    if let Some(tab) = self.shell.active_mut() {
                        tab.title = format!(
                            "{}-第{}版",
                            src.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            i + 1
                        );
                    }
                    self.invalidate_main();
                    self.sync_state();
                }
            }
            version::Hit::Rollback(i) => {
                let Some(git) = self.shell.git() else { return };
                let Some(src) = self.panels.version.source.clone() else {
                    return;
                };
                let Some(oid) = self.panels.version.versions().get(i).map(|c| c.oid.clone()) else {
                    return;
                };
                self.dialog = Some(Dialog {
                    title: "回溯版本".into(),
                    description: "确定回溯到该版本吗？系统会先创建恢复前快照。".into(),
                    field: None,
                    error: String::new(),
                    note: None,
                    buttons: vec![
                        DialogButton {
                            label: "取消".into(),
                            kind: ButtonKind::Ghost,
                            action: DialogAction::Dismiss,
                        },
                        DialogButton {
                            label: "回溯".into(),
                            kind: ButtonKind::Primary,
                            action: DialogAction::RestoreVersion { path: src, oid },
                        },
                    ],
                    dismiss: DialogAction::Dismiss,
                    hover: None,
                });
                self.focus = Focus::Dialog;
                let _ = git;
            }
            version::Hit::Delete(i) => {
                let Some(oid) = self.panels.version.versions().get(i).map(|c| c.oid.clone()) else {
                    return;
                };
                self.panels.version.deleted.insert(oid);
                if let Some(src) = &self.panels.version.source {
                    let list: Vec<&String> = self.panels.version.deleted.iter().collect();
                    self.settings.set(
                        &deleted_versions_key(src),
                        &serde_json::to_string(&list).unwrap_or_default(),
                    );
                }
            }
        }
    }

    pub(super) fn on_comments_click(&mut self, x: f32, y: f32) {
        let Some(hit) = self.panels.comments_layout.hit(x, y) else {
            if self.focus == Focus::CommentCompose {
                self.focus = Focus::Main;
            }
            return;
        };
        match hit {
            comments::Hit::Compose => {
                self.panels.comments.start_compose(None, String::new());
                self.focus = Focus::CommentCompose;
            }
            comments::Hit::Reply(i) => {
                let Some((idx, _, _)) = self.panels.comments_layout.rows.get(i) else {
                    return;
                };
                let parent = self.panels.comments.comments[*idx].id.clone();
                let quote = self.panels.comments.comments[*idx]
                    .anchor
                    .as_ref()
                    .map(|a| a.selected_text.clone())
                    .unwrap_or_default();
                self.panels.comments.start_compose(Some(parent), quote);
                self.focus = Focus::CommentCompose;
            }
            comments::Hit::Delete(i) => {
                let Some((idx, _, _)) = self.panels.comments_layout.rows.get(i) else {
                    return;
                };
                let id = self.panels.comments.comments[*idx].id.clone();
                self.panels.comments.delete_thread(&id);
                self.save_comments();
            }
            comments::Hit::ComposeField => {
                self.focus = Focus::CommentCompose;
                if let (Some(p), Some(r)) = (
                    self.panels.comments.pending.as_mut(),
                    self.panels
                        .comments_layout
                        .rect_of(comments::Hit::ComposeField),
                ) {
                    p.field.multiline_click(r, x, y);
                }
            }
            comments::Hit::Cancel => {
                self.panels.comments.pending = None;
                self.focus = Focus::Main;
            }
            comments::Hit::Send => self.submit_comment(),
        }
    }

    pub(super) fn submit_comment(&mut self) {
        let Some(pending) = self.panels.comments.pending.take() else {
            return;
        };
        let content = pending.field.text().trim().to_owned();
        if content.is_empty() {
            self.panels.comments.pending = Some(pending);
            return;
        }
        let parent_id = pending.parent_id.clone();
        let anchor = pending.anchor.or_else(|| {
            parent_id.is_none().then(|| sidecars::CommentAnchor {
                kind: "document".into(),
                selected_text: String::new(),
                block_text: String::new(),
                block_type: "doc".into(),
                block_index: 0,
                start_offset: 0,
                end_offset: 0,
                prefix: String::new(),
                suffix: String::new(),
            })
        });
        let comment = sidecars::DocumentComment {
            resolved: false,
            id: uuid_v4(),
            target_type: if parent_id.is_some() {
                "comment".into()
            } else if anchor.as_ref().is_some_and(|a| a.kind != "document") {
                "text".into()
            } else {
                "document".into()
            },
            parent_id,
            author: self.panels.comments.author.clone(),
            content,
            created_at: mochi_core::jstime::now(),
            updated_at: None,
            anchor,
            attachments: Vec::new(),
        };
        self.panels.comments.comments.push(comment);
        self.save_comments();
        self.focus = Focus::Main;
    }

    pub(super) fn save_comments(&mut self) {
        let Some(src) = self.panels.comments.source.clone() else {
            return;
        };
        if let Err(error) = sidecars::save_comments(
            &src.to_string_lossy(),
            self.panels.comments.comments.clone(),
        ) {
            self.state.status_text = format!("评论保存失败：{error}");
            return;
        }
        self.refresh_right_panel();
    }

    /// 番茄钟计时器到点。到零就自动暂停并记一条 focus_session。
    pub(super) fn on_pomodoro_tick(&mut self) {
        self.panels.pomodoro_timer_armed = false;
        if self.panels.pomodoro.tick() {
            self.publish_notification(
                crate::ui::notifications::Category::Schedule,
                "专注计时结束",
                "这一轮专注已完成，休息一下吧。",
            );
            self.record_home_activity(mochi_core::analytics::events::ActivityInput {
                kind: Some(mochi_core::analytics::events::ActivityEventType::FocusSession),
                duration_minutes: Some((pomodoro::SECONDS / 60) as f64),
                ..Default::default()
            });
        }
    }
}
