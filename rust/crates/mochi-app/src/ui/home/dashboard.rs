//! 生成首页快捷入口、日程、收件箱和最近内容。
use super::*;

impl Builder {
    pub(super) fn quick_actions(&mut self) {
        self.y += 20.0;
        let actions = [
            (Icon::FILE_PLUS2, "新建笔记", Action::NewNote),
            (Icon::SPARKLES, "AI 助手", Action::OpenAi),
            (Icon::CALENDAR_DAYS, "新建日记", Action::NewJournal),
            (Icon::UPLOAD, "导入文件", Action::ImportFile),
        ];
        let columns = self.columns(actions.len(), 120.0);
        let cell_w =
            ((self.content_width() - gap() * (columns - 1) as f32) / columns as f32).max(1.0);
        let action_count = actions.len();
        for (i, (icon, title, action)) in actions.iter().enumerate() {
            let col = i % columns;
            let row = i / columns;
            let x = self.content_left() + col as f32 * (cell_w + gap());
            let y = self.y + row as f32 * (action_tile_height() + gap());
            self.blocks.push(Block::ActionTile {
                rect: Rect::new(x, y, x + cell_w, y + action_tile_height()),
                icon: *icon,
                title: (*title).to_owned(),
                action: action.clone(),
            });
        }
        let rows = action_count.div_ceil(columns);
        self.y += rows as f32 * action_tile_height() + (rows.saturating_sub(1)) as f32 * gap();
        self.y += 16.0;
    }

    pub(super) fn documents(&mut self, title: &str, docs: &[DashboardDocument], favorites: bool) {
        let top = self.y + gap();
        let panel = self.panel_action(
            title,
            "查看全部",
            if favorites {
                Action::OpenFavoritesView
            } else {
                Action::OpenRecentView
            },
        );
        let shown = docs.len().min(5);
        if shown == 0 {
            self.blocks.push(Block::Empty {
                rect: Rect::new(
                    self.content_left(),
                    self.y,
                    self.content_right(),
                    self.y + 36.0,
                ),
                text: if favorites {
                    "还没有收藏的文档。"
                } else {
                    "还没有最近笔记，先新建一条小记。"
                }
                .to_owned(),
            });
            self.y += 36.0;
        } else {
            for (i, doc) in docs.iter().take(shown).enumerate() {
                let y = self.y + i as f32 * document_row_height();
                self.blocks.push(Block::DocumentRow {
                    rect: Rect::new(
                        self.content_left(),
                        y,
                        self.content_right(),
                        y + document_row_height(),
                    ),
                    title: doc.title.clone(),
                    subtitle: doc.subtitle.clone(),
                    mtime_ms: doc.mtime_ms,
                    favorite: doc.favorite,
                    action: if favorites {
                        Action::OpenFavorite(i)
                    } else {
                        Action::OpenRecent(i)
                    },
                });
            }
            self.y += shown as f32 * document_row_height();
        }
        self.close_panel(panel, top);
    }

    pub(super) fn inbox(&mut self, items: &[DashboardInboxItem]) {
        let top = self.y + gap();
        let panel = self.panel_actions(
            "收件箱",
            vec![("去整理", Action::OpenInbox), ("快速捕获", Action::Capture)],
        );
        let shown = items.len().min(3);
        if shown == 0 {
            self.blocks.push(Block::Empty {
                rect: Rect::new(
                    self.content_left(),
                    self.y,
                    self.content_right(),
                    self.y + 36.0,
                ),
                text: "收件箱已清空，想到什么先记下来。".to_owned(),
            });
            self.y += 36.0;
        } else {
            for item in items.iter().take(shown) {
                let row = Rect::new(
                    self.content_left(),
                    self.y,
                    self.content_right(),
                    self.y + inbox_row_height(),
                );
                self.blocks.push(Block::InboxRow {
                    rect: row,
                    content: item.content.clone(),
                    created_at_ms: item.created_at_ms,
                    action: Action::OpenInbox,
                });
                self.y += inbox_row_height();
            }
        }
        self.close_panel(panel, top);
    }

    pub(super) fn libraries(&mut self, libraries: &[DashboardLibrary]) {
        let top = self.y + gap();
        let panel = self.panel("知识库");
        let shown = libraries.len().min(6);
        if shown == 0 {
            self.blocks.push(Block::Empty {
                rect: Rect::new(
                    self.content_left(),
                    self.y,
                    self.content_right(),
                    self.y + 36.0,
                ),
                text: "还没有知识库，去导航栏新建一个。".to_owned(),
            });
            self.y += 36.0;
        } else {
            for (i, library) in libraries.iter().take(shown).enumerate() {
                let y = self.y + i as f32 * library_row_height();
                self.blocks.push(Block::LibraryRow {
                    rect: Rect::new(
                        self.content_left(),
                        y,
                        self.content_right(),
                        y + library_row_height(),
                    ),
                    name: library.name.clone(),
                    type_name: library.type_name.clone(),
                    action: Action::OpenLibrary(i),
                });
            }
            self.y += shown as f32 * library_row_height();
        }
        self.close_panel(panel, top);
    }

    pub(super) fn ai_sessions(&mut self, sessions: &[DashboardSession]) {
        let top = self.y + gap();
        let panel = self.panel_action("AI 助手", "展开", Action::OpenAi);
        if sessions.is_empty() {
            self.caption("写作、梳理思路，或一起探索一个问题。");
            self.prompt("帮我总结今天的笔记");
            self.prompt("生成一篇关于产品设计的文章");
            self.prompt("brainstorm 一个新功能的想法");
        } else {
            self.caption("继续最近的对话");
        }
        let shown = sessions.len().min(4);
        if shown == 0 {
            self.blocks.push(Block::Empty {
                rect: Rect::new(
                    self.content_left(),
                    self.y,
                    self.content_right(),
                    self.y + 36.0,
                ),
                text: "还没有 AI 会话，打开助手开始交流。".to_owned(),
            });
            self.y += 36.0;
        } else {
            for session in sessions.iter().take(shown) {
                let meta = if session.message_count > 0 {
                    format!(
                        "{} 条消息 · {}",
                        session.message_count,
                        relative_time(session.updated_at_ms, now_ms())
                    )
                } else {
                    relative_time(session.updated_at_ms, now_ms())
                };
                let row = Rect::new(
                    self.content_left(),
                    self.y,
                    self.content_right(),
                    self.y + session_row_height(),
                );
                self.blocks.push(Block::SessionRow {
                    rect: row,
                    title: session.title.clone(),
                    meta,
                    pinned: session.pinned,
                    action: Action::OpenAiSession(session.id.clone()),
                });
                self.y += session_row_height();
            }
        }
        self.close_panel(panel, top);
    }

    pub(super) fn schedule(
        &mut self,
        items: &[DashboardScheduleItem],
        conflict_count: usize,
        risk: &str,
    ) {
        let top = self.y + gap();
        let panel = self.panel_action("今日日程", "打开", Action::OpenSchedule);
        let shown = items.len().min(5);
        if shown == 0 {
            self.blocks.push(Block::Empty {
                rect: Rect::new(
                    self.content_left(),
                    self.y,
                    self.content_right(),
                    self.y + 36.0,
                ),
                text: "今天没有安排，可以自由支配。".to_owned(),
            });
            self.y += 36.0;
        } else {
            for item in items.iter().take(shown) {
                let row = Rect::new(
                    self.content_left(),
                    self.y,
                    self.content_right(),
                    self.y + schedule_row_height(),
                );
                self.blocks.push(Block::ScheduleRow {
                    rect: row,
                    time: item.time_label.clone(),
                    title: item.title.clone(),
                    status: item.status.clone(),
                    kind: item.kind.clone(),
                    priority: item.priority.clone(),
                    action: Action::OpenSchedule,
                });
                self.y += schedule_row_height();
            }
        }
        if conflict_count > 0 {
            self.caption(format!("{conflict_count} 个时间冲突 · 建议打开日程处理"));
        }
        if !risk.is_empty() {
            self.caption(format!("今日负载：{}", schedule_risk_label(risk)));
        }
        self.close_panel(panel, top);
    }

    pub(super) fn journal(&mut self, journal: &DashboardJournal) {
        let top = self.y + gap();
        let panel = self.panel_action("今日日记", "打开", Action::OpenJournal);
        self.blocks.push(Block::JournalPreview {
            rect: Rect::new(
                self.content_left(),
                self.y,
                self.content_right(),
                self.y + journal_height(),
            ),
            date: journal.date.clone(),
            excerpt: if journal.excerpt.is_empty() {
                "今天还没写日记，记录一个想法吧。".to_owned()
            } else {
                journal.excerpt.clone()
            },
            words: journal.words,
            exists: journal.exists,
            action: Action::OpenJournal,
        });
        self.y += journal_height();
        self.close_panel(panel, top);
    }

    pub(super) fn graph(&mut self, analytics: Option<&HomeAnalytics>, graph: &DashboardGraph) {
        let top = self.y + gap();
        let panel = self.panel("知识图谱预览");
        let mut nodes = graph.nodes.clone();
        let links = if graph.local_link_count > 0 {
            graph.local_link_count
        } else {
            analytics.map(|a| a.graph.local_link_count).unwrap_or(0)
        };
        if nodes.is_empty() {
            if let Some(a) = analytics {
                nodes = a
                    .graph
                    .central_nodes
                    .iter()
                    .map(|node| DashboardGraphNode {
                        path: PathBuf::from(&node.path),
                        title: node.title.clone(),
                        link_count: node.link_count,
                    })
                    .collect();
            }
        }
        self.caption(format!("{links} 条双链 · 被引用最多的笔记"));
        let shown = nodes.len().min(5);
        if shown == 0 {
            self.blocks.push(Block::Empty {
                rect: Rect::new(
                    self.content_left(),
                    self.y,
                    self.content_right(),
                    self.y + 52.0,
                ),
                text: "还没有互相链接的笔记，在正文里写 [[笔记名]] 建立双链。".to_owned(),
            });
            self.y += 52.0;
        } else {
            for (i, node) in nodes.iter().take(shown).enumerate() {
                let row = Rect::new(
                    self.content_left(),
                    self.y,
                    self.content_right(),
                    self.y + graph_row_height(),
                );
                self.blocks.push(Block::GraphNodeRow {
                    rect: row,
                    title: node.title.clone(),
                    link_count: node.link_count,
                    action: Action::OpenGraphNode(i),
                });
                self.y += graph_row_height();
            }
        }
        self.close_panel(panel, top);
    }

    pub(super) fn analytics_pending(&mut self) {
        let top = self.y + gap();
        let panel = self.panel("分析");
        self.blocks.push(Block::Empty {
            rect: Rect::new(
                self.content_left(),
                self.y,
                self.content_right(),
                self.y + 42.0,
            ),
            text: "统计生成中…".to_owned(),
        });
        self.y += 42.0;
        self.close_panel(panel, top);
    }

    pub(super) fn today_pending(&mut self) {
        let top = self.y + gap();
        let panel = self.panel("");
        self.blocks.push(Block::Empty {
            rect: Rect::new(
                self.content_left(),
                self.y,
                self.content_right(),
                self.y + 42.0,
            ),
            text: "今日数据统计中…".to_owned(),
        });
        self.y += 42.0;
        self.close_panel(panel, top);
    }

    pub(super) fn stats_pending(&mut self) {
        let top = self.y + gap();
        let panel = self.panel("数据统计");
        self.blocks.push(Block::Empty {
            rect: Rect::new(
                self.content_left(),
                self.y,
                self.content_right(),
                self.y + 42.0,
            ),
            text: "统计生成中…".to_owned(),
        });
        self.y += 42.0;
        self.close_panel(panel, top);
    }

    pub(super) fn persona_pending(&mut self) {
        let top = self.y + gap();
        let panel = self.panel("个人画像");
        self.blocks.push(Block::Empty {
            rect: Rect::new(
                self.content_left(),
                self.y,
                self.content_right(),
                self.y + 42.0,
            ),
            text: "画像生成中…".to_owned(),
        });
        self.y += 42.0;
        self.close_panel(panel, top);
    }
}
