use super::*;

pub use agena_api::resource::{
    RunOptions as SessionRunOptionsRequest, SessionExecutionContextResource,
    SessionExecutionResource, SessionGoalResource, SessionResource, SessionRunState,
    SessionUsageLimitBasis, SessionUsageResource,
};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionGoalSetRequest {
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub status: Option<GoalStatus>,
    #[serde(default)]
    pub clear: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionListQuery {
    #[serde(flatten)]
    pub pagination: SearchPaginationQuery,
    #[serde(default)]
    pub workspace_id: Option<i64>,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub roots: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionHierarchyRequest {
    pub title: String,
    #[serde(default)]
    pub parent_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionCreateRequest {
    pub workspace_id: i64,
    #[serde(flatten)]
    pub session: SessionHierarchyRequest,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionMessageRequest {
    #[serde(flatten)]
    pub run: SessionRunRequestBody,
    #[serde(default)]
    pub parts: Vec<PartContent>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionRunRequestBody {
    #[serde(flatten)]
    pub options: SessionRunOptionsRequest,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionReplyRequestBody<T> {
    #[serde(flatten)]
    pub run: SessionRunRequestBody,
    pub reply: T,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionRewindRequestBody {
    pub message_id: i64,
}
