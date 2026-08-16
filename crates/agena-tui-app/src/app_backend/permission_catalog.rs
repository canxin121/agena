//! Permission tool catalog presentation.

/// A tool catalog item for the permission studio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionToolCatalogItem {
    pub name: String,
    pub summary: String,
    pub tags: Vec<String>,
}

/// The tool catalog shown by the permission studio. Synchronous: rendered
/// every time the permission studio opens.
///
pub(crate) fn permission_tool_catalog(
    application: &crate::TuiBackend,
) -> Vec<PermissionToolCatalogItem> {
    application
        .permission_tools()
        .into_iter()
        .map(|tool| PermissionToolCatalogItem {
            name: tool.name,
            summary: tool.summary,
            tags: tool.tags,
        })
        .collect()
}
