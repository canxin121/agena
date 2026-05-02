//! Read-only queries. Closely mirrors REST GET endpoints but expressed as
//! `Query` enum so they can be invoked over both REST and WS.

use serde::{Deserialize, Serialize};

use crate::pagination::{PageInfo, PaginatedResponse};
use crate::resource::{
    HealthResponse, MessageResource, PartLoadMode, PermissionRuleResource, ProviderModelsResponse,
    ProviderSummaryResource, RuntimeStatusResponse, SessionResource, WorkspaceResource,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Query {
    Health,
    Runtime,
    ListProviders,
    ListProviderModels(ListProviderModelsParams),
    ListWorkspaces(ListWorkspacesParams),
    GetWorkspace(GetWorkspaceParams),
    ListSessions(ListSessionsParams),
    GetSession(GetSessionParams),
    GetSessionState(GetSessionParams),
    ListMessages(ListMessagesParams),
    GetMessage(GetMessageParams),
    ListEvents(ListEventsParams),
    ListPermissionRules(ListPermissionRulesParams),
    GetPermissionRule(GetPermissionRuleParams),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
pub enum QueryResult {
    Health(HealthResponse),
    Runtime(RuntimeStatusResponse),
    Providers(Vec<ProviderSummaryResource>),
    ProviderModels(ProviderModelsResponse),
    Workspaces(PaginatedResponse<WorkspaceResource>),
    Workspace(WorkspaceResource),
    Sessions(PaginatedResponse<SessionResource>),
    Session(SessionResource),
    SessionState(crate::resource::SessionExecutionResource),
    Messages(PaginatedResponse<MessageResource>),
    Message(MessageResource),
    Events(PaginatedEvents),
    PermissionRules(PaginatedResponse<PermissionRuleResource>),
    PermissionRule(PermissionRuleResource),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedEvents {
    pub items: Vec<crate::DomainEvent>,
    pub page: PageInfo,
}

// ─── params ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListProviderModelsParams {
    pub provider_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListWorkspacesParams {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub include_session_count: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetWorkspaceParams {
    pub workspace_id: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListSessionsParams {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub workspace_id: Option<i64>,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub roots: bool,
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSessionParams {
    pub session_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMessagesParams {
    pub session_id: i64,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub parts: PartLoadMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMessageParams {
    pub message_id: i64,
    #[serde(default)]
    pub parts: PartLoadMode,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListEventsParams {
    /// Defaults to global scope.
    #[serde(default = "default_scope")]
    pub scope: crate::Scope,
    #[serde(default)]
    pub kinds: Option<std::collections::HashSet<crate::EventKindTag>>,
    /// Cursor: events with `seq_global > since_seq_global` are returned.
    #[serde(default)]
    pub since_seq_global: Option<i64>,
    #[serde(default)]
    pub limit: Option<u64>,
}

fn default_scope() -> crate::Scope {
    crate::Scope::Global
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListPermissionRulesParams {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetPermissionRuleParams {
    pub rule_id: i64,
}
