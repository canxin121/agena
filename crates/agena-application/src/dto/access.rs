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

pub use agena_api::management::{PermissionRuleBatchOperation, PermissionRuleBatchRequest};

#[derive(Debug, Clone, Serialize)]
/// Rules changed by a bulk operation and selected IDs that no longer matched its scope.
pub struct PermissionRuleBatchResponse {
    pub affected: Vec<agena_api::resource::PermissionRuleResource>,
    pub missing_ids: Vec<i64>,
}

use super::{Deserialize, Serialize};
