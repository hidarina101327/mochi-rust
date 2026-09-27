//! 分派 AI 助手面板中的点击操作。
use super::*;

impl App {
    pub(super) fn on_assistant_click(&mut self, hwnd: HWND, x: f32, y: f32) {
        if self.ai_begin_text_selection(x, y) {
            self.drag = Some(Drag {
                target: DragTarget::AiTextSelect,
                grab_offset: 0.0,
            });
            self.focus = Focus::Main;
            return;
        }
        let Some(mut hit) = self.ai.layout.hit(x, y) else {
            return;
        };
        if self.ai.panel.is_streaming()
            && matches!(
                hit,
                assistant::Hit::NewSession
                    | assistant::Hit::PopoverNew
                    | assistant::Hit::Session(_)
                    | assistant::Hit::SessionDelete(_)
                    | assistant::Hit::Clear
                    | assistant::Hit::ApproveMode
                    | assistant::Hit::AutomaticMode
            )
        {
            return;
        }
        if hit == assistant::Hit::Messages && self.ai_scroll_click(x, y) {
            return;
        }
        if hit == assistant::Hit::Messages {
            if let Some(action) = self.ai.layout.message_hit(self.ai.panel.scroll, x, y) {
                hit = action;
            }
        }
        if hit == assistant::Hit::Messages {
            if let Some((_, target)) = self.ai.layout.link_at(self.ai.panel.scroll, x, y) {
                self.open_link(&target);
                return;
            }
        }
        if hit == assistant::Hit::Messages {
            if let Some((message, action)) =
                self.ai.layout.content_at(self.ai.panel.scroll, x, y, false)
            {
                hit = assistant::Hit::Content(message, action);
            }
        }
        // 点弹层外面就把弹层收起来
        if self.ai.panel.show_conversations
            && !matches!(
                hit,
                assistant::Hit::Session(_)
                    | assistant::Hit::SessionDelete(_)
                    | assistant::Hit::PopoverNew
                    | assistant::Hit::PopoverInside
                    | assistant::Hit::Conversations
            )
        {
            self.ai.panel.show_conversations = false;
        }
        match hit {
            assistant::Hit::RemoveImage(index) => self.remove_pending_image(index),
            assistant::Hit::RemoveFile(index) => {
                if index < self.ai.panel.pending_files.len() {
                    self.ai.panel.pending_files.remove(index);
                }
            }
            assistant::Hit::FilesBack | assistant::Hit::FilesForward => {
                let step = self
                    .ai
                    .layout
                    .files_viewport
                    .map(|r| r.width() * 0.8)
                    .unwrap_or(160.0);
                let delta = if hit == assistant::Hit::FilesBack {
                    -step
                } else {
                    step
                };
                self.ai.panel.files_scroll = (self
                    .ai
                    .panel
                    .files_scroll
                    .clamp(0.0, self.ai.layout.files_max_scroll)
                    + delta)
                    .clamp(0.0, self.ai.layout.files_max_scroll);
            }
            assistant::Hit::OpenFile(index) => {
                if let Some(path) = self.ai.panel.pending_files.get(index).cloned() {
                    self.open_file_from_ui(&path);
                    self.sync_state();
                }
            }
            assistant::Hit::AttachFiles => {
                self.open_object_picker(object_picker_host::Purpose::Attach)
            }
            assistant::Hit::Content(message, action) => {
                let owner = HWND(self.hwnd_raw as *mut _);
                self.ai_content_copy_with(message, action, |p| {
                    platform::copy_payload(owner, &p.text, p.html.as_deref())
                });
            }
            assistant::Hit::Message(i, action) => {
                if let Some((sid, mid)) = self
                    .ai
                    .layout
                    .messages
                    .get(i)
                    .and_then(|m| m.action_target.clone())
                {
                    self.ai_message_action(&sid, &mid, action, x, y);
                }
            }
            assistant::Hit::PendingEditApprove(i) => self.ai_pending_edit_action(i, true),
            assistant::Hit::PendingEditReject(i) => self.ai_pending_edit_action(i, false),
            assistant::Hit::PendingSelectionRemove(i) => {
                Self::ai_remove_pending_selection(&mut self.ai.panel.pending_selections, i);
            }
            assistant::Hit::PendingSelectionLocate(i) => {
                if let Some(value) = self.ai.panel.pending_selections.get(i).cloned() {
                    self.ai_locate_selection_value(&value);
                }
            }
            assistant::Hit::SelectionLocate(message, selection) => {
                if let Some(value) = self
                    .ai
                    .layout
                    .messages
                    .get(message)
                    .and_then(|m| m.selection_values.get(selection))
                    .cloned()
                {
                    self.ai_locate_selection_value(&value);
                }
            }
            assistant::Hit::PendingCardReview(i) => {
                self.ai_review_pending_message(i);
            }
            assistant::Hit::TraceToggle(i) => {
                if !self.ai.panel.expanded_traces.remove(&i) {
                    self.ai.panel.expanded_traces.insert(i);
                }
                self.ai.panel.stick_to_bottom = false;
            }
            assistant::Hit::TraceStep(i, step) => {
                if !self.ai.panel.expanded_trace_steps.remove(&(i, step)) {
                    self.ai.panel.expanded_trace_steps.insert((i, step));
                }
                self.ai.panel.stick_to_bottom = false;
            }
            assistant::Hit::StreamingReasoningToggle => {
                self.ai.panel.streaming_reasoning_expanded =
                    !self.ai.panel.streaming_reasoning_expanded
            }
            assistant::Hit::ScrollToBottom => {
                self.ai.panel.stick_to_bottom = true;
                self.ai.panel.scroll = self.ai.layout.max_scroll();
            }
            assistant::Hit::NavigateMessage(i) => self.ai_scroll_to_message(i),
            assistant::Hit::PreviousQuestion => {
                if let Some(i) = self
                    .ai
                    .layout
                    .messages
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, m)| m.role == "user" && m.top < self.ai.panel.scroll - 4.0)
                    .map(|(i, _)| i)
                {
                    self.ai_scroll_to_message(i);
                } else {
                    self.ai_scroll_to_message(0);
                }
            }
            assistant::Hit::SearchToggle => self.ai_toggle_search(),
            assistant::Hit::SearchClose => self.ai_close_search(),
            assistant::Hit::SearchNext => self.ai_search_next(1),
            assistant::Hit::SearchPrevious => self.ai_search_next(-1),
            assistant::Hit::SearchInput => {
                self.focus = Focus::AiMessageQuery;
                if let Some(r) = self.ai.layout.rect_of(hit) {
                    self.ai.panel.search_query.click(x - r.left - 32.0, false);
                }
            }
            assistant::Hit::AgentPicker => {
                if let Some(r) = self.ai.layout.rect_of(hit) {
                    self.ai_pick_agent(r);
                }
            }
            assistant::Hit::MountSession => {
                if let Some(r) = self.ai.layout.rect_of(hit) {
                    self.ai_pick_mount(r);
                }
            }
            assistant::Hit::ApproveMode => self.ai_set_apply_mode(false),
            assistant::Hit::AutomaticMode => self.ai_set_apply_mode(true),
            assistant::Hit::Conversations => {
                self.ai.panel.show_conversations = !self.ai.panel.show_conversations;
                if self.ai.panel.show_conversations {
                    self.ai.panel.conversations_scroll = 0.0;
                }
            }
            assistant::Hit::NewSession | assistant::Hit::PopoverNew => self.ai_new_session(),
            assistant::Hit::Clear => self.ai_ask_clear_session(),
            assistant::Hit::Settings | assistant::Hit::OpenSettings => self.open_settings("ai"),
            assistant::Hit::AgentConfig => {
                self.state.view = WorkspaceView::AgentConfig;
                self.focus = Focus::Main;
                self.invalidate_main();
            }
            assistant::Hit::Float => self.toggle_ai_float(),
            assistant::Hit::Close => {
                if self.ai.float.take().is_some() {
                    self.invalidate_main();
                    return;
                }
                if self.state.view == WorkspaceView::MochiAi {
                    self.state.view = WorkspaceView::Home;
                }
                self.state.ai_panel_open = false;
                self.invalidate_main();
            }
            assistant::Hit::Input => {
                self.ai_clear_text_selection();
                self.focus = Focus::AiInput;
                if let Some(r) = self.ai.layout.rect_of(assistant::Hit::Input) {
                    self.ai.panel.input.multiline_click(r, x, y);
                }
            }
            assistant::Hit::Send => {
                if self.ai.panel.is_streaming() {
                    self.ai_cancel();
                } else {
                    self.ai_send(hwnd);
                }
            }
            assistant::Hit::Session(i) => {
                if let Some(id) = self.ai.panel.sorted_sessions().get(i).map(|s| s.id.clone()) {
                    self.ai_open_session(&id);
                }
            }
            assistant::Hit::SessionDelete(i) => {
                if let Some(id) = self.ai.panel.sorted_sessions().get(i).map(|s| s.id.clone()) {
                    self.ai_ask_delete_session(&id);
                }
            }
            assistant::Hit::FollowUpQuestion(msg_idx, q_idx) => {
                self.ai_click_follow_up(msg_idx, q_idx);
            }
            assistant::Hit::PopoverInside | assistant::Hit::Header => {}
            assistant::Hit::Messages => {
                if self.focus == Focus::AiInput {
                    self.focus = Focus::Main;
                }
            }
        }
    }
}
