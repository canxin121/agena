use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceResource {
    pub id: i64,
    pub path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_count: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkspaceListQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub include_session_count: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceWriteRequest {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceResolveRequest {
    pub path: String,
    #[serde(default)]
    pub create_if_missing: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkspaceFileTreeQuery {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub depth: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceFileKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceFileNode {
    pub name: String,
    pub path: String,
    pub kind: WorkspaceFileKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<WorkspaceFileNode>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceFileTreeResource {
    pub workspace_id: i64,
    pub root: String,
    pub path: String,
    pub entries: Vec<WorkspaceFileNode>,
}
