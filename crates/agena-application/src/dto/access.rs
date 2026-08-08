#[derive(Debug, Clone, Serialize)]
/// A persisted permission rule.
pub struct PermissionRuleResource {
    pub id: i64,
    pub action_key: String,
    pub subject_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_access_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_port: Option<u16>,
    pub mode: agena_domain::PermissionMode,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<i64>,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
/// Git status of a workspace.
pub struct GitStatusResource {
    pub workspace_root: String,
    pub git_available: bool,
    pub repo: bool,
    pub gh_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ahead: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behind: Option<u64>,
    pub staged_files: u64,
    pub unstaged_files: u64,
    pub untracked_files: u64,
    pub changed_files: u64,
    pub clean: bool,
    pub snapshot_active_sessions: u64,
    pub snapshot_managed_dirs: u64,
}

#[derive(Debug, Clone, Serialize)]
/// Whether a snapshot backend is available and why.
pub struct SnapshotBackendSupportResource {
    pub backend: String,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
/// An active workspace snapshot.
pub struct ActiveSnapshotResource {
    pub session_id: i64,
    pub path: String,
    pub branch: String,
    pub backend: String,
    pub created_here: bool,
}

#[derive(Debug, Clone, Serialize)]
/// A managed workspace snapshot.
pub struct ManagedSnapshotResource {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    pub registered_with_git: bool,
    pub registered_with_rift: bool,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize)]
/// Snapshot backends, active and managed snapshots.
pub struct SnapshotStatusResource {
    pub workspace_root: String,
    pub session_runtime_available: bool,
    pub registry_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_backend: Option<String>,
    pub git: SnapshotBackendSupportResource,
    pub rift: SnapshotBackendSupportResource,
    pub active: Vec<ActiveSnapshotResource>,
    pub managed: Vec<ManagedSnapshotResource>,
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Request to stage paths in git.
pub struct GitStageRequest {
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to commit staged changes.
pub struct GitCommitRequest {
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
/// Result of a git commit.
pub struct GitCommitResource {
    pub commit: String,
    pub summary: String,
    pub status: GitStatusResource,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to create a pull request.
pub struct GitPullRequestCreateRequest {
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub head: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
/// A created pull request.
pub struct GitPullRequestResource {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to write a permission rule.
pub struct PermissionRuleWriteRequest {
    #[serde(default)]
    pub action_key: Option<String>,
    #[serde(default)]
    pub subject_kind: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub qualifier: Option<String>,
    #[serde(default)]
    pub path_access_kind: Option<String>,
    #[serde(default)]
    pub workspace_root: Option<String>,
    #[serde(default)]
    pub target_path: Option<String>,
    #[serde(default)]
    pub network_target: Option<String>,
    #[serde(default)]
    pub network_host: Option<String>,
    #[serde(default)]
    pub network_port: Option<u16>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub session_id: Option<i64>,
    pub mode: ApiPermissionMode,
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Request to revoke a permission rule.
pub struct PermissionRuleRevokeRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Query for a session event stream.
pub struct SessionEventStreamQuery {
    #[serde(default)]
    pub after_seq: Option<i64>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub poll_interval_ms: Option<u64>,
    #[serde(default)]
    pub idle_timeout_ms: Option<u64>,
}
use super::{DateTime, Deserialize, Serialize, Utc};
use agena_api::resource::PermissionMode as ApiPermissionMode;
