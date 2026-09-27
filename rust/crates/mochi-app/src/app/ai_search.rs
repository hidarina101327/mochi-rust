//! 搜索 AI 会话消息，并管理匹配结果和消息定位。
use super::*;

impl App {
    pub(super) fn ai_reset_message_view(&mut self) {
        self.ai.panel.expanded_traces.clear();
        self.ai.panel.expanded_trace_steps.clear();
        self.ai.panel.streaming_reasoning_expanded = false;
        self.ai.panel.search_result_index = -1;
        self.ai.panel.search_query.clear();
        self.ai.panel.hover_message = None;
        self.ai.panel.hover_content = None;
        self.ai.panel.hover_hit = None;
        // 消息索引属于当前显示的对话。重建视图以显示另一个会话，
        // 或删除、重新生成消息后，
        // 都要清除拖选状态。
        self.ai.panel.clear_text_selection();
        self.ai.panel.navigation_scroll = 0.0;
        self.ai.panel.located = None;
    }
    pub(super) fn ai_scroll_to_message(&mut self, index: usize) {
        if let Some(message) = self.ai.layout.messages.get(index) {
            self.ai.panel.scroll = (message.top - 12.0)
                .max(0.0)
                .min(self.ai.layout.max_scroll());
            self.ai.panel.stick_to_bottom = false;
        }
    }
    pub(super) fn ai_toggle_search(&mut self) {
        if self.ai.panel.search_open {
            self.ai_close_search();
        } else {
            self.ai.panel.search_open = true;
            self.ai.panel.show_conversations = false;
            self.focus = Focus::AiMessageQuery;
            self.ai.panel.search_query.buffer.select_all();
        }
    }
    pub(super) fn ai_close_search(&mut self) {
        self.ai.panel.search_open = false;
        self.ai.panel.search_query.clear();
        self.ai.panel.search_result_index = -1;
        self.focus = Focus::AiInput;
    }
    fn ai_search_matches(&self) -> Vec<usize> {
        let query = self.ai.panel.search_query.text().trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        self.ai
            .panel
            .visible_messages()
            .iter()
            .enumerate()
            .filter_map(|(i, (_, text))| text.to_lowercase().contains(&query).then_some(i))
            .collect()
    }
    pub(super) fn ai_search_changed(&mut self) {
        let matches = self.ai_search_matches();
        self.ai.panel.search_result_index = if matches.is_empty() { -1 } else { 0 };
        if let Some(&index) = matches.first() {
            self.ai_scroll_to_message(index);
        }
    }
    pub(super) fn ai_search_next(&mut self, direction: i32) {
        let matches = self.ai_search_matches();
        if matches.is_empty() {
            self.ai.panel.search_result_index = -1;
            return;
        }
        let next = (self.ai.panel.search_result_index + direction).rem_euclid(matches.len() as i32);
        self.ai.panel.search_result_index = next;
        self.ai_scroll_to_message(matches[next as usize]);
    }
}
