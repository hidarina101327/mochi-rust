//! 管理工作区搜索请求、结果更新和结果打开。
use super::*;

impl App {
    pub fn open_search(&mut self) {
        self.search_job.invalidate();
        let s = SearchState {
            history: self.search_history.clone(),
            ..Default::default()
        };
        self.search = Some(s);
        self.menu = None;
        self.focus = Focus::Search;
    }

    pub(super) fn close_search(&mut self) {
        if let Some(s) = self.search.take() {
            self.search_history = s.history;
            self.settings.set(
                "search.history",
                &serde_json::to_string(&self.search_history).unwrap_or_default(),
            );
        }
        self.search_job.invalidate();
        self.search_job.timer_pending = false;
        if self.focus == Focus::Search {
            self.focus = Focus::Main;
        }
    }

    /// 查询词或选项变了：清空旧结果、重置展开/选中，并请求去抖计时器。
    pub(super) fn search_changed(&mut self) {
        // 立即作废旧结果，不要等到防抖计时结束：否则旧的
        // 响应可能会在新查询的计时器尚未触发时覆盖界面。
        self.search_job.invalidate();
        let Some(s) = self.search.as_mut() else {
            return;
        };
        s.on_query_changed();
        if s.trimmed_query().is_empty() {
            s.clear_results();
            self.search_job.timer_pending = false;
            return;
        }
        s.searching = true;
        self.search_job.timer_pending = true;
    }

    /// 后台搜完了。代号对不上的是过期结果，丢掉。
    pub fn take_search_result(&mut self) {
        let Some(rx) = &self.search_job.rx else {
            return;
        };
        let Ok((generation, result)) = rx.try_recv() else {
            return;
        };
        if generation != self.search_job.generation {
            return;
        }
        if let Some(s) = self.search.as_mut() {
            // 期间用户又改了词（计时器还没到点）就别把旧词的结果贴上去
            if s.searching || s.result.is_none() {
                s.apply_result(result);
            }
        }
    }

    pub(super) fn on_search_click(&mut self, x: f32, y: f32) {
        let hit = self.search_layout.hit(x, y);
        let Some(s) = self.search.as_mut() else {
            return;
        };
        match hit {
            SearchHit::Backdrop | SearchHit::Close => self.close_search(),
            SearchHit::Input => {
                if let Some(r) = self.search_layout.rect_of(SearchHit::Input) {
                    s.query.click(x - r.left, false);
                }
            }
            SearchHit::CaseSensitive => {
                s.options.case_sensitive = !s.options.case_sensitive;
                self.search_changed();
            }
            SearchHit::WholeWord => {
                s.options.whole_word = !s.options.whole_word;
                self.search_changed();
            }
            SearchHit::Regex => {
                s.options.use_regex = !s.options.use_regex;
                self.search_changed();
            }
            SearchHit::Phrase => {
                s.options.match_phrase = !s.options.match_phrase;
                self.search_changed();
            }
            SearchHit::Filters => s.filters_open = !s.filters_open,
            SearchHit::Preset(None) => {
                s.options.extensions.clear();
                self.search_changed();
            }
            SearchHit::Preset(Some(i)) => {
                s.toggle_preset(search::FILE_TYPE_PRESETS[i].2);
                self.search_changed();
            }
            SearchHit::Collapse(i) => {
                if let Some(RowKind::File(g)) = s.rows().get(i).copied() {
                    if let Some(path) = s
                        .result
                        .as_ref()
                        .and_then(|r| r.groups.get(g))
                        .map(|g| g.path.clone())
                    {
                        if !s.collapsed.remove(&path) {
                            s.collapsed.insert(path);
                        }
                    }
                }
            }
            SearchHit::Row(i) => {
                s.selected = i;
                self.open_selected_search_row();
            }
            SearchHit::History(i) => {
                if let Some(q) = s.history.get(i).cloned() {
                    s.query.set_text(&q);
                    self.search_changed();
                }
            }
            SearchHit::ClearHistory => s.history.clear(),
            SearchHit::Inside => {}
        }
    }

    pub(super) fn open_selected_search_row(&mut self) {
        let Some(s) = self.search.as_mut() else {
            return;
        };
        let rows = s.rows();
        let Some(row) = rows.get(s.selected).copied() else {
            return;
        };
        let Some(path) = s.result_at(row).map(|r| PathBuf::from(&r.path)) else {
            return;
        };
        let base_location = s
            .result_at(row)
            .and_then(|result| result.base_location.clone());
        let q = s.trimmed_query();
        s.add_to_history(&q);
        self.close_search();
        if self.open_file_from_ui(&path) {
            self.state.view = WorkspaceView::Editor;
            if let Some(location) = base_location {
                self.reveal_base(location);
            }
            self.invalidate_main();
            self.sync_state();
        }
    }
}
