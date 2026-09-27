//! 绘制英语学习页面及其导航内容。
use super::*;

impl App {
    pub(super) fn paint_english_lab(&mut self, area: Rect, p: &Palette) {
        self.english_lab_hits.clear();
        self.paint_english_lab_navigation(area, p);
        if self.english_lab_page != EnglishLabPage::Dashboard {
            self.paint_english_lab_page(area, p);
            return;
        }
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            crate::view::placeholder(&mut self.list, area, "请先打开工作区", p);
            return;
        };
        let service = mochi_core::english_lab::Service::new(root);
        let Ok(dashboard) = service.dashboard() else {
            crate::view::placeholder(&mut self.list, area, "English Lab 数据库读取失败", p);
            return;
        };
        self.list.text(
            Rect::new(
                area.left + 30.0,
                area.top + 24.0,
                area.right - 190.0,
                area.top + 58.0,
            ),
            "English Lab",
            TextStyle::Large,
            p.foreground,
        );
        self.list.text(
            Rect::new(
                area.left + 30.0,
                area.top + 62.0,
                area.right - 190.0,
                area.top + 84.0,
            ),
            "原生词库、间隔复习与学习统计（兼容旧版数据）。",
            TextStyle::Caption,
            p.muted,
        );
        let import = Rect::new(
            area.right - 286.0,
            area.top + 26.0,
            area.right - 160.0,
            area.top + 58.0,
        );
        self.list.rounded_rect(import, 6.0, p.surface_elevated);
        self.list
            .text(import, "导入词典/词表", TextStyle::Caption, p.foreground);
        self.english_lab_hits
            .push((import, EnglishLabHit::ImportDictionary));
        let add = Rect::new(
            area.right - 150.0,
            area.top + 26.0,
            area.right - 30.0,
            area.top + 58.0,
        );
        self.list.rounded_rect(add, 6.0, p.accent);
        self.list
            .text(add, "＋ 添加单词", TextStyle::Caption, p.accent_foreground);
        self.english_lab_hits.push((add, EnglishLabHit::AddWord));
        let stats = [
            ("词库", dashboard.words.to_string()),
            ("今日待复习", dashboard.due.to_string()),
            ("今日新词", dashboard.learned_today.to_string()),
            ("今日正确率", format!("{:.0}%", dashboard.accuracy * 100.0)),
        ];
        let width = (area.width() - 60.0) / 4.0;
        for (index, (label, value)) in stats.iter().enumerate() {
            let x = area.left + 30.0 + index as f32 * width;
            let card = Rect::new(x, area.top + 106.0, x + width - 10.0, area.top + 186.0);
            self.list.rounded_rect(card, 8.0, p.surface);
            self.list.text(
                Rect::new(
                    card.left + 14.0,
                    card.top + 13.0,
                    card.right - 14.0,
                    card.top + 33.0,
                ),
                *label,
                TextStyle::Caption,
                p.muted,
            );
            self.list.text(
                Rect::new(
                    card.left + 14.0,
                    card.top + 37.0,
                    card.right - 14.0,
                    card.bottom - 10.0,
                ),
                value,
                TextStyle::Large,
                p.foreground,
            );
        }
        self.list.text(
            Rect::new(
                area.left + 30.0,
                area.top + 211.0,
                area.right - 30.0,
                area.top + 238.0,
            ),
            "今日复习",
            TextStyle::Label,
            p.foreground,
        );
        match service.due_words(6) {
            Ok(words) if words.is_empty() => self.list.text(
                Rect::new(
                    area.left + 30.0,
                    area.top + 248.0,
                    area.right - 30.0,
                    area.top + 272.0,
                ),
                "暂时没有到期词汇。添加词条后即可开始复习。",
                TextStyle::Caption,
                p.muted,
            ),
            Ok(words) => {
                for (index, word) in words.iter().enumerate() {
                    let y = area.top + 246.0 + index as f32 * 56.0;
                    let row = Rect::new(area.left + 30.0, y, area.right - 30.0, y + 46.0);
                    self.list.rounded_rect(row, 6.0, p.surface);
                    self.list.text(
                        Rect::new(
                            row.left + 14.0,
                            row.top + 8.0,
                            row.left + 180.0,
                            row.bottom - 8.0,
                        ),
                        &word.word,
                        TextStyle::Label,
                        p.foreground,
                    );
                    self.list.text(
                        Rect::new(
                            row.left + 190.0,
                            row.top + 8.0,
                            row.right - 224.0,
                            row.bottom - 8.0,
                        ),
                        &word.meaning,
                        TextStyle::Caption,
                        p.muted,
                    );
                    for (offset, label, grade) in [
                        (0.0, "忘记", 1u8),
                        (48.0, "困难", 2),
                        (96.0, "记得", 3),
                        (144.0, "简单", 4),
                    ] {
                        let r = Rect::new(
                            row.right - 202.0 + offset,
                            row.top + 11.0,
                            row.right - 158.0 + offset,
                            row.bottom - 11.0,
                        );
                        self.list.text(
                            r,
                            label,
                            TextStyle::Caption,
                            if grade >= 3 { p.accent } else { p.muted },
                        );
                        self.english_lab_hits
                            .push((r, EnglishLabHit::Grade(word.id, grade)));
                    }
                }
            }
            Err(error) => self.list.text(
                Rect::new(
                    area.left + 30.0,
                    area.top + 248.0,
                    area.right - 30.0,
                    area.top + 272.0,
                ),
                format!("读取复习队列失败：{error}"),
                TextStyle::Caption,
                p.danger,
            ),
        }
    }

    pub(super) fn paint_english_lab_navigation(&mut self, area: Rect, p: &Palette) {
        let pages = [
            (EnglishLabPage::Dashboard, "概览"),
            (EnglishLabPage::Practice, "训练"),
            (EnglishLabPage::Listening, "听力"),
            (EnglishLabPage::Reading, "阅读"),
            (EnglishLabPage::Dictionaries, "词库"),
            (EnglishLabPage::Library, "内容库"),
            (EnglishLabPage::Stats, "统计"),
            (EnglishLabPage::History, "历史"),
            (EnglishLabPage::Settings, "设置"),
        ];
        let mut x = area.left + 28.0;
        let y = area.top + 86.0;
        for (page, label) in pages {
            let width = 58.0;
            let rect = Rect::new(x, y, x + width, y + 18.0);
            if self.english_lab_page == page {
                self.list.rounded_rect(rect, 4.0, p.accent_hover);
            }
            self.list.text(
                rect,
                label,
                TextStyle::Caption,
                if self.english_lab_page == page {
                    p.accent
                } else {
                    p.muted
                },
            );
            self.english_lab_hits
                .push((rect, EnglishLabHit::Page(page)));
            x += width + 6.0;
        }
    }

    pub(super) fn paint_english_lab_page(&mut self, area: Rect, p: &Palette) {
        let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) else {
            crate::view::placeholder(&mut self.list, area, "请先打开工作区", p);
            return;
        };
        let service = mochi_core::english_lab::Service::new(root);
        let content = Rect::new(
            area.left + 30.0,
            area.top + 122.0,
            area.right - 30.0,
            area.bottom - 24.0,
        );
        let (title, subtitle) = match self.english_lab_page {
            EnglishLabPage::Practice => (
                "单词训练",
                if self.english_lab_practice_group.is_some() {
                    "当前仅训练所选词书的到期词；再次点击“训练”可返回全局队列。"
                } else {
                    "基于同一份 FSRS 队列复习到期词。"
                },
            ),
            EnglishLabPage::Listening => (
                "听力与听写",
                "使用 Windows 本机语音播放句子；题目与作答保存在旧版兼容数据库。",
            ),
            EnglishLabPage::Reading => (
                "阅读训练",
                "浏览旧版文章库；文章内容与译文保留在同一份本地数据库。",
            ),
            EnglishLabPage::Library => ("内容库", "本地词库中的最近词条与当前掌握度。"),
            EnglishLabPage::Dictionaries => {
                ("词库中心", "已导入词典包直接来自 Electron 兼容数据库。")
            }
            EnglishLabPage::Stats => ("学习统计", "由真实作答记录汇总，不使用演示数据。"),
            EnglishLabPage::History => ("训练历史", "逐题记录保留在当前工作区。"),
            EnglishLabPage::Settings => ("设置", "English Lab 数据库与导入管理。"),
            EnglishLabPage::Dashboard => return,
        };
        self.list.text(
            Rect::new(
                content.left,
                area.top + 24.0,
                content.right,
                area.top + 58.0,
            ),
            title,
            TextStyle::Large,
            p.foreground,
        );
        self.list.text(
            Rect::new(
                content.left,
                area.top + 62.0,
                content.right,
                area.top + 82.0,
            ),
            subtitle,
            TextStyle::Caption,
            p.muted,
        );
        match self.english_lab_page {
            EnglishLabPage::Practice => match self
                .english_lab_practice_group
                .map(|group_id| service.due_words_in_group(group_id, 12))
                .unwrap_or_else(|| service.due_words(12))
            {
                Ok(words) if words.is_empty() => self.list.text(
                    Rect::new(content.left, content.top, content.right, content.top + 28.0),
                    "暂无到期词。添加词条或等待下一次复习即可。",
                    TextStyle::Caption,
                    p.muted,
                ),
                Ok(words) => {
                    for (index, word) in words.iter().enumerate() {
                        let y = content.top + index as f32 * 54.0;
                        let row = Rect::new(content.left, y, content.right, y + 44.0);
                        self.list.rounded_rect(row, 6.0, p.surface);
                        self.list.text(
                            Rect::new(
                                row.left + 14.0,
                                row.top + 7.0,
                                row.left + 200.0,
                                row.bottom - 6.0,
                            ),
                            &word.word,
                            TextStyle::Label,
                            p.foreground,
                        );
                        self.list.text(
                            Rect::new(
                                row.left + 210.0,
                                row.top + 8.0,
                                row.right - 216.0,
                                row.bottom - 6.0,
                            ),
                            &word.meaning,
                            TextStyle::Caption,
                            p.muted,
                        );
                        for (offset, label, grade) in [
                            (0.0, "忘记", 1u8),
                            (48.0, "困难", 2),
                            (96.0, "记得", 3),
                            (144.0, "简单", 4),
                        ] {
                            let hit = Rect::new(
                                row.right - 204.0 + offset,
                                row.top + 11.0,
                                row.right - 160.0 + offset,
                                row.bottom - 10.0,
                            );
                            self.list.text(
                                hit,
                                label,
                                TextStyle::Caption,
                                if grade >= 3 { p.accent } else { p.muted },
                            );
                            self.english_lab_hits
                                .push((hit, EnglishLabHit::Grade(word.id, grade)));
                        }
                    }
                }
                Err(error) => self.list.text(
                    Rect::new(content.left, content.top, content.right, content.top + 28.0),
                    format!("读取训练队列失败：{error}"),
                    TextStyle::Caption,
                    p.danger,
                ),
            },
            EnglishLabPage::Listening => {
                let add = Rect::new(
                    content.right - 136.0,
                    content.top,
                    content.right,
                    content.top + 32.0,
                );
                self.list.rounded_rect(add, 6.0, p.accent);
                self.list
                    .text(add, "＋ 添加句子", TextStyle::Caption, p.accent_foreground);
                self.english_lab_hits
                    .push((add, EnglishLabHit::AddSentence));
                match service.sentences(16) {
                    Ok(sentences) if sentences.is_empty() => self.list.text(
                        Rect::new(
                            content.left,
                            content.top + 48.0,
                            content.right,
                            content.top + 76.0,
                        ),
                        "听力句库为空。添加一句英文后，可直接调用 Windows 语音朗读并进行听写。",
                        TextStyle::Caption,
                        p.muted,
                    ),
                    Ok(sentences) => {
                        for (index, sentence) in sentences.iter().enumerate() {
                            let y = content.top + 48.0 + index as f32 * 64.0;
                            let card = Rect::new(content.left, y, content.right, y + 54.0);
                            self.list.rounded_rect(card, 7.0, p.surface);
                            self.list.text(
                                Rect::new(
                                    card.left + 14.0,
                                    card.top + 7.0,
                                    card.right - 190.0,
                                    card.top + 29.0,
                                ),
                                &sentence.content,
                                TextStyle::Label,
                                p.foreground,
                            );
                            self.list.text(
                                Rect::new(
                                    card.left + 14.0,
                                    card.top + 31.0,
                                    card.right - 190.0,
                                    card.bottom - 5.0,
                                ),
                                format!(
                                    "{}  ·  难度 {}",
                                    sentence.translation, sentence.difficulty
                                ),
                                TextStyle::Caption,
                                p.muted,
                            );
                            let listen = Rect::new(
                                card.right - 164.0,
                                card.top + 13.0,
                                card.right - 96.0,
                                card.bottom - 11.0,
                            );
                            self.list.rounded_rect(listen, 5.0, p.surface_elevated);
                            self.list
                                .text(listen, "▶ 播放", TextStyle::Caption, p.foreground);
                            self.english_lab_hits
                                .push((listen, EnglishLabHit::Listen(sentence.content.clone())));
                            let dictate = Rect::new(
                                card.right - 86.0,
                                card.top + 13.0,
                                card.right - 10.0,
                                card.bottom - 11.0,
                            );
                            self.list.rounded_rect(dictate, 5.0, p.accent_hover);
                            self.list
                                .text(dictate, "听写", TextStyle::Caption, p.accent);
                            self.english_lab_hits
                                .push((dictate, EnglishLabHit::Dictate(sentence.id)));
                        }
                    }
                    Err(error) => self.list.text(
                        Rect::new(
                            content.left,
                            content.top + 48.0,
                            content.right,
                            content.top + 76.0,
                        ),
                        format!("读取听力句库失败：{error}"),
                        TextStyle::Caption,
                        p.danger,
                    ),
                }
            }
            EnglishLabPage::Reading => {
                if let Some(article_id) = self.english_lab_article {
                    match service.article(article_id) {
                        Ok(article) => {
                            let back = Rect::new(
                                content.left,
                                content.top,
                                content.left + 82.0,
                                content.top + 30.0,
                            );
                            self.list.rounded_rect(back, 5.0, p.surface_elevated);
                            self.list
                                .text(back, "← 文章列表", TextStyle::Caption, p.foreground);
                            self.english_lab_hits
                                .push((back, EnglishLabHit::CloseArticle));
                            self.list.text(
                                Rect::new(
                                    content.left,
                                    content.top + 46.0,
                                    content.right,
                                    content.top + 78.0,
                                ),
                                &article.title,
                                TextStyle::Large,
                                p.foreground,
                            );
                            self.list.text(
                                Rect::new(
                                    content.left,
                                    content.top + 82.0,
                                    content.right,
                                    content.top + 104.0,
                                ),
                                format!("难度 {} · {}", article.difficulty, article.source),
                                TextStyle::Caption,
                                p.muted,
                            );
                            let mut y = content.top + 120.0;
                            for paragraph in article
                                .content
                                .split("\n\n")
                                .filter(|text| !text.trim().is_empty())
                                .take(7)
                            {
                                self.list.text(
                                    Rect::new(
                                        content.left + 12.0,
                                        y,
                                        content.right - 12.0,
                                        y + 42.0,
                                    ),
                                    paragraph.trim(),
                                    TextStyle::Body,
                                    p.foreground,
                                );
                                y += 48.0;
                            }
                            if !article.translation.trim().is_empty() && y + 62.0 < content.bottom {
                                self.list.rounded_rect(
                                    Rect::new(content.left, y + 6.0, content.right, y + 58.0),
                                    6.0,
                                    p.surface,
                                );
                                self.list.text(
                                    Rect::new(
                                        content.left + 12.0,
                                        y + 14.0,
                                        content.right - 12.0,
                                        y + 52.0,
                                    ),
                                    &article.translation,
                                    TextStyle::Caption,
                                    p.muted,
                                );
                            }
                        }
                        Err(error) => self.list.text(
                            Rect::new(content.left, content.top, content.right, content.top + 28.0),
                            format!("读取文章失败：{error}"),
                            TextStyle::Caption,
                            p.danger,
                        ),
                    }
                    return;
                }
                let add = Rect::new(
                    content.right - 136.0,
                    content.top,
                    content.right,
                    content.top + 32.0,
                );
                self.list.rounded_rect(add, 6.0, p.accent);
                self.list
                    .text(add, "＋ 添加文章", TextStyle::Caption, p.accent_foreground);
                self.english_lab_hits.push((add, EnglishLabHit::AddArticle));
                match service.articles(20) {
                    Ok(articles) if articles.is_empty() => self.list.text(
                        Rect::new(
                            content.left,
                            content.top + 48.0,
                            content.right,
                            content.top + 76.0,
                        ),
                        "文章库为空。可添加英文正文和译文，数据会与旧版 Electron 共用。",
                        TextStyle::Caption,
                        p.muted,
                    ),
                    Ok(articles) => {
                        for (index, article) in articles.iter().enumerate() {
                            let y = content.top + 48.0 + index as f32 * 76.0;
                            let card = Rect::new(content.left, y, content.right, y + 64.0);
                            self.list.rounded_rect(card, 7.0, p.surface);
                            self.list.text(
                                Rect::new(
                                    card.left + 14.0,
                                    card.top + 9.0,
                                    card.right - 14.0,
                                    card.top + 31.0,
                                ),
                                &article.title,
                                TextStyle::Label,
                                p.foreground,
                            );
                            let preview = article.content.chars().take(110).collect::<String>();
                            self.list.text(
                                Rect::new(
                                    card.left + 14.0,
                                    card.top + 35.0,
                                    card.right - 14.0,
                                    card.bottom - 7.0,
                                ),
                                format!("{}  ·  难度 {}", preview, article.difficulty),
                                TextStyle::Caption,
                                p.muted,
                            );
                            self.english_lab_hits
                                .push((card, EnglishLabHit::OpenArticle(article.id)));
                        }
                    }
                    Err(error) => self.list.text(
                        Rect::new(
                            content.left,
                            content.top + 48.0,
                            content.right,
                            content.top + 76.0,
                        ),
                        format!("读取文章库失败：{error}"),
                        TextStyle::Caption,
                        p.danger,
                    ),
                }
            }
            EnglishLabPage::Library => match service.recent_words(24) {
                Ok(words) if words.is_empty() => self.list.text(
                    Rect::new(content.left, content.top, content.right, content.top + 28.0),
                    "内容库还是空的。可从词库中心导入 JSON 词典包。",
                    TextStyle::Caption,
                    p.muted,
                ),
                Ok(words) => {
                    for (index, word) in words.iter().enumerate() {
                        let y = content.top + index as f32 * 30.0;
                        self.list.text(
                            Rect::new(content.left + 12.0, y + 4.0, content.left + 190.0, y + 27.0),
                            &word.word,
                            TextStyle::Caption,
                            p.foreground,
                        );
                        self.list.text(
                            Rect::new(
                                content.left + 200.0,
                                y + 4.0,
                                content.right - 130.0,
                                y + 27.0,
                            ),
                            &word.meaning,
                            TextStyle::Caption,
                            p.muted,
                        );
                        self.list.text(
                            Rect::new(
                                content.right - 118.0,
                                y + 4.0,
                                content.right - 8.0,
                                y + 27.0,
                            ),
                            format!("掌握 {:.0}%", word.mastery),
                            TextStyle::Caption,
                            p.muted,
                        );
                    }
                }
                Err(error) => self.list.text(
                    Rect::new(content.left, content.top, content.right, content.top + 28.0),
                    format!("读取内容库失败：{error}"),
                    TextStyle::Caption,
                    p.danger,
                ),
            },
            EnglishLabPage::Dictionaries => {
                let import = Rect::new(
                    content.right - 142.0,
                    content.top,
                    content.right,
                    content.top + 32.0,
                );
                self.list.rounded_rect(import, 6.0, p.accent);
                self.list.text(
                    import,
                    "导入词典/词表",
                    TextStyle::Caption,
                    p.accent_foreground,
                );
                self.english_lab_hits
                    .push((import, EnglishLabHit::ImportDictionary));
                match service.dictionary_sources() {
                    Ok(sources) if sources.is_empty() => self.list.text(
                        Rect::new(
                            content.left,
                            content.top + 48.0,
                            content.right,
                            content.top + 76.0,
                        ),
                        "尚未安装词典/词表。支持原 Electron schemaVersion 2 JSON，以及 CSV、TSV 和文本词表。",
                        TextStyle::Caption,
                        p.muted,
                    ),
                    Ok(sources) => {
                        for (index, source) in sources.iter().enumerate() {
                            let y = content.top + 48.0 + index as f32 * 44.0;
                            let row = Rect::new(content.left, y, content.right, y + 34.0);
                            self.list.rounded_rect(row, 6.0, p.surface);
                            self.list.text(
                                Rect::new(
                                    row.left + 12.0,
                                    row.top + 6.0,
                                    row.left + 260.0,
                                    row.bottom - 4.0,
                                ),
                                &source.name,
                                TextStyle::Label,
                                p.foreground,
                            );
                            self.list.text(
                                Rect::new(
                                    row.left + 270.0,
                                    row.top + 7.0,
                                    row.right - 74.0,
                                    row.bottom - 4.0,
                                ),
                                format!(
                                    "{} · L{} · {} 词",
                                    source.category, source.level, source.word_count
                                ),
                                TextStyle::Caption,
                                p.muted,
                            );
                            let remove = Rect::new(
                                row.right - 64.0,
                                row.top + 5.0,
                                row.right - 10.0,
                                row.bottom - 4.0,
                            );
                            if let Some(group_id) = source.group_id {
                                let practice = Rect::new(
                                    row.right - 126.0,
                                    row.top + 5.0,
                                    row.right - 70.0,
                                    row.bottom - 4.0,
                                );
                                self.list
                                    .text(practice, "练这本", TextStyle::Caption, p.accent);
                                self.english_lab_hits
                                    .push((practice, EnglishLabHit::PracticeGroup(group_id)));
                            }
                            self.list.text(remove, "移除", TextStyle::Caption, p.danger);
                            self.english_lab_hits.push((
                                remove,
                                EnglishLabHit::RemoveDictionary(source.book_id.clone()),
                            ));
                        }
                    }
                    Err(error) => self.list.text(
                        Rect::new(
                            content.left,
                            content.top + 48.0,
                            content.right,
                            content.top + 76.0,
                        ),
                        format!("读取词库失败：{error}"),
                        TextStyle::Caption,
                        p.danger,
                    ),
                }
                // Electron 的自定义词表是 `groups_data(type='user')`，不属于
                // dictionary_sources；单独展示才不会让已导入的 CSV/TSV 看似丢失。
                if let Ok(lists) = service.custom_word_lists() {
                    let source_count = service
                        .dictionary_sources()
                        .map(|sources| sources.len())
                        .unwrap_or(0);
                    for (index, list) in lists.iter().enumerate() {
                        let y = content.top + 92.0 + (source_count + index) as f32 * 44.0;
                        let row = Rect::new(content.left, y, content.right, y + 34.0);
                        self.list.rounded_rect(row, 6.0, p.surface);
                        self.list.text(
                            Rect::new(
                                row.left + 12.0,
                                row.top + 6.0,
                                row.left + 280.0,
                                row.bottom - 4.0,
                            ),
                            format!("{}  ·  自定义词表", list.name),
                            TextStyle::Label,
                            p.foreground,
                        );
                        self.list.text(
                            Rect::new(
                                row.left + 290.0,
                                row.top + 7.0,
                                row.right - 12.0,
                                row.bottom - 4.0,
                            ),
                            format!("{} 词", list.word_count),
                            TextStyle::Caption,
                            p.muted,
                        );
                    }
                }
            }
            EnglishLabPage::Stats => match service.learning_stats() {
                Ok(stats) => {
                    let cards = [
                        ("累计答题", stats.total_answers.to_string()),
                        ("正确答案", stats.correct_answers.to_string()),
                        ("掌握词汇", stats.mastered_words.to_string()),
                        ("学习中", stats.learning_words.to_string()),
                        ("活跃天数", stats.active_days.to_string()),
                        ("学习时长", format!("{} 分钟", stats.total_seconds / 60)),
                    ];
                    for (index, (label, value)) in cards.iter().enumerate() {
                        let col = index % 3;
                        let row = index / 3;
                        let x = content.left + col as f32 * (content.width() / 3.0);
                        let y = content.top + row as f32 * 104.0;
                        let card = Rect::new(x, y, x + content.width() / 3.0 - 12.0, y + 88.0);
                        self.list.rounded_rect(card, 8.0, p.surface);
                        self.list.text(
                            Rect::new(
                                card.left + 14.0,
                                card.top + 12.0,
                                card.right - 14.0,
                                card.top + 34.0,
                            ),
                            *label,
                            TextStyle::Caption,
                            p.muted,
                        );
                        self.list.text(
                            Rect::new(
                                card.left + 14.0,
                                card.top + 38.0,
                                card.right - 14.0,
                                card.bottom - 8.0,
                            ),
                            value,
                            TextStyle::Large,
                            p.foreground,
                        );
                    }
                }
                Err(error) => self.list.text(
                    Rect::new(content.left, content.top, content.right, content.top + 28.0),
                    format!("读取统计失败：{error}"),
                    TextStyle::Caption,
                    p.danger,
                ),
            },
            EnglishLabPage::History => match service.recent_reviews(50) {
                Ok(rows) if rows.is_empty() => self.list.text(
                    Rect::new(content.left, content.top, content.right, content.top + 28.0),
                    "还没有训练历史。完成一次评分后会在这里保留记录。",
                    TextStyle::Caption,
                    p.muted,
                ),
                Ok(rows) => {
                    for (index, review) in rows.iter().enumerate() {
                        let y = content.top + index as f32 * 30.0;
                        let tone = if review.correct >= 1.0 {
                            p.accent
                        } else {
                            p.danger
                        };
                        self.list.text(
                            Rect::new(content.left + 10.0, y + 4.0, content.left + 180.0, y + 27.0),
                            &review.word,
                            TextStyle::Caption,
                            p.foreground,
                        );
                        self.list.text(
                            Rect::new(
                                content.left + 190.0,
                                y + 4.0,
                                content.right - 280.0,
                                y + 27.0,
                            ),
                            &review.meaning,
                            TextStyle::Caption,
                            p.muted,
                        );
                        self.list.text(
                            Rect::new(
                                content.right - 270.0,
                                y + 4.0,
                                content.right - 120.0,
                                y + 27.0,
                            ),
                            &review.created_at,
                            TextStyle::Caption,
                            p.muted,
                        );
                        self.list.text(
                            Rect::new(
                                content.right - 112.0,
                                y + 4.0,
                                content.right - 6.0,
                                y + 27.0,
                            ),
                            if review.correct >= 1.0 {
                                "正确"
                            } else {
                                "待加强"
                            },
                            TextStyle::Caption,
                            tone,
                        );
                    }
                }
                Err(error) => self.list.text(
                    Rect::new(content.left, content.top, content.right, content.top + 28.0),
                    format!("读取历史失败：{error}"),
                    TextStyle::Caption,
                    p.danger,
                ),
            },
            EnglishLabPage::Settings => {
                self.list.rounded_rect(
                    Rect::new(
                        content.left,
                        content.top,
                        content.right,
                        content.top + 112.0,
                    ),
                    8.0,
                    p.surface,
                );
                self.list.text(
                    Rect::new(
                        content.left + 16.0,
                        content.top + 14.0,
                        content.right - 16.0,
                        content.top + 38.0,
                    ),
                    "本地数据",
                    TextStyle::Label,
                    p.foreground,
                );
                self.list.text(
                    Rect::new(
                        content.left + 16.0,
                        content.top + 44.0,
                        content.right - 16.0,
                        content.top + 70.0,
                    ),
                    service.path().display().to_string(),
                    TextStyle::Caption,
                    p.muted,
                );
                let import = Rect::new(
                    content.left + 16.0,
                    content.top + 78.0,
                    content.left + 142.0,
                    content.top + 104.0,
                );
                self.list.rounded_rect(import, 5.0, p.surface_elevated);
                self.list
                    .text(import, "导入词典/词表", TextStyle::Caption, p.foreground);
                self.english_lab_hits
                    .push((import, EnglishLabHit::ImportDictionary));
            }
            EnglishLabPage::Dashboard => {}
        }
    }

    pub(super) fn on_english_lab_click(&mut self, x: f32, y: f32) {
        let hit = self
            .english_lab_hits
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, hit)| hit.clone());
        match hit {
            Some(EnglishLabHit::Page(page)) => {
                // 侧栏中的这一项提供了明确的退出入口。
                // 从 Electron 版的“单本练习”范围切回“所有到期词汇”。
                if page == EnglishLabPage::Practice && self.english_lab_page == page {
                    self.english_lab_practice_group = None;
                }
                self.english_lab_page = page;
                if page != EnglishLabPage::Practice {
                    self.english_lab_practice_group = None;
                }
                if page != EnglishLabPage::Reading {
                    self.english_lab_article = None;
                }
                self.invalidate_main();
            }
            Some(EnglishLabHit::PracticeGroup(group_id)) => {
                self.english_lab_practice_group = Some(group_id);
                self.english_lab_page = EnglishLabPage::Practice;
                self.invalidate_main();
            }
            Some(EnglishLabHit::OpenArticle(article_id)) => {
                self.english_lab_article = Some(article_id);
                self.invalidate_main();
            }
            Some(EnglishLabHit::CloseArticle) => {
                self.english_lab_article = None;
                self.invalidate_main();
            }
            Some(EnglishLabHit::AddWord) => {
                self.dialog = Some(Dialog {
                    title: "添加 English Lab 词条".into(),
                    description: "格式：英文单词 | 中文释义，例如 abandon | 放弃。".into(),
                    field: Some(TextField::new("word | meaning")),
                    error: String::new(),
                    note: None,
                    buttons: vec![
                        DialogButton {
                            label: "取消".into(),
                            kind: ButtonKind::Ghost,
                            action: DialogAction::Dismiss,
                        },
                        DialogButton {
                            label: "添加".into(),
                            kind: ButtonKind::Primary,
                            action: DialogAction::EnglishAddWord,
                        },
                    ],
                    dismiss: DialogAction::Dismiss,
                    hover: None,
                });
                self.focus = Focus::Dialog;
            }
            Some(EnglishLabHit::AddArticle) => {
                self.dialog = Some(Dialog {
                    title: "添加阅读文章".into(),
                    description: "格式：标题 | 英文正文 | 中文译文。正文可先填一段，之后仍可在旧版数据库中继续使用。".into(),
                    field: Some(TextField::new("标题 | English content | 中文译文")),
                    error: String::new(), note: None,
                    buttons: vec![DialogButton { label: "取消".into(), kind: ButtonKind::Ghost, action: DialogAction::Dismiss }, DialogButton { label: "添加文章".into(), kind: ButtonKind::Primary, action: DialogAction::EnglishAddArticle }],
                    dismiss: DialogAction::Dismiss, hover: None,
                });
                self.focus = Focus::Dialog;
            }
            Some(EnglishLabHit::AddSentence) => {
                self.dialog = Some(Dialog {
                    title: "添加听力句子".into(),
                    description: "格式：英文句子 | 中文译文。添加后可由 Windows 本机语音播放。"
                        .into(),
                    field: Some(TextField::new("English sentence | 中文译文")),
                    error: String::new(),
                    note: None,
                    buttons: vec![
                        DialogButton {
                            label: "取消".into(),
                            kind: ButtonKind::Ghost,
                            action: DialogAction::Dismiss,
                        },
                        DialogButton {
                            label: "添加句子".into(),
                            kind: ButtonKind::Primary,
                            action: DialogAction::EnglishAddSentence,
                        },
                    ],
                    dismiss: DialogAction::Dismiss,
                    hover: None,
                });
                self.focus = Focus::Dialog;
            }
            Some(EnglishLabHit::Listen(sentence)) => match platform::speak_text(&sentence) {
                Ok(()) => self.show_global_notice("正在使用 Windows 本机语音播放"),
                Err(error) => self.show_global_notice(error),
            },
            Some(EnglishLabHit::Dictate(sentence_id)) => {
                self.dialog = Some(Dialog {
                    title: "听写".into(),
                    description: "先点击播放，听完后输入你听到的英文。忽略大小写、空格和标点。"
                        .into(),
                    field: Some(TextField::new("输入英文句子")),
                    error: String::new(),
                    note: None,
                    buttons: vec![
                        DialogButton {
                            label: "取消".into(),
                            kind: ButtonKind::Ghost,
                            action: DialogAction::Dismiss,
                        },
                        DialogButton {
                            label: "提交听写".into(),
                            kind: ButtonKind::Primary,
                            action: DialogAction::EnglishGradeDictation { sentence_id },
                        },
                    ],
                    dismiss: DialogAction::Dismiss,
                    hover: None,
                });
                self.focus = Focus::Dialog;
            }
            Some(EnglishLabHit::ImportDictionary) => {
                if let Some(path) = platform::pick_file(HWND(self.hwnd_raw as *mut _)) {
                    if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                        let is_json = path
                            .extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
                        let service = mochi_core::english_lab::Service::new(root);
                        let result = if is_json {
                            service.import_dictionary_file(&path)
                        } else {
                            let name = path
                                .file_stem()
                                .and_then(|stem| stem.to_str())
                                .filter(|name| !name.trim().is_empty())
                                .unwrap_or("我的词表");
                            std::fs::read_to_string(&path)
                                .map_err(anyhow::Error::from)
                                .and_then(|contents| service.import_word_list(name, &contents))
                        };
                        match result {
                            Ok(count) => self.show_global_notice(format!(
                                "已导入 {count} 个词条，原有学习记录已保留"
                            )),
                            Err(error) => {
                                self.show_global_notice(format!("导入词典/词表失败：{error}"))
                            }
                        }
                    }
                    self.invalidate_main();
                }
            }
            Some(EnglishLabHit::RemoveDictionary(book_id)) => {
                let plan = self.shell.workspace().and_then(|ws| {
                    mochi_core::english_lab::Service::new(&ws.root)
                        .dictionary_removal_plan(&book_id)
                        .ok()
                });
                let Some(plan) = plan else {
                    self.show_global_notice("无法读取词典移除计划");
                    return;
                };
                self.dialog = Some(Dialog {
                    title: format!("移除《{}》", plan.name),
                    description: format!(
                        "将移除 {} 个未练习且仅属于这本词书的词；{} 个已有记录或属于其他词书的词会保留。",
                        plan.removable_words, plan.retained_words
                    ),
                    field: None,
                    error: String::new(),
                    note: Some("不会删除你的训练历史。".into()),
                    buttons: vec![
                        DialogButton { label: "取消".into(), kind: ButtonKind::Ghost, action: DialogAction::Dismiss },
                        DialogButton { label: "确认移除".into(), kind: ButtonKind::Danger, action: DialogAction::EnglishRemoveDictionary(book_id) },
                    ],
                    dismiss: DialogAction::Dismiss,
                    hover: None,
                });
                self.focus = Focus::Dialog;
            }
            Some(EnglishLabHit::Grade(word, grade)) => {
                if let Some(root) = self.shell.workspace().map(|ws| ws.root.clone()) {
                    match mochi_core::english_lab::Service::new(root).grade_fsrs(word, grade, 0) {
                        Ok(()) => self.show_global_notice("复习记录已保存，已按 FSRS 安排下次复习"),
                        Err(error) => self.show_global_notice(format!("保存复习失败：{error}")),
                    }
                }
                self.invalidate_main();
            }
            None => {}
        }
    }
}
