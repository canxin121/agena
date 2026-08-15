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
/// The plugin permission-tool catalog is an in-process host DTO with no public
/// center endpoint, so remote client mode shows an empty catalog.
pub(crate) fn permission_tool_catalog(
    application: &crate::TuiBackend,
) -> Vec<PermissionToolCatalogItem> {
    let _ = application;
    Vec::new()
}
