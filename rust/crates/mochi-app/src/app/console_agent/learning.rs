//! 组织控制台中的学习内容和页面数据。
use super::*;
impl App {
    pub(super) fn console_learning(
        &mut self,
        name: &str,
        action: &str,
        a: &Value,
    ) -> CResult<Value> {
        let root = self.console_root()?;
        let d = data(a);
        if name == "pomodoro_manage" {
            if action == "history" {
                let now = chrono::Local::now();
                let days = d["days"].as_i64().unwrap_or(30).clamp(1, 365);
                return Ok(json!(
                    mochi_core::analytics::events::read_events(
                        &root,
                        now - chrono::Duration::days(days),
                        now
                    )
                    .into_iter()
                    .filter(|e| e.kind
                        == mochi_core::analytics::events::ActivityEventType::FocusSession)
                    .collect::<Vec<_>>()
                ));
            }
            match action {
                "get" => {}
                "start" => {
                    ensure!(!self.panels.pomodoro.running, "番茄钟已经运行");
                    self.panels.pomodoro.reset();
                    self.panels.pomodoro.start();
                }
                "resume" => {
                    ensure!(!self.panels.pomodoro.running, "番茄钟已经运行");
                    self.panels.pomodoro.start();
                }
                "pause" => self.panels.pomodoro.pause(),
                "reset" => self.panels.pomodoro.reset(),
                _ => bail!("未知番茄钟操作"),
            }
            return Ok(
                json!({"running":self.panels.pomodoro.running,"remainingSeconds":self.panels.pomodoro.remaining_now(),"complete":self.panels.pomodoro.is_complete()}),
            );
        }
        if name == "english_manage" {
            let svc = mochi_core::english_lab::Service::new(&root);
            self.console_path(&svc.path().to_string_lossy(), action != "get")?;
            let limit = d["limit"].as_u64().unwrap_or(30).clamp(1, 200) as usize;
            return Ok(match action {
                "get" => match d["section"].as_str().unwrap_or("dashboard") {
                    "dashboard" => json!(svc.dashboard()?),
                    "due" => json!(svc.due_words(limit)?),
                    "words" => json!(svc.recent_words(limit)?),
                    "dictionaries" => {
                        json!({"dictionaries":svc.dictionary_sources()?,"customLists":svc.custom_word_lists()?})
                    }
                    "stats" => json!(svc.learning_stats()?),
                    "history" => json!(svc.recent_reviews(limit)?),
                    "articles" => json!(svc.articles(limit)?),
                    "sentences" => json!(svc.sentences(limit)?),
                    _ => bail!("未知 section"),
                },
                "add_word" => json!({"id":svc.add_word(text(d,"word")?,text(d,"meaning")?)?}),
                "import_words" => {
                    json!({"count":svc.import_word_list(text(d,"name")?,text(d,"content")?)?})
                }
                "remove_dictionary" => json!(svc.remove_dictionary(text(a, "id")?)?),
                "add_article" => {
                    json!({"id":svc.add_article(text(d,"title")?,text(d,"content")?,d["translation"].as_str().unwrap_or(""),d["difficulty"].as_i64().unwrap_or(3),d["source"].as_str().unwrap_or("agent"))?})
                }
                "add_sentence" => {
                    json!({"id":svc.add_sentence(text(d,"content")?,d["translation"].as_str().unwrap_or(""),d["difficulty"].as_i64().unwrap_or(3),d["source"].as_str().unwrap_or("agent"))?})
                }
                "grade" => {
                    let grade = d["grade"].as_u64().context("缺少 grade")?;
                    ensure!((1..=4).contains(&grade), "grade 范围 1–4");
                    svc.grade_fsrs(
                        text(a, "id")?.parse()?,
                        grade as u8,
                        d["elapsedMs"].as_u64().unwrap_or(0),
                    )?;
                    json!({"recorded":true})
                }
                "dictation" => json!(svc.grade_dictation(
                    text(a, "id")?.parse()?,
                    text(d, "answer")?,
                    d["elapsedMs"].as_u64().unwrap_or(0)
                )?),
                _ => bail!("未知英语学习操作"),
            });
        }
        ensure!(name == "exam_manage", "未知学习模块");
        let path = self.console_target(a, !matches!(action, "get" | "open"))?;
        ensure!(
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exam")),
            "需要 .exam 文件"
        );
        ensure!(self.shell.open_file_with_mode(&path, true), "无法打开试卷");
        self.state.view = WorkspaceView::Editor;
        self.sync_state();
        self.load_exam_drafts();
        let block = d["block"].as_u64().unwrap_or(0) as usize;
        let s = match self.viewer_content_mut() {
            Some(viewer::Content::Exam(s)) => s,
            _ => bail!("试卷尚未加载"),
        };
        ensure!(block < s.blocks.len(), "试卷 block 越界");
        match action {
            "get" | "open" => {}
            "answer" => {
                ensure!(!s.locked(block), "当前试卷已经提交或正在回看历史");
                let id = text(d, "questionId")?;
                ensure!(
                    s.blocks[block].model["questions"]
                        .as_array()
                        .is_some_and(|qs| qs.iter().any(|q| q["id"] == id)),
                    "题目不存在"
                );
                let answer = d.get("answer").context("缺少 answer")?;
                s.answers[block].insert(id.into(), answer.clone());
                self.remember_exam_drafts();
                self.settings.flush()?;
            }
            "reset" => {
                s.answers[block].clear();
                s.notes[block].clear();
                s.revealed[block] = false;
                s.replay[block] = None;
                self.remember_exam_drafts();
                self.settings.flush()?;
            }
            "submit" => {
                ensure!(!s.locked(block), "本次答卷已经提交");
                self.on_exam_click(exam_view::Hit::Submit(block));
                let s = match self.viewer_content_mut() {
                    Some(viewer::Content::Exam(s)) => s,
                    _ => bail!("试卷已关闭"),
                };
                ensure!(s.error.is_empty(), "提交失败：{}", s.error);
            }
            "update" => {
                ensure!(
                    s.answers.iter().all(|a| a.is_empty()),
                    "存在作答草稿，请先提交或重置"
                );
                check_revision(a, &json!(s.raw))?;
                ensure!(std::fs::read_to_string(&path)? == s.raw, "试卷已被外部修改");
                let model = d.get("model").context("缺少 model")?;
                let next = mochi_core::exam::update(
                    &s.raw,
                    block,
                    &mochi_core::exam::normalize(model.clone()),
                )?;
                mochi_core::files::FileService::new().write_file_safe(&path, &next)?;
                *s = exam_view::State::new(next);
            }
            _ => bail!("未知试卷操作"),
        }
        let s = match self.viewer_content_mut() {
            Some(viewer::Content::Exam(s)) => s,
            _ => bail!("试卷已关闭"),
        };
        Ok(
            json!({"revision":revision(&json!(s.raw)),"blocks":s.blocks.iter().map(|b|&b.model).collect::<Vec<_>>(),"answers":s.answers,"submitted":s.revealed,"scores":s.blocks.iter().enumerate().map(|(i,b)|mochi_core::exam::score(&b.model,&s.answers[i])).collect::<Vec<_>>()}),
        )
    }
}
