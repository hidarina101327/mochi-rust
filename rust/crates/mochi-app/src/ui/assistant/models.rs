//! 定义 AI 助手的消息选择、流式输出和面板状态。
use super::*;

/// 进行中的一次回复。
#[derive(Debug, Clone, Default)]
pub struct Streaming {
    pub plan: Vec<String>,
    pub status: String,
    pub reasoning: String,
    pub content: String,
    pub model: String,
    /// 回合进行中时，运行时公开的服务方用量。
    /// 回合完成后用量也会持久化到 assistant 消息上。
    pub usage: Option<serde_json::Value>,
    /// 完整的结构化实时轨迹。旧字段仍用于兼容已有事件/测试，绘制优先使用此字段。
    pub trace: Vec<AgentTraceStep>,
    /// 本轮到目前为止的工具调用：(名字, 结果 ok?, 摘要)。
    pub tools: Vec<(String, Option<bool>, Option<String>)>,
    /// 驱动流式输出的省略号动画，不改动真实的状态文本。
    pub animation_phase: u8,
    pub started_at_ms: i64,
    pub last_event_at_ms: i64,
}

/// 锚定在一条已渲染 assistant 消息上的拖拽选区。消息下标来自当前的
/// 可见消息布局；文本布局持有 run/UTF-8 偏移对，使换行、带样式的
/// Markdown 仍然可以选中。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSelection {
    pub anchor: (usize, crate::ui::ai_markdown::TextPoint),
    pub focus: (usize, crate::ui::ai_markdown::TextPoint),
    pub dragging: bool,
}

impl TextSelection {
    pub fn collapsed(self) -> bool {
        self.anchor == self.focus
    }

    pub fn ordered(
        self,
    ) -> (
        (usize, crate::ui::ai_markdown::TextPoint),
        (usize, crate::ui::ai_markdown::TextPoint),
    ) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }
}

pub struct State {
    pub pending_files: Vec<std::path::PathBuf>,
    pub files_scroll: f32,
    pub pending_images: Vec<String>,
    /// 从编辑器右键菜单排队的文档选区。值保持 Electron 的
    /// `DocumentSelectionContext` 结构，直到 App 发送出去。
    pub pending_selections: context::PendingSelections,
    pub image_placeholders: std::collections::HashMap<String, String>,
    pub next_image_index: u64,
    pub image_previews: std::collections::HashMap<String, ImagePreview>,
    pub horizontal: crate::ui::ai_scroll::State,
    pub hover_content: Option<(usize, usize)>,
    pub hover_content_hit: bool,
    pub hover_message: Option<usize>,
    /// 指针下最近的语义控件。指针跟踪归 App 负责；
    /// 面板用它来显示紧凑的悬停提示/tooltip。
    pub hover_hit: Option<Hit>,
    /// 已完成回答的思考卡展开状态；按可见消息下标保存，换会话时自动失效。
    pub expanded_traces: HashSet<usize>,
    /// 思考/工具行的完整详情展开状态。
    pub expanded_trace_steps: HashSet<(usize, usize)>,
    /// 流式思考默认只展示尾部，用户可展开完整本轮 reasoning。
    pub streaming_reasoning_expanded: bool,
    pub locator_pending: Option<String>,
    pub located: Option<String>,
    pub locator_timer: Option<u32>,
    pub sessions: Vec<AiSessionMeta>,
    pub active: Option<AiConversation>,
    pub streaming: Option<Streaming>,
    pub input: TextField,
    pub show_conversations: bool,
    pub conversations_scroll: f32,
    pub scroll: f32,
    /// 独立工作区右侧导航的内部滚动位置；由 App 的滚轮路由更新。
    pub navigation_scroll: f32,
    /// 新内容到达时是否自动滚到底（用户手动往上滚过就不再打扰）。
    pub stick_to_bottom: bool,
    pub error: String,
    /// 未配置模型服务方时显示的提示。
    pub provider_missing: bool,
    pub agents: Vec<(String, String)>,
    pub selected_agent_id: Option<String>,
    pub agent_name: String,
    pub automatic_edits: bool,
    pub follow_up_frequency: mochi_core::ai::FollowUpFrequency,
    pub loaded_skills: Vec<String>,
    pub standalone: bool,
    /// Electron 面板的会话内容搜索状态。搜索结果由 root 在点击/键盘事件中定位。
    pub search_open: bool,
    pub search_query: TextField,
    pub search_focused: bool,
    pub search_result_index: i32,
    /// 原生拖拽选区，作用于已渲染的 assistant 文本。
    /// 刻意与输入框 TextField 的选区/焦点保持独立。
    pub text_selection: Option<TextSelection>,
}

impl Default for State {
    fn default() -> Self {
        let mut input = TextField::new("输入消息... (Shift+Enter 换行，可粘贴图片)");
        input.style = TextStyle::Label;
        State {
            pending_files: Vec::new(),
            files_scroll: 0.0,
            pending_images: Vec::new(),
            image_previews: Default::default(),
            pending_selections: Vec::new(),
            image_placeholders: Default::default(),
            next_image_index: 1,
            horizontal: Default::default(),
            hover_content: None,
            hover_content_hit: false,
            hover_message: None,
            hover_hit: None,
            expanded_traces: HashSet::new(),
            expanded_trace_steps: HashSet::new(),
            streaming_reasoning_expanded: false,
            locator_pending: None,
            located: None,
            locator_timer: None,
            sessions: Vec::new(),
            active: None,
            streaming: None,
            input,
            show_conversations: false,
            conversations_scroll: 0.0,
            scroll: 0.0,
            navigation_scroll: 0.0,
            stick_to_bottom: true,
            error: String::new(),
            provider_missing: false,
            agents: Vec::new(),
            selected_agent_id: None,
            agent_name: "通用助手".into(),
            automatic_edits: false,
            follow_up_frequency: mochi_core::ai::FollowUpFrequency::Medium,
            loaded_skills: Vec::new(),
            standalone: false,
            search_open: false,
            search_query: TextField::new("搜索会话内容…"),
            search_focused: false,
            search_result_index: -1,
            text_selection: None,
        }
    }
}

impl State {
    pub fn is_streaming(&self) -> bool {
        self.streaming.is_some()
    }

    pub fn begin_text_selection(
        &mut self,
        message: usize,
        point: crate::ui::ai_markdown::TextPoint,
    ) {
        let value = (message, point);
        self.text_selection = Some(TextSelection {
            anchor: value,
            focus: value,
            dragging: true,
        });
    }

    pub fn update_text_selection(
        &mut self,
        message: usize,
        point: crate::ui::ai_markdown::TextPoint,
    ) -> bool {
        let Some(selection) = self.text_selection.as_mut() else {
            return false;
        };
        selection.focus = (message, point);
        true
    }

    /// 拖拽扩展时始终保持选区在起始消息内部。外围聊天框架是语义边界：
    /// 指针一旦跨进另一条 user/assistant 气泡，就夹紧到锚定回复最近的
    /// 边缘，而不是让选区变成跨消息的无效范围、在下一次绘制时消失。
    pub fn update_text_selection_clamped(
        &mut self,
        layout: &Layout,
        message: usize,
        point: crate::ui::ai_markdown::TextPoint,
    ) -> bool {
        let Some(selection) = self.text_selection else {
            return false;
        };
        if message == selection.anchor.0 {
            return self.update_text_selection(message, point);
        }
        let Some(anchor) = layout.messages.get(selection.anchor.0) else {
            self.clear_text_selection();
            return false;
        };
        let fragments = anchor.body.selectable_text(Some(&anchor.horizontal));
        let Some(last) = fragments.last() else {
            return false;
        };
        let edge = if message < selection.anchor.0 {
            crate::ui::ai_markdown::TextPoint {
                fragment: 0,
                offset: 0,
            }
        } else {
            crate::ui::ai_markdown::TextPoint {
                fragment: fragments.len() - 1,
                offset: last.text.len(),
            }
        };
        self.update_text_selection(selection.anchor.0, edge)
    }

    pub fn end_text_selection(&mut self) -> bool {
        let Some(selection) = self.text_selection.as_mut() else {
            return false;
        };
        selection.dragging = false;
        !selection.collapsed()
    }

    pub fn clear_text_selection(&mut self) {
        self.text_selection = None;
    }

    pub fn selected_text(&self, layout: &Layout) -> Option<String> {
        let selection = self.text_selection?;
        if selection.collapsed() {
            return None;
        }
        let (first, last) = selection.ordered();
        if first.0 != last.0 {
            // 选区被限制在单条消息内。消息边界是语义间隙
            // （头像/轨迹/操作按钮），复制时不会误带上框架或隐藏的工具消息。
            return None;
        }
        let message = layout.messages.get(first.0)?;
        Some(
            message
                .body
                .selected_text(first.1, last.1, Some(&message.horizontal)),
        )
    }

    pub fn selection_rects(&self, layout: &Layout, message_index: usize, scroll: f32) -> Vec<Rect> {
        let Some(selection) = self.text_selection else {
            return Vec::new();
        };
        if selection.anchor.0 != message_index || selection.focus.0 != message_index {
            return Vec::new();
        }
        let Some(message) = layout.messages.get(message_index) else {
            return Vec::new();
        };
        let (first, last) = selection.ordered();
        let body_top = layout.messages_rect.top + message.body_top - scroll;
        message
            .body
            .text_selection_rects(first.1, last.1, Some(&message.horizontal))
            .into_iter()
            .map(|rect| {
                Rect::new(
                    rect.left + message.body_left,
                    rect.top + body_top,
                    rect.right + message.body_left,
                    rect.bottom + body_top,
                )
            })
            .collect()
    }

    /// 可见消息：user/assistant 且未标 hidden（工具往返不显示）。
    pub fn visible_messages(&self) -> Vec<(String, String)> {
        let Some(c) = &self.active else {
            return Vec::new();
        };
        c.messages
            .iter()
            .filter(|m| mochi_core::ai::locator::visible(m))
            .map(|m| {
                let mut content = m.content().to_owned();
                if let Some(trace) = m.get("trace").and_then(|v| v.as_array()) {
                    if let Some(plan) = trace
                        .iter()
                        .rev()
                        .find(|v| v["kind"] == "plan")
                        .and_then(|v| v["steps"].as_array())
                    {
                        content.push_str(&format!("\n\n执行计划\n{}", plan_lines(plan).join("\n")));
                    }
                }
                (m.role().to_owned(), content)
            })
            .collect()
    }

    /// 会话列表按更新时间倒序（`sortedSessions`）。
    pub fn sorted_sessions(&self) -> Vec<&AiSessionMeta> {
        let mut v: Vec<&AiSessionMeta> = self.sessions.iter().collect();
        v.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.updated_at.cmp(&a.updated_at))
        });
        v
    }
    pub fn message_ids(&self) -> Vec<Option<String>> {
        self.active
            .iter()
            .flat_map(|c| c.messages.iter())
            .filter(|m| mochi_core::ai::locator::visible(m))
            .map(|m| m.id().map(str::to_owned))
            .collect()
    }
    /// 搜索语义刻意与 Electron 面板保持一致：只搜可见的 user/assistant
    /// 消息，不区分大小写，并保留根视图滚动与高亮绘制所用的可见消息下标。
    pub fn search_results(&self) -> Vec<usize> {
        let query = self.search_query.text().trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        self.visible_messages()
            .iter()
            .enumerate()
            .filter_map(|(index, (_, content))| {
                content.to_lowercase().contains(&query).then_some(index)
            })
            .collect()
    }
    pub fn apply_locator(&mut self, layout: &Layout) {
        let Some(id) = self.locator_pending.take() else {
            return;
        };
        if let Some(index) = self
            .message_ids()
            .iter()
            .position(|value| value.as_deref() == Some(&id))
        {
            if let Some(message) = layout.messages.get(index) {
                self.scroll = (message.top + message.height / 2.0
                    - layout.messages_rect.height() / 2.0)
                    .clamp(0.0, layout.max_scroll());
                self.stick_to_bottom = false;
                self.located = Some(id);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageAction {
    Mount,
    Copy,
    Delete,
    Insert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    AttachFiles,
    FilesBack,
    FilesForward,
    RemoveFile(usize),
    OpenFile(usize),
    RemoveImage(usize),
    Content(usize, usize),
    Message(usize, MessageAction),
    PendingEditApprove(usize),
    PendingEditReject(usize),
    AgentPicker,
    MountSession,
    ApproveMode,
    AutomaticMode,
    Conversations,
    NewSession,
    Clear,
    Settings,
    AgentConfig,
    Close,
    /// 把右侧栏收起，改成窗口内的悬浮窗；再点一次回到侧栏。
    Float,
    Input,
    Send,
    /// 搜索栏开关及其控件（对应 Electron 的会话搜索）。
    SearchToggle,
    SearchInput,
    SearchPrevious,
    SearchNext,
    SearchClose,
    /// 已发送消息上的选区上下文小片：(可见消息下标, 小片下标)。
    SelectionLocate(usize, usize),
    /// 输入区待发送小片的控件。
    PendingSelectionLocate(usize),
    PendingSelectionRemove(usize),
    /// 为已持久化的提案打开现有的原生评审界面。
    PendingCardReview(usize),
    /// 头部「上一个问题」定位。
    PreviousQuestion,
    /// 流式状态与历史轨迹交互。
    TraceToggle(usize),
    TraceStep(usize, usize),
    StreamingReasoningToggle,
    ScrollToBottom,
    /// 独立 AI 工作区右侧「对话导航」中的可见消息。
    NavigateMessage(usize),
    /// 会话弹层里的某一项（下标进 `sorted_sessions()`）。
    Session(usize),
    SessionDelete(usize),
    /// 弹层的「新建」。
    PopoverNew,
    /// 弹层里但不在任何项上。
    PopoverInside,
    /// 未配置模型服务方时显示的“前往设置”按钮。
    OpenSettings,
    Messages,
    Header,
    FollowUpQuestion(usize, usize),
}

/// 智能追问区域布局：容器顶部 y、高度及各个建议按钮相对于追问内容区的矩形与文本。
#[derive(Debug, Clone)]
pub struct LaidFollowUp {
    pub top: f32,
    pub height: f32,
    pub buttons: Vec<(Rect, String)>,
}

/// 排好的一条消息：角色、视觉行（已断行）、在内容区的 y。
#[derive(Debug, Clone)]
pub struct LaidMessage {
    pub images: Vec<(Rect, String)>,
    pub scroll_key: String,
    pub horizontal: crate::ui::ai_markdown::Offsets,
    pub source: String,
    pub action_target: Option<(String, String)>,
    pub actions_top: f32,
    pub role: String,
    pub body: std::rc::Rc<crate::ui::ai_markdown::Layout>,
    /// 原始结构化轨迹 JSON。`AgentTraceStep` 目前只保证序列化，
    /// 因而这里保留 Value 以兼容历史 Electron 会话字段。
    pub trace: Vec<serde_json::Value>,
    pub trace_expanded: bool,
    pub trace_top: f32,
    pub trace_height: f32,
    pub activity_height: f32,
    pub streaming_reasoning: Option<String>,
    pub active_tool: Option<(String, Option<String>)>,
    pub live_plan: Option<serde_json::Value>,
    pub streaming_status: Option<String>,
    /// 挂在某条已发送 user 消息上的选区上下文。
    pub selection_values: Vec<serde_json::Value>,
    pub selection_chips: context::SelectionChipsLayout,
    pub selection_top: f32,
    /// 已持久化的审批提案，旧版 pendingEdit 卡除外。
    pub pending_card: Option<context::PendingCard>,
    pub pending_card_top: f32,
    pub pending_card_height: f32,
    pub body_top: f32,
    pub body_left: f32,
    pub body_width: f32,
    pub bubble: Option<Rect>,
    pub metadata: String,
    pub tools: Vec<String>,
    pub pending_edit: Option<serde_json::Value>,
    pub pending_top: f32,
    pub follow_up: Option<LaidFollowUp>,
    pub top: f32,
    pub height: f32,
    pub streaming: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub files_viewport: Option<Rect>,
    pub files_max_scroll: f32,
    pub session_id: Option<String>,
    pub entries: Vec<(Rect, Hit)>,
    pub messages_rect: Rect,
    pub composer_rect: Option<Rect>,
    pub navigation_rect: Option<Rect>,
    pub navigation_max_scroll: f32,
    pub messages: Vec<LaidMessage>,
    pub content_height: f32,
    pub popover: Option<Rect>,
    pub conversations_viewport: Option<Rect>,
    pub conversations_max_scroll: f32,
    pub conversation_rows: Vec<(usize, Rect)>,
}

impl Layout {
    /// Markdown 原点。保留这个公开 API 供公式/表格横向滚动和复制命中使用；
    /// chrome（头像、思考卡、用户气泡）增加后，原点由每条消息自身记录。
    pub fn body_origin(&self, index: usize, scroll: f32) -> Option<(f32, f32)> {
        let m = self.messages.get(index)?;
        Some((m.body_left, self.messages_rect.top + m.body_top - scroll))
    }
    pub fn content_at(&self, scroll: f32, x: f32, y: f32, hover: bool) -> Option<(usize, usize)> {
        let i = self.message_at(scroll, x, y)?;
        let message = &self.messages[i];
        let origin = self.body_origin(i, scroll)?;
        let x = x - origin.0;
        let y = y - origin.1;
        if x < 0.0 || x >= self.messages_rect.width() - MSG_PAD_X * 2.0 {
            return None;
        }
        let hit = message
            .body
            .content_at(x, y, Some(&message.horizontal), hover)?;
        Some((i, hit))
    }
    /// 在渲染正文中做文本命中，不含头像、轨迹卡和消息操作按钮。
    /// 这是原生鼠标拖拽选区的入口，刻意复用缓存的 Markdown 布局。
    pub fn text_point_at(
        &self,
        scroll: f32,
        x: f32,
        y: f32,
    ) -> Option<(usize, crate::ui::ai_markdown::TextPoint)> {
        // 长回复会超出被裁剪的消息视口，视口外的文本
        // 不得拦截输入框和 agent 控件的点击。
        if !self.messages_rect.contains(x, y) {
            return None;
        }
        self.messages
            .iter()
            .enumerate()
            .find_map(|(index, message)| {
                let body_top = self.messages_rect.top + message.body_top - scroll;
                let body = Rect::new(
                    message.body_left,
                    body_top,
                    message.body_left + message.body_width,
                    body_top + message.body.height,
                );
                if !body.contains(x, y) {
                    return None;
                }
                let point = message.body.text_point_at(
                    x - message.body_left,
                    y - body_top,
                    Some(&message.horizontal),
                )?;
                Some((index, point))
            })
    }

    /// 拖拽中指针离开正文时，从最近的消息边缘继续扩展。
    /// 与原生文本控件一致：拖过回复末尾会一直选中到最后一行，
    /// 而不是在最后一个界内像素处冻结。
    pub fn text_point_near(
        &self,
        scroll: f32,
        x: f32,
        y: f32,
    ) -> Option<(usize, crate::ui::ai_markdown::TextPoint)> {
        if let Some(point) = self.text_point_at(scroll, x, y) {
            return Some(point);
        }
        self.messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                if message.body.height <= 0.0 || message.body_width <= 0.0 {
                    return None;
                }
                let body_top = self.messages_rect.top + message.body_top - scroll;
                let body = Rect::new(
                    message.body_left,
                    body_top,
                    message.body_left + message.body_width,
                    body_top + message.body.height,
                );
                let dx = if x < body.left {
                    body.left - x
                } else if x > body.right {
                    x - body.right
                } else {
                    0.0
                };
                let dy = if y < body.top {
                    body.top - y
                } else if y > body.bottom {
                    y - body.bottom
                } else {
                    0.0
                };
                let clamped_x = x.clamp(body.left, (body.right - 0.01).max(body.left));
                let clamped_y = y.clamp(body.top, (body.bottom - 0.01).max(body.top));
                let point = message.body.text_point_at(
                    clamped_x - message.body_left,
                    clamped_y - body_top,
                    Some(&message.horizontal),
                )?;
                Some(((dx * dx + dy * dy, index), (index, point)))
            })
            .min_by(|(a, _), (b, _)| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
            .map(|(_, point)| point)
    }
    pub fn link_at(&self, scroll: f32, x: f32, y: f32) -> Option<(usize, String)> {
        let i = self.message_at(scroll, x, y)?;
        let message = &self.messages[i];
        let origin = self.body_origin(i, scroll)?;
        let x = x - origin.0;
        let y = y - origin.1;
        if x < 0.0 || x >= self.messages_rect.width() - MSG_PAD_X * 2.0 {
            return None;
        }
        Some((i, message.body.link_at(x, y, Some(&message.horizontal))?))
    }
    pub fn content_copy(
        &self,
        state: &State,
        index: usize,
        action: usize,
    ) -> Option<crate::ui::ai_markdown::CopyPayload> {
        if self.session_id != state.active.as_ref().map(|c| c.id.clone()) {
            return None;
        }
        let message = self.messages.get(index)?;
        let unchanged = if message.streaming {
            let st = state.streaming.as_ref()?;
            message.source.as_str()
                == if st.content.is_empty() {
                    st.status.as_str()
                } else {
                    st.content.as_str()
                }
        } else {
            state
                .active
                .as_ref()
                .and_then(|conversation| {
                    conversation
                        .messages
                        .iter()
                        .filter(|stored| mochi_core::ai::locator::visible(stored))
                        .nth(index)
                })
                .is_some_and(|stored| {
                    stored.role() == message.role && stored.content() == message.source
                })
        };
        if !unchanged {
            return None;
        }
        message.body.copy_payload(action).cloned()
    }
    pub fn message_at(&self, scroll: f32, x: f32, y: f32) -> Option<usize> {
        if !self.messages_rect.contains(x, y) || self.popover.is_some_and(|r| r.contains(x, y)) {
            return None;
        }
        let local = y - self.messages_rect.top + scroll;
        self.messages
            .iter()
            .position(|m| local >= m.top && local < m.top + m.height)
    }
    pub fn message_buttons(&self, index: usize, scroll: f32) -> Vec<(Rect, MessageAction)> {
        let Some(message) = self
            .messages
            .get(index)
            .filter(|m| m.action_target.is_some())
        else {
            return Vec::new();
        };
        let available = (message.body_width - 12.0).max(4.0);
        let width = 60.0_f32.min((available - 12.0) / 4.0).max(1.0);
        [
            MessageAction::Mount,
            MessageAction::Copy,
            MessageAction::Delete,
            MessageAction::Insert,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, a)| {
            (
                Rect::from_size(
                    message.body_left + i as f32 * (width + 4.0),
                    self.messages_rect.top + message.actions_top - scroll,
                    width,
                    30.0,
                ),
                a,
            )
        })
        .collect()
    }
    pub fn message_hit(&self, scroll: f32, x: f32, y: f32) -> Option<Hit> {
        let index = self.message_at(scroll, x, y)?;
        if let Some((_, hit)) = self
            .pending_edit_buttons(index, scroll)
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
        {
            return Some(hit);
        }
        self.message_buttons(index, scroll)
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, a)| Hit::Message(index, a))
    }
    pub fn pending_edit_buttons(&self, index: usize, scroll: f32) -> Vec<(Rect, Hit)> {
        let Some(message) = self.messages.get(index) else {
            return Vec::new();
        };
        let Some(edit) = message.pending_edit.as_ref() else {
            return Vec::new();
        };
        if edit["status"].as_str().unwrap_or("pending") != "pending" {
            return Vec::new();
        }
        // `pending_top` 是卡片顶边。两个操作按钮要放在卡片下方的行内，
        // 而不是定位在卡片上方一整个卡片高度处
        // （旧布局在这里存的是卡片底边）。
        let top =
            self.messages_rect.top + message.pending_top - scroll + PENDING_EDIT_CARD_H - 36.0;
        let width = ((self.messages_rect.width() - MSG_PAD_X * 2.0 - 8.0) / 2.0).max(1.0);
        let left = self.messages_rect.left + MSG_PAD_X;
        vec![
            (
                Rect::new(left, top, left + width, top + 28.0),
                Hit::PendingEditApprove(index),
            ),
            (
                Rect::new(
                    left + width + 8.0,
                    top,
                    self.messages_rect.right - MSG_PAD_X,
                    top + 28.0,
                ),
                Hit::PendingEditReject(index),
            ),
        ]
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.entries
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h)
    }
    pub fn rect_of(&self, h: Hit) -> Option<Rect> {
        // 多个控件刻意共享同一语义命中（搜索栏背景+输入框，
        // 或每张图片一个移除按钮）。最后插入的矩形是最小、
        // 可交互的那个。
        self.entries
            .iter()
            .rev()
            .find(|(_, x)| *x == h)
            .map(|(r, _)| *r)
    }
    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.messages_rect.height()).max(0.0)
    }
}
