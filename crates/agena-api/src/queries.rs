//! Read-only queries. Mostly mirrors REST GET endpoints, but also includes
//! parameterized read-only list operations that use POST on the HTTP surface.
//! Expressed as a `Query` enum so they can be invoked over both REST and WS.

use serde::{Deserialize, Serialize};

use crate::pagination::{PageInfo, PaginatedResponse};
use crate::resource::{
    HealthResponse, PermissionRuleResource, ProviderAdapterModelsResponse, ProviderModelsResponse,
    ProviderSummaryResource, RuntimeStatusResponse, SessionResource, WorkspaceResource,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Query {
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
    ListEvents(ListEventsParams),
    ListPermissionRules(ListPermissionRulesParams),
    GetPermissionRule(GetPermissionRuleParams),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
#[serde(tag = "result", content = "data", rename_all = "snake_case")]
pub enum QueryResult {
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
    Events(PaginatedEvents),
    PermissionRules(PaginatedResponse<PermissionRuleResource>),
    PermissionRule(PermissionRuleResource),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedEvents {
    pub items: Vec<crate::EventResource>,
    pub page: PageInfo,
}

// ─── params ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListProviderModelsParams {
    pub provider_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[cfg(test)]
mod provider_discovery_contract_tests {
    use super::{ListProviderAdapterModelsParams, ProviderProtocolPaths, ProviderSecretSource};

    #[test]
    fn discovery_params_have_a_self_owned_stable_wire_shape() {
        let params = ListProviderAdapterModelsParams {
            provider_id: Some("example".to_owned()),
            base_url: "https://models.example".to_owned(),
            protocol_paths: ProviderProtocolPaths {
                openai: "/openai".to_owned(),
                ..ProviderProtocolPaths::default()
            },
            api_key: Some(ProviderSecretSource::Env("EXAMPLE_API_KEY".to_owned())),
            adapter_ids: vec!["openai".to_owned()],
        };

        assert_eq!(
            serde_json::to_value(params).expect("serialize discovery params"),
            serde_json::json!({
                "provider_id": "example",
                "base_url": "https://models.example",
                "protocol_paths": {
                    "openai": "/openai",
                    "anthropic": "/v1",
                    "gemini": "/v1beta"
                },
                "api_key": { "kind": "env", "value": "EXAMPLE_API_KEY" },
                "adapter_ids": ["openai"]
            })
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListSavedProviderAdapterModelsParams {
    pub provider_id: String,
    #[serde(default)]
    pub adapter_ids: Vec<String>,
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
