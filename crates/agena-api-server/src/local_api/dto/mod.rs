use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use agena::{
    agent::{AgentMode, AgentPermissionConfig},
    agents::AgentScope,
    message::{PartContent, UserInputReply},
    model_catalog::{ModelCatalogEntryRecord, ModelCatalogEntrySourceKind},
    permission::PermissionMode,
    permission::PermissionReply,
    provider::ProviderModel,
    runtime::{
        RuntimeBackgroundTask, RuntimeBackgroundTaskKind, RuntimeBackgroundTaskOrigin,
        RuntimeBackgroundTaskStatus,
    },
    session::GoalStatus,
};

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
pub use agena_api::resource::{
    ScheduledJobResource, ScheduledJobRunResource, SessionAutomationResource,
};
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub background_tasks: Vec<RuntimeBackgroundTaskResource>,
    pub automation: RuntimeAutomationResource,
    pub operator: RuntimeOperatorResource,
}
