//! Business tasks live longer than model executions. This crate contains only
//! contracts: Runtime owns dispatch, SQLite owns persistence, and transports
//! adapt these operations. Chat messages never implicitly authorize a task.

mod chat;
mod companion;
pub use companion::*;
mod conversation;
mod integrations;
pub use integrations::*;
mod task;

pub use chat::{ChatCommand, ChatMessage, ChatReply};
pub use conversation::{ConversationWork, WorkNotice, WorkUpdate};
pub use task::{CreateTask, ResumeTask, Task, TaskQuery, TaskState};

use async_trait::async_trait;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("{0}")]
    Policy(String),
    #[error("task not found in this scope")]
    NotFound,
    #[error("{0}")]
    Conflict(&'static str),
    #[error("{0}")]
    Unavailable(&'static str),
    #[error("task persistence failed: {0}")]
    Storage(String),
}

/// All writes must be atomic. An insert replay returns the original task only
/// when its input matches; revisions fence stale writers.
#[async_trait]
pub trait TaskRepository: Send + Sync {
    async fn insert(&self, task: Task) -> Result<Task>;
    async fn get(&self, id: Uuid) -> Result<Task>;
    async fn list(&self, query: TaskQuery) -> Result<Vec<Task>>;
    /// Active work first, then the oldest queued work. Terminal tasks must not
    /// starve dispatch. The initial controller admits one worker at a time.
    async fn pending(&self, limit: usize) -> Result<Vec<Task>>;
    async fn replace(&self, task: Task, expected_revision: i64) -> Result<Task>;
    /// Bind an ingress event to its original text before any effect. A retry
    /// cannot change a chat event into a task/control command or vice versa.
    async fn reserve_chat(&self, message: &ChatMessage) -> Result<Option<ChatReply>>;
    async fn finish_chat(&self, message: &ChatMessage, reply: ChatReply) -> Result<ChatReply>;
    async fn for_worker(&self, session_id: i64) -> Result<Option<Task>>;
    async fn active_workers(&self, source_session_id: i64) -> Result<Vec<i64>>;
    async fn pending_notices(&self, now_ms: i64, limit: usize) -> Result<Vec<WorkNotice>>;
    async fn prepare_notice(&self, key: &str, text: &str) -> Result<String>;
    async fn acknowledge_notice(&self, key: &str) -> Result<()>;
    async fn defer_notice(&self, key: &str, next_ms: i64) -> Result<()>;
}

/// Operator-facing service. This is not a public webhook contract: a trusted
/// adapter must authenticate external identities before granting this access.
#[async_trait]
pub trait SuperagentService: Send + Sync {
    async fn integrations(&self, source: i64) -> Result<IntegrationsSnapshot>;
    async fn update_channel_policy(
        &self,
        source: i64,
        id: Uuid,
        request: ChannelPolicyUpdate,
    ) -> Result<IntegrationBinding>;

    async fn create_integration(
        &self,
        source: i64,
        request: CreateIntegration,
    ) -> Result<IntegrationBinding>;
    async fn integration_detail(&self, source: i64, id: Uuid) -> Result<IntegrationDetail>;
    async fn control_integration(
        &self,
        source: i64,
        id: Uuid,
        request: IntegrationControl,
    ) -> Result<IntegrationBinding>;

    async fn create(&self, request: CreateTask) -> Result<Task>;
    async fn get(&self, id: Uuid) -> Result<Task>;
    async fn list(&self, query: TaskQuery) -> Result<Vec<Task>>;
    async fn cancel(&self, id: Uuid, expected_revision: Option<i64>) -> Result<Task>;
    async fn resume(&self, id: Uuid, request: ResumeTask) -> Result<Task>;
    async fn accept(&self, id: Uuid, expected_revision: i64) -> Result<Task>;
    async fn chat(&self, message: ChatMessage) -> Result<ChatReply>;
    async fn assistant(&self, source: i64) -> Result<AssistantSnapshot>;
    async fn update_assistant(
        &self,
        source: i64,
        update: AssistantUpdate,
    ) -> Result<AssistantProfile>;
    async fn control_reminder(
        &self,
        source: i64,
        id: Uuid,
        control: ReminderControl,
    ) -> Result<Reminder>;
    async fn change_assistant_memory(
        &self,
        source: i64,
        update: AssistantMemoryUpdate,
    ) -> Result<Option<AssistantMemory>>;
}

pub(crate) fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > 160 || key.trim() != key || key.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "request key must contain 1–160 non-control bytes without surrounding whitespace",
        ));
    }
    Ok(())
}

pub(crate) fn validate_text(text: &str) -> Result<()> {
    if text.trim().is_empty() || text.len() > 32 * 1024 {
        return Err(Error::Invalid("message must contain 1–32768 UTF-8 bytes"));
    }
    Ok(())
}

mod channel_access;
pub use channel_access::*;
