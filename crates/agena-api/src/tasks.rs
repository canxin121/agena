//! Shared task contracts for HTTP, Web and terminal clients. Lifecycle and
//! permission decisions remain in the server, not in presentation code.
pub use agena_superagent::{
    ChatMessage, ChatReply, CreateTask, ResumeTask, Task, TaskQuery, TaskState,
};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRevisionRequest {
    pub expected_revision: i64,
}
