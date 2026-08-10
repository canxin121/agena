#[derive(Debug, Clone, Deserialize, Default)]
/// Query for listing workspaces.
pub struct WorkspaceListQuery {
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

#[derive(Debug, Clone, Serialize)]
/// A file uploaded into a workspace. `path` is relative to the workspace root
/// and is the value clients should reference as a workspace attachment.
pub struct WorkspaceFileUploadResource {
    pub workspace_id: i64,
    pub path: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of a workspace file entry.
pub enum WorkspaceFileKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize)]
/// A node in a workspace file tree.
pub struct WorkspaceFileNode {
    pub name: String,
    pub path: String,
    pub kind: WorkspaceFileKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<WorkspaceFileNode>,
}

#[derive(Debug, Clone, Serialize)]
/// A workspace file tree.
pub struct WorkspaceFileTreeResource {
    pub workspace_id: i64,
    pub root: String,
    pub path: String,
    pub entries: Vec<WorkspaceFileNode>,
}
use super::{DateTime, Deserialize, SearchPaginationQuery, Serialize, Utc};
