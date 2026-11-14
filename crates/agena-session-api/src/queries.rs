//! Read-only queries. Mostly mirrors REST GET endpoints, but also includes
//! parameterized read-only list operations that use POST on the HTTP surface.
//! Expressed as a `Query` enum so they can be invoked over both REST and WS.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
/// A validated read-only query sent to the runtime.
pub enum Query {
    /// Product operations shared by HTTP and plugin callers.
    Business(crate::business::BusinessQuery),
    Management(crate::management::ManagementQuery),
    ExtendedSession(crate::extended_sessions::SessionQuery),
    Server(crate::server_control::ServerQuery),
    ReadParts(ReadPartsParams),
    ReadRuns(ReadRunsParams),
    ReadContent(super::content::ReadContentParams),
    ReadContentText(super::content::ReadContentTextParams),
    Health,
    Runtime,
    ListProviders,
    ListProviderModels(ListProviderModelsParams),
    ListProviderAdapterModels(ListProviderAdapterModelsParams),
    ListSavedProviderAdapterModels(ListSavedProviderAdapterModelsParams),
    ListWorkspaces(ListWorkspacesParams),
    GetWorkspace(GetWorkspaceParams),
    ListSessions(ListSessionsParams),
    GetSession(GetSessionParams),
    GetSessionState(GetSessionParams),
    /// Token and cost accounting for one session.
    GetSessionCost(GetSessionParams),
    /// Every session sharing the tree root, in `(depth, id)` order.
    GetSessionTree(GetSessionParams),
    /// Chat platforms bound to the workspace of one session.
    GetSessionPlatforms(GetSessionParams),
    ListPermissionRules(ListPermissionRulesParams),
    GetPermissionRule(GetPermissionRuleParams),
    ListActivities(ListActivitiesParams),
    GetActivity(GetActivityParams),
    ActivityLogs(ActivityLogsParams),
}

pub use crate::business::BusinessQuery;
pub use crate::extended_sessions::SessionQuery;
pub use crate::management::ManagementQuery;

impl From<BusinessQuery> for Query {
    fn from(query: BusinessQuery) -> Self {
        Self::Business(query)
    }
}
impl From<ManagementQuery> for Query {
    fn from(query: ManagementQuery) -> Self {
        Self::Management(query)
    }
}
impl From<SessionQuery> for Query {
    fn from(query: SessionQuery) -> Self {
        Self::ExtendedSession(query)
    }
}

impl From<crate::server_control::ServerQuery> for Query {
    fn from(request: crate::server_control::ServerQuery) -> Self {
        Self::Server(request)
    }
}

// ─── params ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ReadPartsParams {
    pub session_id: i64,
    #[serde(default)]
    pub ids: Vec<i64>,
    #[serde(default)]
    pub run_ids: Vec<i64>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub sections: Vec<super::live::ToolDetailSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ReadRunsParams {
    pub session_id: i64,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub part_limit: Option<u64>,
    #[serde(default)]
    pub sections: Vec<super::live::ToolDetailSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for listing models of a provider.
pub struct ListProviderModelsParams {
    pub provider_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Parameters for probing adapter models from a provider endpoint.
pub struct ListProviderAdapterModelsParams {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub protocol_paths: ProviderProtocolPaths,
    #[serde(default)]
    pub api_key: Option<ProviderSecretSource>,
    #[serde(default)]
    pub adapter_ids: Vec<String>,
}

/// Protocol-specific paths used when discovering models from an ad-hoc
/// provider endpoint. This is a wire contract, intentionally independent of
/// the runtime configuration representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderProtocolPaths {
    pub openai: String,
    pub anthropic: String,
    pub gemini: String,
}

impl Default for ProviderProtocolPaths {
    fn default() -> Self {
        Self {
            openai: "/v1".to_owned(),
            anthropic: "/v1".to_owned(),
            gemini: "/v1beta".to_owned(),
        }
    }
}

/// A secret reference supplied for one discovery request. The server maps it
/// to its internal configuration only at the application boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ProviderSecretSource {
    Inline(String),
    Env(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Parameters for listing saved provider adapter models.
pub struct ListSavedProviderAdapterModelsParams {
    pub provider_id: String,
    #[serde(default)]
    pub adapter_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Parameters for listing workspaces.
pub struct ListWorkspacesParams {
    #[serde(default)]
    pub offset: u64,
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
/// Parameters for fetching a workspace by id.
pub struct GetWorkspaceParams {
    pub workspace_id: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionListBucket {
    Pinned,
    Favorite,
    Running,
    Attention,
    Recent,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Parameters for listing sessions.
pub struct ListSessionsParams {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub workspace_id: Option<i64>,
    /// Restrict to conversations owned by this Bot (UUID).
    #[serde(default)]
    pub owner_bot_id: Option<uuid::Uuid>,
    /// Restrict to one entrance shape: `local`, `direct`, `group`, `broadcast`.
    #[serde(default)]
    pub owner_surface: Option<agena_domain::SessionSurface>,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub roots: bool,
    /// Hide agent child sessions (`relation_kind = 'subagent'`).
    #[serde(default)]
    pub exclude_subagents: bool,
    #[serde(default)]
    pub search: Option<String>,
    /// Numbered pages may use an offset instead of a cursor.
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub bucket: Option<SessionListBucket>,
    #[serde(default)]
    pub include_total: bool,
    /// Return only the matching count without loading session resources.
    #[serde(default)]
    pub count_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for fetching a session by id.
pub struct GetSessionParams {
    pub session_id: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Parameters for listing permission rules.
pub struct ListPermissionRulesParams {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for fetching a permission rule by id.
pub struct GetPermissionRuleParams {
    pub rule_id: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Parameters for listing background activities.
pub struct ListActivitiesParams {
    /// Comma-separated kind filters (`shell`, `agent`, `runtime`, `browser`).
    #[serde(default)]
    pub kinds: Option<String>,
    /// Comma-separated status filters (`running`, `succeeded`, `failed`, …).
    #[serde(default)]
    pub statuses: Option<String>,
    /// Only include activities for this session (owns or parent).
    #[serde(default)]
    pub session_id: Option<i64>,
    /// Only include active (pending/running) activities.
    #[serde(default)]
    pub active_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Parameters for fetching a background activity by id.
pub struct GetActivityParams {
    pub activity_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Parameters for reading activity log lines.
pub struct ActivityLogsParams {
    pub activity_id: String,
    /// Cursor: lines with `seq > since_seq` are returned.
    #[serde(default)]
    pub since_seq: u64,
    /// Max lines to return (clamped server-side).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Block for fresh output up to this many ms when no new lines exist.
    #[serde(default)]
    pub wait_ms: u64,
}

impl Query {
    /// The session this read targets, when it is session-scoped.
    ///
    /// Transports use this to fence reach: a plugin may only name the session
    /// its callback belongs to. Declaring it beside the enum means a new
    /// variant cannot silently escape that check.
    pub fn session_id(&self) -> Option<i64> {
        match self {
            Query::Business(query) => query.session_id(),
            Query::Management(query) => query.session_id(),
            Query::ExtendedSession(query) => query.session_id(),
            Query::Server(_) => None,
            Query::Health
            | Query::Runtime
            | Query::ListProviders
            | Query::ListProviderModels(_)
            | Query::ListProviderAdapterModels(_)
            | Query::ListSavedProviderAdapterModels(_)
            | Query::ListWorkspaces(_)
            | Query::GetWorkspace(_)
            | Query::ListSessions(_)
            | Query::ListPermissionRules(_)
            | Query::GetPermissionRule(_)
            | Query::ListActivities(_)
            | Query::GetActivity(_)
            | Query::ActivityLogs(_) => None,
            Query::ReadParts(params) => Some(params.session_id),
            Query::ReadRuns(params) => Some(params.session_id),
            Query::ReadContent(params) => Some(params.session_id),
            Query::ReadContentText(params) => Some(params.session_id),
            Query::GetSession(params)
            | Query::GetSessionState(params)
            | Query::GetSessionCost(params)
            | Query::GetSessionTree(params)
            | Query::GetSessionPlatforms(params) => Some(params.session_id),
        }
    }
}

#[cfg(test)]
mod adapter_probe_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adapter_probe_preserves_defaults_partial_paths_and_secret_tags() {
        let request: ListProviderAdapterModelsParams =
            serde_json::from_value(json!({"base_url":"https://example.test"})).unwrap();
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            json!({
                "provider_id":null, "base_url":"https://example.test",
                "protocol_paths":{"openai":"/v1","anthropic":"/v1","gemini":"/v1beta"},
                "api_key":null, "adapter_ids":[]
            })
        );
        let request: ListProviderAdapterModelsParams = serde_json::from_value(json!({
            "base_url":"https://example.test", "protocol_paths":{"openai":"/custom"},
            "api_key":{"kind":"env","value":"API_KEY"}, "adapter_ids":["anthropic"]
        }))
        .unwrap();
        assert_eq!(request.protocol_paths.openai, "/custom");
        assert_eq!(request.protocol_paths.anthropic, "/v1");
        assert_eq!(request.protocol_paths.gemini, "/v1beta");
        assert!(
            matches!(request.api_key, Some(ProviderSecretSource::Env(ref name)) if name == "API_KEY")
        );
        for invalid in [
            json!({}),
            json!({"base_url":"x", "adapter_ids":null}),
            json!({"base_url":"x", "api_key":{"kind":"unknown","value":"x"}}),
        ] {
            assert!(serde_json::from_value::<ListProviderAdapterModelsParams>(invalid).is_err());
        }
    }
}
