#[derive(Debug, Clone)]
/// Refresh signal of a session.
pub struct SessionRefresh {
    pub latest_event_seq: Option<i64>,
    pub event_count: usize,
    pub execution: Option<SessionExecutionResource>,
}

#[derive(Debug, Clone)]
/// A row in the inspector view.
pub struct InspectorRow {
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Effect of a plugin command.
pub enum PluginCommandEffect {
    None,
    Message(String),
    SubmitPrompt(String),
    OpenPluginWorkbench {
        plugin_id: String,
        tab: Option<String>,
    },
    OpenUrl(String),
}

#[derive(Debug, Clone)]
/// Permission studio state of a session.
pub struct SessionPermissionStudioState {
    pub session_id: i64,
    pub session_title: String,
    pub permission: agena_domain::PermissionConfig,
    pub effective_permission: agena_domain::PermissionConfig,
}
use crate::SessionExecutionResource;

#[derive(Debug, Clone, PartialEq, Eq)]
/// A tool catalog item for the permission studio.
pub struct PermissionToolCatalogItem {
    pub name: String,
    pub summary: String,
    pub tags: Vec<String>,
}
