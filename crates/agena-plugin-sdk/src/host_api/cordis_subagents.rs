//! In-process authority for one invocation and one accepted child attempt.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::AgentSpawnRequest;
use crate::Result;

pub struct CordisSubagentStart {
    pub request: AgentSpawnRequest,
    pub provider: String,
    pub label: String,
    pub reserved_id: Option<String>,
    pub attachments: Vec<agena_domain::AttachmentItem>,
    pub cwd: Option<String>,
    pub max_output_tokens: Option<u32>,
    pub max_active_children: u64,
    pub max_depth: Option<u64>,
    pub delivery: CordisSubagentDelivery,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CordisSubagentDelivery {
    Parent,
    Caller,
}

pub struct CordisSubagentStarted {
    pub child_id: i64,
    pub reserved_id: Option<String>,
    pub message_id: i64,
    pub name: String,
    pub ancestor_sessions: Vec<i64>,
    pub run: Arc<dyn CordisSubagentRun>,
}

impl CordisSubagentStarted {
    pub fn visible_child_id(&self) -> String {
        self.reserved_id
            .clone()
            .unwrap_or_else(|| self.child_id.to_string())
    }
}

#[derive(Clone)]
pub struct CordisSubagentResult {
    pub stop_reason: CordisSubagentStopReason,
    pub text: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CordisSubagentStopReason {
    Completed,
    Aborted,
    Error,
    MaxTokens,
    Refusal,
}

#[async_trait::async_trait]
pub trait CordisSubagentRun: Send + Sync {
    async fn result(&self) -> Result<CordisSubagentResult>;
    async fn dispose(&self) -> Result<()>;
}

/// The adapter parks a foreground VM while it awaits an accepted child. The
/// runtime can yield the outer tool's concurrency slot during that wait.
pub type CordisSubagentWait =
    std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;

/// Session IDs in this request are addresses, not callback authority.
#[derive(Debug, Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum CordisSubagentControl {
    Scope,
    List { parent_id: String },
    Resolve { name: String },
    Message {
        target_id: String,
        text: String,
        #[serde(default)]
        attachments: Vec<agena_domain::AttachmentItem>,
    },
    Interrupt { target_id: String },
    Wait { timeout_ms: u64 },
    WaitForTreeWork,
    ModelRoutes,
}
