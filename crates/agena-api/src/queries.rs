//! Read-only queries.
//!
//! The request contract lives in [`agena_session_api`] so
//! REST, WebSocket and plugin transports share one declaration. Only the
//! transport-facing result enum lives here.

pub use agena_session_api::queries::*;

use serde::{Deserialize, Serialize};

use crate::pagination::PaginatedResponse;
use crate::resource::{
    BackgroundActivityLogResource, BackgroundActivityResource, HealthResponse,
    PermissionRuleResource, ProviderAdapterModelsResponse, ProviderModelsResponse,
    ProviderSummaryResource, RuntimeStatusResponse, SessionResource, WorkspaceResource,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
/// Result of a [`Query`].
pub enum QueryResult {
    Parts(crate::live::SessionPartsResource),
    Content(agena_domain::ContentPage),
    ContentText(agena_domain::ContentTextPage),
    Health(HealthResponse),
    Runtime(RuntimeStatusResponse),
    Providers(Vec<ProviderSummaryResource>),
    ProviderModels(ProviderModelsResponse),
    ProviderAdapterModels(ProviderAdapterModelsResponse),
    Workspaces(PaginatedResponse<WorkspaceResource>),
    Workspace(WorkspaceResource),
    Sessions(PaginatedResponse<SessionResource>),
    Session(SessionResource),
    SessionState(crate::resource::SessionExecutionResource),
    SessionCost(agena_domain::SessionCostSummary),
    SessionTree(Vec<SessionResource>),
    SessionPlatforms(agena_assistant::PlatformInventory),
    PermissionRules(PaginatedResponse<PermissionRuleResource>),
    PermissionRule(PermissionRuleResource),
    Activities(Vec<BackgroundActivityResource>),
    Activity(BackgroundActivityResource),
    ActivityLogs(BackgroundActivityLogResource),
}
