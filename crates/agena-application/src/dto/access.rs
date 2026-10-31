pub use agena_api::resource::GitStatusResource;

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

pub use agena_api::resource::GitCommitResource;

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

pub use agena_api::resource::GitPullRequestResource;

#[derive(Debug, Clone, Deserialize, Default)]
/// Request to revoke a permission rule.
pub struct PermissionRuleRevokeRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to apply one operation to selected permission rules in one scope.
pub struct PermissionRuleBatchRequest {
    pub rule_ids: Vec<i64>,
    pub scope: String,
    #[serde(default)]
    pub session_id: Option<i64>,
    pub operation: PermissionRuleBatchOperation,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
/// Supported bulk permission-rule operations.
pub enum PermissionRuleBatchOperation {
    Delete,
    SetMode { mode: ApiPermissionMode },
}

#[derive(Debug, Clone, Serialize)]
/// Rules changed by a bulk operation and selected IDs that no longer matched its scope.
pub struct PermissionRuleBatchResponse {
    pub affected: Vec<agena_api::resource::PermissionRuleResource>,
    pub missing_ids: Vec<i64>,
}

use super::{Deserialize, Serialize};
use agena_api::resource::PermissionMode as ApiPermissionMode;
