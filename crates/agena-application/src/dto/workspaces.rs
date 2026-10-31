#[derive(Debug, Clone, Deserialize, Default)]
/// Query for listing workspaces.
pub struct WorkspaceListQuery {
    #[serde(default)]
    pub offset: u64,
    #[serde(flatten)]
    pub pagination: SearchPaginationQuery,
    #[serde(default)]
    pub include_session_count: bool,
}

#[derive(Debug, Clone, Deserialize)]
/// Request keyed by workspace path.
pub struct WorkspacePathRequest {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to resolve a workspace path, optionally creating it.
pub struct WorkspaceResolveRequest {
    #[serde(flatten)]
    pub workspace: WorkspacePathRequest,
    #[serde(default)]
    pub create_if_missing: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Query for a workspace file tree.
pub struct WorkspaceFileTreeQuery {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub depth: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
    /// Apply workspace ignore files and omit VCS metadata while walking.
    #[serde(default)]
    pub respect_ignores: bool,
}

#[derive(Debug, Clone, Deserialize)]
/// Query to download a workspace file.
pub struct WorkspaceFileDownloadQuery {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to upload a file into a workspace's managed uploads directory.
pub struct WorkspaceFileUploadRequest {
    pub filename: String,
    pub data_base64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

use super::{Deserialize, SearchPaginationQuery};
pub use agena_api::resource::{
    WorkspaceFileKind, WorkspaceFileNode, WorkspaceFileTreeResource, WorkspaceFileUploadResource,
};
