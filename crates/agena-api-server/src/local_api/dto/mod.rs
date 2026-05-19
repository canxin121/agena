use agena_api::resource::MessageRole;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use agena::{
    agent::{AgentMode, AgentPermissionConfig, AgentRunConfig, PermissionConfig},
    agents::AgentScope,
    message::{
        MessageMetadata, MessagePart, MessageStatus, MessageUsage, PartContent, UserInputReply,
        UserInputRequest,
    },
    model::ModelRef,
    model_catalog::{CatalogModelDefinition, ModelCatalogEntryRecord, ModelCatalogEntrySourceKind},
    permission::PermissionMode,
    permission::{PermissionReply, PermissionRequest},
    provider::ProviderModel,
    session::{GoalStatus, SessionStatus, SessionSummary},
};

#[cfg(test)]
use agena::message::ExecutionStatus;
#[cfg(test)]
use agena::role::Role;

mod access;
mod auth;
mod marketplace;
mod messages;
mod model_catalog;
mod plugins;
mod providers;
mod runtime;
mod sessions;
mod workspaces;

pub use access::*;
pub use auth::*;
pub use marketplace::*;
pub use messages::*;
pub use model_catalog::*;
pub use plugins::*;
pub use providers::*;
pub use runtime::*;
pub use sessions::*;
pub use workspaces::*;

#[derive(Debug, Clone, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
    pub database_connected: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeTaskResource {
    pub enabled: bool,
    pub interval_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSessionCacheResource {
    pub max_sessions: usize,
    pub ttl_secs: u64,
    pub max_bytes: usize,
    pub entry_count: usize,
    pub total_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledJobRunResource {
    pub triggered_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: agena_scheduler::JobRunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledJobResource {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<DateTime<Utc>>,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_fire_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fired_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<ScheduledJobRunResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionAutomationResource {
    pub job_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_job: Option<ScheduledJobResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeAutomationResource {
    pub enabled: bool,
    pub job_count: usize,
    pub recent_jobs: Vec<ScheduledJobResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStatusResponse {
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
    pub workspace_root: String,
    pub config_path: String,
    pub config_found: bool,
    pub provider_ids: Vec<String>,
    pub plugin_count: usize,
    pub session_runtime_available: bool,
    pub watch_paths: Vec<String>,
    pub reload: RuntimeTaskResource,
    pub janitor: RuntimeTaskResource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_cache: Option<RuntimeSessionCacheResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_catalog: Option<ModelCatalogResponse>,
    pub automation: RuntimeAutomationResource,
    pub operator: RuntimeOperatorResource,
}
