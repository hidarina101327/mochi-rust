//! AI 层：OpenAI/Anthropic 客户端、会话存储、权限门、Agent 运行时与工具。

pub mod agent_config;
pub mod agent_inbox;
pub mod agent_runner;
pub mod agent_tasks;
pub mod anthropic;
#[cfg(test)]
mod anthropic_transport_tests;
pub mod assets;
pub mod attachments;
pub mod document_mounts;
pub mod follow_up;
pub mod locator;
pub mod mcp;
pub mod memory_extract;
pub mod message_ops;
pub mod models;
pub mod permission;
pub mod provider_presets;
pub mod providers;
pub mod runtime_config;
pub mod service;
pub mod session;
pub mod tools;

pub use follow_up::{
    compute_response_revision, get_message_follow_up, set_message_follow_up, FollowUpFrequency,
    FollowUpState, FollowUpStatus, FollowUpSuggestion,
};

pub use agent_config::{
    AgentConfigService, AgentDefinition, McpServerDefinition, PromptToolDefinition,
    QuickActionDefinition, Selection, SkillDefinition, GENERAL_ASSISTANT_ID,
};
pub use agent_runner::{
    AgentEvent, AgentRequest, AgentResult, AgentTraceStep, ModelClient, ToolRunner, SYSTEM_PROMPT,
};
pub use agent_tasks::{
    filter_tools_for_autonomy, is_task_due, load_tasks, take_due_tasks, AgentTask, TaskAutonomy,
    TaskRunRecord, TaskState,
};
pub use document_mounts::{
    document_relative_path, AiDocumentMount, AiDocumentMountIndex, AiDocumentMountService,
    CreateMountInput, MountScope,
};
pub use memory_extract::{
    extract_from_exchange, ExtractedMemory, ExtractionOutcome, MemoryExtractionParams, OneShotModel,
};
pub use models::*;
pub use permission::{
    ActionPermissions, AiPermissionLevel, AiPermissionService, AiToolAction, PermissionDenied,
};
pub use service::{chat_url, map_finish_reason, AiService, SseAccumulator};
pub use session::{
    new_conversation_id, new_project_id, AiConversation, AiProject, AiSessionIndex, AiSessionMeta,
    AiSessionService, AiStoredMessage,
};
