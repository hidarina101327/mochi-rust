//! 整理首页所需的活动统计、趋势和按日汇总数据。
use super::*;

impl Builder {
    #[cfg(test)]
    pub(super) fn analytics(&mut self, a: &HomeAnalytics) {
        self.today(a);
        self.analytics_rest(a);
    }

    pub(super) fn analytics_rest(&mut self, a: &HomeAnalytics) {
        self.stats(a);
        self.writing(a);
        self.notes(a);
        self.contributions(a);
        self.persona(a);
    }

    /// 左列的分析区。它保留 Electron 首页的五类真实趋势：写作、笔记、
    /// 作息、AI 消息和模型用量，以及年度活动。存量统计留在右列，避免把
    /// 「今天在做什么」和「工作区有多少东西」混成一张卡。
    pub(super) fn analytics_main(&mut self, analytics: Option<&HomeAnalytics>) {
        let Some(a) = analytics else {
            self.analytics_pending();
            return;
        };
        self.writing(a);
        self.notes(a);
        self.hourly(a);
        self.ai_trends(a);
        self.token_trends(a);
        self.contributions(a);
    }

    pub(super) fn today(&mut self, a: &HomeAnalytics) {
        let top = self.y + gap();
        let panel = self.panel("");
        let d = &a.today.day;
        let prev = &a.today.previous;

        self.greeting(greeting());
        self.caption(today_narrative(d, a.streak.current));

        let h = TextStyle::Heading2.line_height();
        self.blocks.push(Block::Headline {
            rect: Rect::new(
                self.left() + panel_padding(),
                self.y,
                self.right_inner(),
                self.y + h,
            ),
            text: headline(d.net_words, d.notes_edited),
        });
        self.y += h + 4.0;

        if a.streak.current > 0 {
            let w = 96.0;
            let x = self.left() + panel_padding();
            self.blocks.push(Block::Badge {
                rect: Rect::new(x, self.y, x + w, self.y + 22.0),
                text: format!("连续 {} 天", a.streak.current),
                accent: true,
            });
            self.y += 22.0 + 6.0;
        }

        let (focus_label, focus_value, focus_unit, focus_delta) = if d.focus_minutes > 0 {
            (
                "专注",
                d.focus_minutes.to_string(),
                "分钟".to_owned(),
                d.focus_minutes - prev.focus_minutes,
            )
        } else {
            (
                "在线",
                duration(d.active_minutes),
                String::new(),
                d.active_minutes - prev.active_minutes,
            )
        };
        self.tiles(vec![
            (
                "今日字数".into(),
                signed(d.net_words),
                "字".into(),
                Some(d.net_words - prev.net_words),
            ),
            (
                "编辑笔记".into(),
                d.notes_edited.to_string(),
                "篇".into(),
                Some(d.notes_edited as i64 - prev.notes_edited as i64),
            ),
            (
                "AI 对话".into(),
                d.ai_messages.to_string(),
                "条".into(),
                Some(d.ai_messages - prev.ai_messages),
            ),
            (
                focus_label.into(),
                focus_value,
                focus_unit,
                Some(focus_delta),
            ),
        ]);

        let mut parts: Vec<String> = Vec::new();
        // 在线/专注时长已经在第四张统计卡中展示（并且带有与昨日的趋势）。
        // 再把它塞进整行摘要会从面板最左边开始画，视觉上像最后一张卡片的
        // 内容错位到了第一张下面。
        if d.notes_created > 0 {
            parts.push(format!("新建 {} 篇", d.notes_created));
        }
        if d.tasks_completed > 0 {
            parts.push(format!("完成 {} 个待办", d.tasks_completed));
        }
        if d.searches > 0 {
            parts.push(format!("搜索 {} 次", d.searches));
        }
        self.caption(parts.join(" · "));
        if let Some(note) = a.today.top_notes.first() {
            self.caption(format!(
                "今天投入最多：{} · 保存 {} 次 · 净增 {} 字",
                note.title,
                note.save_count,
                signed(note.net_words)
            ));
        }
        self.close_panel(panel, top);
    }

    pub(super) fn stats(&mut self, a: &HomeAnalytics) {
        let top = self.y + gap();
        let panel = self.panel("数据统计");
        let f = &a.files;
        self.tiles(vec![
            ("笔记".into(), f.total_notes.to_string(), "篇".into(), None),
            (
                "总字数".into(),
                compact(f.total_words as i64),
                "字".into(),
                None,
            ),
            ("文件".into(), f.total_files.to_string(), "个".into(), None),
            ("占用".into(), bytes(f.total_size), String::new(), None),
        ]);

        let g = &a.graph;
        self.caption(format!(
            "双链 {} 条 · 孤立笔记 {} 篇 · 未打标签 {:.0}%",
            g.local_link_count,
            g.isolated_note_count,
            g.untagged_note_ratio * 100.0
        ));

        self.storage_bar(&f.file_types, f.total_size);

        let mut types: Vec<&FileTypeStat> = f.file_types.iter().collect();
        types.sort_by(|a, b| b.count.cmp(&a.count));
        let top_types: Vec<String> = types
            .iter()
            .take(6)
            .map(|t| format!("{} {}", t.extension, t.count))
            .collect();
        self.caption(top_types.join("   "));
        self.caption(format!(
            "AI {} 个会话 · {} 条消息 · {} tokens",
            a.ai.total_sessions,
            a.ai.total_messages,
            compact(a.ai.total_tokens.max(a.ai.estimated_tokens))
        ));
        self.close_panel(panel, top);
    }

    pub(super) fn hourly(&mut self, a: &HomeAnalytics) {
        let top = self.y + gap();
        let panel = self.panel("活跃时段");
        let cells = hourly_cells(&a.activity.hourly);
        let height = 7.0 * 16.0 + 22.0;
        self.blocks.push(Block::HourlyHeatmap {
            rect: Rect::new(
                self.content_left(),
                self.y,
                self.content_right(),
                self.y + height,
            ),
            cells,
        });
        self.y += height;
        let peak = a
            .activity
            .hourly
            .iter()
            .max_by_key(|point| point.weight)
            .filter(|point| point.weight > 0);
        if let Some(point) = peak {
            self.caption(format!(
                "活跃高峰：周{} {}:00 · {} 次活动",
                weekday_label(point.day as usize),
                point.hour,
                point.count
            ));
        } else {
            self.caption("还没有足够的活动记录来判断作息。");
        }
        self.close_panel(panel, top);
    }

    pub(super) fn ai_trends(&mut self, a: &HomeAnalytics) {
        let top = self.y + gap();
        let panel = self.panel("AI 对话");
        let points: Vec<&AiTrendPoint> = a.ai.day.iter().rev().take(30).rev().collect();
        let peak = points
            .iter()
            .map(|point| point.conversation_count.max(0))
            .max()
            .unwrap_or(0)
            .max(1);
        let last = points.len().saturating_sub(1);
        let bars = points
            .iter()
            .enumerate()
            .map(|(i, point)| {
                (
                    point.label.clone(),
                    point.conversation_count.max(0) as f32 / peak as f32,
                    i == last,
                )
            })
            .collect();
        self.bars(bars);
        self.caption(format!(
            "近 30 天 {} 次对话 · 用户 {} 条 · AI {} 条",
            points
                .iter()
                .map(|point| point.conversation_count.max(0))
                .sum::<i64>(),
            points
                .iter()
                .map(|point| point.user_message_count.max(0))
                .sum::<i64>(),
            points
                .iter()
                .map(|point| point.assistant_message_count.max(0))
                .sum::<i64>(),
        ));
        self.close_panel(panel, top);
    }

    pub(super) fn token_trends(&mut self, a: &HomeAnalytics) {
        let top = self.y + gap();
        let panel = self.panel("AI 用量");
        let points: Vec<&TokenTrendPoint> = a.ai.tokens.day.iter().rev().take(30).rev().collect();
        let peak = points
            .iter()
            .map(|point| point.total_tokens.max(point.estimated_tokens).max(0))
            .max()
            .unwrap_or(0)
            .max(1);
        let last = points.len().saturating_sub(1);
        let bars = points
            .iter()
            .enumerate()
            .map(|(i, point)| {
                (
                    point.label.clone(),
                    point.total_tokens.max(point.estimated_tokens).max(0) as f32 / peak as f32,
                    i == last,
                )
            })
            .collect();
        self.bars(bars);
        self.caption(format!(
            "近 30 天 {} tokens · 输入 {} · 输出 {}",
            compact(
                points
                    .iter()
                    .map(|point| point.total_tokens.max(point.estimated_tokens))
                    .sum()
            ),
            compact(points.iter().map(|point| point.prompt_tokens.max(0)).sum()),
            compact(
                points
                    .iter()
                    .map(|point| point.completion_tokens.max(0))
                    .sum()
            ),
        ));
        self.close_panel(panel, top);
    }

    pub(super) fn writing(&mut self, a: &HomeAnalytics) {
        let top = self.y + gap();
        let panel = self.panel("写作产出");
        let points: Vec<&WritingTrendPoint> = a.writing.day.iter().rev().take(30).rev().collect();
        let peak = points
            .iter()
            .map(|p| p.words_added.max(0))
            .max()
            .unwrap_or(0)
            .max(1);
        let last = points.len().saturating_sub(1);
        let bars = points
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    p.label.clone(),
                    p.words_added.max(0) as f32 / peak as f32,
                    i == last,
                )
            })
            .collect();
        self.bars(bars);
        self.caption(format!(
            "近 30 天累计 {} 字 · 峰值 {} 字/天",
            compact(points.iter().map(|p| p.words_added.max(0)).sum()),
            compact(peak)
        ));
        self.close_panel(panel, top);
    }

    pub(super) fn notes(&mut self, a: &HomeAnalytics) {
        let top = self.y + gap();
        let panel = self.panel("笔记增长");
        let points: Vec<&TrendPoint> = a.note_trends.day.iter().rev().take(30).rev().collect();
        // `total_count` 是历史累计存量，用它做柱高会把 30 天画成一堵
        // 几乎等高的墙。增长图应表达每天真正发生的新建与更新。
        let activity = |point: &TrendPoint| point.created_count + point.updated_count;
        let peak = points
            .iter()
            .map(|p| activity(p) as i64)
            .max()
            .unwrap_or(0)
            .max(1);
        let last = points.len().saturating_sub(1);
        let bars = points
            .iter()
            .enumerate()
            .map(|(i, p)| (p.label.clone(), activity(p) as f32 / peak as f32, i == last))
            .collect();
        self.bars(bars);
        self.caption(format!(
            "近 30 天新建 {} 篇 · 更新 {} 次",
            points.iter().map(|p| p.created_count).sum::<usize>(),
            points.iter().map(|p| p.updated_count).sum::<usize>()
        ));
        self.close_panel(panel, top);
    }

    pub(super) fn contributions(&mut self, a: &HomeAnalytics) {
        let top = self.y + gap();
        let panel = self.panel("年度活跃图");
        let cells = heat_cells(&a.activity.contributions);
        let height = 7.0 * (heat_cell() + heat_gap());
        self.blocks.push(Block::Heatmap {
            rect: Rect::new(
                self.content_left(),
                self.y,
                self.content_right(),
                self.y + height,
            ),
            cells,
        });
        self.y += height + 6.0;

        let active = a
            .activity
            .contributions
            .iter()
            .filter(|p| p.count > 0)
            .count();
        self.caption(format!(
            "{} 天有记录 · 最长连续 {} 天 · 本周活跃 {} 天",
            active, a.streak.longest, a.streak.active_days_this_week
        ));
        self.close_panel(panel, top);
    }

    pub(super) fn persona(&mut self, a: &HomeAnalytics) {
        let top = self.y + gap();
        let panel = self.panel("个人画像");
        let p = &a.persona;

        let mut badges: Vec<String> = Vec::new();
        if let Some(c) = &p.chronotype {
            badges.push(chronotype_label(c).to_owned());
        }
        if let Some(h) = p.peak_hour {
            badges.push(format!("高峰 {h} 点"));
        }
        if let Some(d) = p.busiest_weekday {
            badges.push(format!("最忙 周{}", weekday_label(d)));
        }
        let mut x = self.left() + panel_padding();
        let badge_top = self.y;
        for text in badges {
            let w = text::measure(&text, TextStyle::Caption) + 20.0;
            if x + w > self.right_inner() {
                break;
            }
            self.blocks.push(Block::Badge {
                rect: Rect::new(x, badge_top, x + w, badge_top + 22.0),
                text,
                accent: false,
            });
            x += w + 8.0;
        }
        self.y += 22.0 + 8.0;

        if let Some(hour) = p.peak_hour {
            self.detail(
                "黄金时段",
                format!("{:02}:00 · 占活跃 {:.0}%", hour, p.peak_hour_share * 100.0),
            );
        }
        if let Some(day) = p.busiest_weekday {
            self.detail("最常活跃", format!("周{}", weekday_label(day)));
        }
        if p.focus_sessions > 0 || p.focus_minutes > 0 {
            self.detail(
                "专注记录",
                format!("{} 次 · {}", p.focus_sessions, duration(p.focus_minutes)),
            );
        }
        self.caption(format!(
            "日均在线 {} · 活跃日均 {} 字 · AI 协作占比 {:.0}%",
            duration(p.average_active_minutes),
            compact(p.average_words_per_active_day),
            p.ai_collaboration_ratio * 100.0
        ));
        if !p.top_areas.is_empty() {
            // `WorkAreaStat` 只给绝对数，占比自己算——分母用**所有**区域的事件数，
            // 不是 top_areas 的和，否则"前四名占 100%"这种废话就出来了
            let total: i64 = p
                .top_areas
                .iter()
                .map(|a| a.event_count)
                .sum::<i64>()
                .max(1);
            let areas: Vec<String> = p
                .top_areas
                .iter()
                .take(4)
                .map(|a| {
                    format!(
                        "{} {:.0}%",
                        a.name,
                        a.event_count as f64 * 100.0 / total as f64
                    )
                })
                .collect();
            self.caption(format!("主要投入：{}", areas.join(" · ")));
        }
        self.close_panel(panel, top);
    }
}
