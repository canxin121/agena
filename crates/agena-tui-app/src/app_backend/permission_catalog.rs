//! Permission tool catalog presentation.

use agena_application::Application;

/// A tool catalog item for the permission studio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionToolCatalogItem {
    pub name: String,
    pub summary: String,
    pub tags: Vec<String>,
}

/// The tool catalog shown by the permission studio. Synchronous: rendered
/// every time the permission studio opens.
pub(crate) fn permission_tool_catalog(application: &Application) -> Vec<PermissionToolCatalogItem> {
    application
        .plugin_runtime()
        .permission_tool_catalog()
        .into_iter()
        .map(|tool| PermissionToolCatalogItem {
            name: tool.name,
            summary: tool.summary,
            tags: tool.tags,
        })
        .collect()
}
