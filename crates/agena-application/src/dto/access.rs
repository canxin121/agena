#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default)]
    pub workspace_id: Option<i64>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Result of a git commit.
pub struct GitCommitResource {
    pub commit: String,
    pub summary: String,
    pub status: GitStatusResource,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to create a pull request.
pub struct GitPullRequestCreateRequest {
    #[serde(default)]
    pub workspace_id: Option<i64>,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub head: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

use super::{Deserialize, Serialize};
use agena_api::resource::PermissionMode as ApiPermissionMode;
