//! 保存考试草稿，并处理考试页面的编辑和打开操作。
use super::*;

impl App {
    pub(super) fn remember_exam_drafts(&self) {
        if let Some((path, viewer::Content::Exam(s))) = self.viewer_tab() {
            for b in 0..s.blocks.len() {
                self.settings.set(
                    &format!("mochi:exam-draft:{}#{b}", path.to_string_lossy()),
                    &serde_json::json!({"answers":s.answers[b],"notes":s.notes[b]}).to_string(),
                );
            }
            let _ = self.settings.flush();
        }
    }

    pub(super) fn load_exam_drafts(&mut self) {
        let Some((path, viewer::Content::Exam(s))) = self.viewer_tab() else {
            return;
        };
        if s.drafts_loaded {
            return;
        }
        let drafts = (0..s.blocks.len())
            .map(|b| {
                self.settings
                    .get(&format!("mochi:exam-draft:{}#{b}", path.to_string_lossy()))
                    .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        if let Some(viewer::Content::Exam(s)) = self.viewer_content_mut() {
            for (b, d) in drafts.into_iter().enumerate() {
                s.answers[b] = d["answers"].as_object().cloned().unwrap_or_default();
                s.notes[b] = d["notes"].as_object().cloned().unwrap_or_default();
            }
            s.drafts_loaded = true;
        }
    }

    pub(super) fn on_exam_click(&mut self, hit: exam_view::Hit) {
        use exam_view::Hit;
        if hit == Hit::Source {
            self.toggle_source_mode();
            return;
        }
        let Some((path, viewer::Content::Exam(s))) = self.viewer_tab() else {
            return;
        };
        let path = path.to_path_buf();
        match hit {
            Hit::Pick(b, q, i) => {
                if let Some(viewer::Content::Exam(s)) = self.viewer_content_mut() {
                    s.pick(b, q, i);
                }
            }
            Hit::Analysis(b, q) => {
                if let Some(viewer::Content::Exam(s)) = self.viewer_content_mut() {
                    if !s.analysis.insert((b, q)) {
                        s.analysis.remove(&(b, q));
                    }
                }
            }
            Hit::Reset(b) => {
                if let Some(viewer::Content::Exam(s)) = self.viewer_content_mut() {
                    s.revealed[b] = false;
                    s.replay[b] = None;
                }
            }
            Hit::Replay(b, h) => {
                if let Some(viewer::Content::Exam(s)) = self.viewer_content_mut() {
                    s.replay[b] = Some(h);
                }
            }
            Hit::Short(b, q) => {
                if !s.locked(b) {
                    self.open_exam_text(path, b, Some(q), false);
                }
            }
            Hit::Note(b, q) => {
                if s.replay[b].is_none() {
                    self.open_exam_text(path, b, q, true);
                }
            }
            Hit::Submit(_) | Hit::Template => {
                let block = if let Hit::Submit(b) = hit {
                    Some(b)
                } else {
                    None
                };
                let result = (|| -> anyhow::Result<String> {
                    let current = std::fs::read_to_string(&path)?;
                    if current != s.raw {
                        anyhow::bail!("试卷已被外部修改，请重新打开后提交")
                    }
                    let next = if let Some(b) = block {
                        let mut model = s.blocks[b].model.clone();
                        let mut entry = serde_json::json!({"submitted_at":Self::now_ms()/1000,"answers":s.answers[b],"score":mochi_core::exam::score(&model,&s.answers[b])});
                        let notes = s.notes[b]
                            .iter()
                            .filter(|(_, v)| v.as_str().is_some_and(|v| !v.trim().is_empty()))
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect::<serde_json::Map<_, _>>();
                        if !notes.is_empty() {
                            entry["notes"] = notes.into();
                        }
                        model["history"].as_array_mut().unwrap().push(entry);
                        mochi_core::exam::update(&s.raw, b, &model)?
                    } else {
                        format!("{}\n{}", s.raw, mochi_core::exam::template())
                    };
                    mochi_core::files::FileService::new().write_file_safe(&path, &next)?;
                    Ok(next)
                })();
                if let Some(viewer::Content::Exam(s)) = self.viewer_content_mut() {
                    match result {
                        Ok(raw) => {
                            s.raw = raw;
                            s.blocks = mochi_core::exam::parse(&s.raw);
                            if let Some(b) = block {
                                s.revealed[b] = true;
                                s.replay[b] = None;
                            } else {
                                *s = exam_view::State::new(s.raw.clone());
                            }
                        }
                        Err(e) => s.error = e.to_string(),
                    }
                }
            }
            Hit::Source => {}
        }
        self.remember_exam_drafts();
    }

    pub(super) fn open_exam_text(
        &mut self,
        path: PathBuf,
        block: usize,
        question: Option<usize>,
        note: bool,
    ) {
        let Some((_, viewer::Content::Exam(s))) = self.viewer_tab() else {
            return;
        };
        let id = question
            .and_then(|q| s.question(block, q))
            .and_then(|q| q["id"].as_str())
            .unwrap_or("global");
        let value = if note {
            &s.notes[block]
        } else {
            &s.answers[block]
        };
        let mut field = TextField::new(if note {
            "记录你的想法、易错点……"
        } else {
            "在此作答……"
        });
        field.set_text(value.get(id).and_then(|v| v.as_str()).unwrap_or(""));
        self.dialog = Some(Dialog {
            title: if note { "备注" } else { "简答题作答" }.into(),
            description: String::new(),
            field: Some(field),
            error: String::new(),
            note: None,
            buttons: vec![
                DialogButton {
                    label: "取消".into(),
                    kind: ButtonKind::Ghost,
                    action: DialogAction::Dismiss,
                },
                DialogButton {
                    label: "保存".into(),
                    kind: ButtonKind::Primary,
                    action: DialogAction::ExamText {
                        path,
                        block,
                        question,
                        note,
                    },
                },
            ],
            dismiss: DialogAction::Dismiss,
            hover: None,
        });
        self.focus = Focus::Dialog;
    }
}
