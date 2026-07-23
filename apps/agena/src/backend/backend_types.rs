#[derive(Debug, Clone)]
pub struct SessionRefresh {
    pub latest_event_seq: Option<i64>,
    pub event_count: usize,
    pub execution: Option<SessionExecutionResource>,
    pub latest_messages: Option<PaginatedResponse<MessageResource>>,
}

#[derive(Debug, Clone)]
pub struct InspectorRow {
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginCommandEffect {
    None,
    Message(String),
    SubmitPrompt(String),
    OpenRoute(String),
    OpenUrl(String),
}

#[derive(Debug, Clone)]
pub struct SessionPermissionStudioState {
    pub session_id: i64,
    pub session_title: String,
    pub agent_name: Option<String>,
    pub agent_permission: Option<agena_domain::PermissionConfig>,
    pub permission: agena_domain::PermissionConfig,
    pub effective_permission: agena_domain::PermissionConfig,
}
use crate::backend::{MessageResource, PaginatedResponse, SessionExecutionResource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionToolCatalogItem {
    pub name: String,
    pub summary: String,
    pub tags: Vec<String>,
}
