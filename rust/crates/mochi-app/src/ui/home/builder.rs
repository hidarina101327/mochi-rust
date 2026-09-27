//! 组合首页的标题区、面板和左右两侧布局。
use super::*;

impl Builder {
    pub(super) fn new(area: Rect) -> Self {
        Builder {
            blocks: Vec::new(),
            area,
            y: area.top + panel_padding(),
            quiet: false,
        }
    }

    pub(super) fn left(&self) -> f32 {
        self.area.left + panel_padding()
    }

    pub(super) fn masthead(&mut self, analytics: Option<&HomeAnalytics>) {
        use chrono::Datelike;
        let now = chrono::Local::now();
        self.caption(format!(
            "工作台  /  {} · 星期{}",
            now.format("%m月%d日"),
            weekday_label(now.weekday().num_days_from_sunday() as usize)
        ));
        self.y += 8.0;
        self.greeting(greeting().split('，').next().unwrap_or("你好"));
        self.y += 4.0;
        self.caption(
            analytics
                .map(|a| today_narrative(&a.today.day, a.streak.current))
                .unwrap_or_else(|| "从一篇笔记开始，或继续上次的思考。".into()),
        );
        self.y += 16.0;
        // 分析数据加载时，先保留相同的四个指标位置。后台刷新
        // 不能让文档在用户点击时突然移出原位。
        let items = if let Some(a) = analytics {
            let d = &a.today.day;
            let prev = &a.today.previous;
            vec![
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
                    if d.focus_minutes > 0 {
                        "专注"
                    } else {
                        "在线"
                    }
                    .into(),
                    duration(if d.focus_minutes > 0 {
                        d.focus_minutes
                    } else {
                        d.active_minutes
                    }),
                    String::new(),
                    None,
                ),
            ]
        } else {
            ["今日字数", "编辑笔记", "AI 对话", "在线"]
                .map(|label| (label.into(), "—".into(), String::new(), None))
                .into()
        };
        self.tiles(items);
    }

    /// 开一个面板：先占位，内容画完再回填高度。
    ///
    /// 面板底板必须**先**进列表（它在内容底下），但高度要等内容排完才知道。
    /// 记下索引回填，比先量内容再画底板简单得多。
    pub(super) fn panel(&mut self, title: &str) -> usize {
        self.y += gap();
        let index = self.blocks.len();
        self.blocks.push(Block::Panel {
            rect: Rect::ZERO,
            quiet: self.quiet,
        });
        self.y += panel_padding();
        if !title.is_empty() {
            let h = TextStyle::Title.line_height();
            self.blocks.push(Block::SectionTitle {
                rect: Rect::new(
                    self.left() + panel_padding(),
                    self.y,
                    self.right_inner(),
                    self.y + h,
                ),
                text: title.to_owned(),
            });
            self.y += h + 8.0;
        }
        index
    }

    pub(super) fn right_inner(&self) -> f32 {
        self.area.right - panel_padding() * 2.0
    }

    pub(super) fn close_panel(&mut self, index: usize, top_of_panel: f32) {
        self.y += panel_padding();
        if let Some(Block::Panel { rect, .. }) = self.blocks.get_mut(index) {
            *rect = Rect::new(
                self.area.left + panel_padding(),
                top_of_panel,
                self.area.right - panel_padding(),
                self.y,
            );
        }
    }

    /// 一行统计小格。
    pub(super) fn tiles(&mut self, items: Vec<(String, String, String, Option<i64>)>) {
        if items.is_empty() {
            return;
        }
        let inner_left = self.left() + panel_padding();
        let inner_width = (self.right_inner() - inner_left).max(80.0);
        // 首页也会在窄侧栏/分屏里显示。四个统计格硬塞进一行会让
        // “431.9 MB”这类真实值穿出卡片；先保证每格能承载一个数值。
        let preferred = if inner_width < 240.0 {
            1
        } else if inner_width < 520.0 {
            2
        } else {
            TILES_PER_ROW
        };
        let per_row = preferred.min(items.len().max(1));
        let cell_w = (inner_width - gap() * (per_row - 1) as f32) / per_row as f32;

        let count = items.len();
        for (i, (label, value, unit, delta)) in items.into_iter().enumerate() {
            let col = i % per_row;
            let row = i / per_row;
            let x = inner_left + col as f32 * (cell_w + gap());
            let y = self.y + row as f32 * (tile_height() + gap());
            self.blocks.push(Block::Tile {
                rect: Rect::new(x, y, x + cell_w, y + tile_height()),
                label,
                value,
                unit,
                delta,
            });
        }
        // 按**实际行数**推进游标。现在每处都恰好传 4 个（正好一行），
        // 但按一行写死的话，将来加第五个统计格就会和下一块叠在一起。
        let rows = count.div_ceil(per_row);
        self.y += rows as f32 * tile_height() + (rows.saturating_sub(1)) as f32 * gap();
    }

    pub(super) fn caption(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        let h = TextStyle::Caption.line_height();
        self.blocks.push(Block::Caption {
            rect: Rect::new(
                self.left() + panel_padding(),
                self.y,
                self.right_inner(),
                self.y + h,
            ),
            text,
        });
        self.y += h + 4.0;
    }

    pub(super) fn greeting(&mut self, text: impl Into<String>) {
        let h = TextStyle::Display.line_height();
        self.blocks.push(Block::Greeting {
            rect: Rect::new(
                self.left() + panel_padding(),
                self.y,
                self.right_inner(),
                self.y + h,
            ),
            text: text.into(),
        });
        self.y += h + 2.0;
    }

    pub(super) fn detail(&mut self, label: impl Into<String>, value: impl Into<String>) {
        let h = TextStyle::Label.line_height() + 4.0;
        self.blocks.push(Block::DetailRow {
            rect: Rect::new(
                self.content_left(),
                self.y,
                self.content_right(),
                self.y + h,
            ),
            label: label.into(),
            value: value.into(),
        });
        self.y += h;
    }

    pub(super) fn prompt(&mut self, text: impl Into<String>) {
        let h = 34.0;
        self.blocks.push(Block::PromptRow {
            rect: Rect::new(
                self.content_left(),
                self.y,
                self.content_right(),
                self.y + h,
            ),
            text: text.into(),
            action: Action::OpenAi,
        });
        self.y += h + 6.0;
    }

    pub(super) fn bars(&mut self, bars: Vec<(String, f32, bool)>) {
        if bars.is_empty() {
            return;
        }
        let rect = Rect::new(
            self.left() + panel_padding(),
            self.y,
            self.right_inner(),
            self.y + chart_height(),
        );
        self.blocks.push(Block::Bars { rect, bars });
        self.y += chart_height() + TextStyle::Caption.line_height();
    }

    pub(super) fn storage_bar(&mut self, file_types: &[FileTypeStat], total: u64) {
        if total == 0 || file_types.is_empty() {
            return;
        }
        let mut segments: Vec<(String, u64)> = file_types
            .iter()
            .filter(|item| item.size > 0)
            .map(|item| (item.extension.clone(), item.size))
            .collect();
        segments.sort_by(|a, b| b.1.cmp(&a.1));
        segments.truncate(6);
        if segments.is_empty() {
            return;
        }
        let h = 14.0;
        self.blocks.push(Block::StorageBar {
            rect: Rect::new(
                self.content_left(),
                self.y,
                self.content_right(),
                self.y + h,
            ),
            segments,
            total,
        });
        self.y += h + 8.0;
    }

    /// 面板标题下的一行文字入口。单独占一行可以避免窄窗中标题和按钮互相
    /// 覆盖，同时保留和 Electron 一样的「查看全部 / 打开」语义。
    pub(super) fn panel_action(&mut self, title: &str, label: &str, action: Action) -> usize {
        self.panel_actions(title, vec![(label, action)])
    }

    pub(super) fn panel_actions(&mut self, title: &str, actions: Vec<(&str, Action)>) -> usize {
        let panel = self.panel(title);
        let h = 28.0;
        let widths: Vec<f32> = actions
            .iter()
            .map(|(label, _)| text::measure(label, TextStyle::Caption) + 16.0)
            .collect();
        let total = widths.iter().sum::<f32>() + gap() * widths.len().saturating_sub(1) as f32;
        let title_width = text::measure(title, TextStyle::Title);
        let shares_title = title_width + total + 24.0 <= self.content_width();
        let title_y = self.y - TextStyle::Title.line_height() - 8.0;
        if total <= self.content_width() {
            if shares_title {
                if let Some(Block::SectionTitle { rect, .. }) = self.blocks.last_mut() {
                    rect.right -= total + 16.0;
                }
            }
            let y = if shares_title { title_y - 3.0 } else { self.y };
            let mut right = self.right_inner();
            for ((label, action), width) in actions.iter().zip(widths.iter()).rev() {
                let left = (right - *width).max(self.left() + panel_padding());
                self.blocks.push(Block::ActionLink {
                    rect: Rect::new(left, y, right, y + h),
                    text: (*label).to_owned(),
                    action: action.clone(),
                });
                right = left - gap();
            }
            if !shares_title {
                self.y += h + 4.0;
            }
        } else {
            // 极窄窗口时逐行放置，保证两个入口仍然各自有完整命中区。
            for ((label, action), width) in actions.iter().zip(widths.iter()) {
                let right = self.right_inner();
                let left = (right - *width).max(self.left() + panel_padding());
                self.blocks.push(Block::ActionLink {
                    rect: Rect::new(left, self.y, right, self.y + h),
                    text: (*label).to_owned(),
                    action: action.clone(),
                });
                self.y += h + 2.0;
            }
            self.y += 2.0;
        }
        panel
    }

    pub(super) fn content_left(&self) -> f32 {
        self.left() + panel_padding()
    }

    pub(super) fn content_right(&self) -> f32 {
        self.right_inner()
    }

    pub(super) fn content_width(&self) -> f32 {
        (self.content_right() - self.content_left()).max(1.0)
    }

    /// 根据可用宽度决定网格列数。每个入口卡片保留至少 132px，
    /// 再窄就自然退化为单列，避免内容跑出窗口。
    pub(super) fn columns(&self, max: usize, minimum: f32) -> usize {
        let count = ((self.content_width() + gap()) / (minimum + gap())).floor() as usize;
        count.clamp(1, max)
    }
}
