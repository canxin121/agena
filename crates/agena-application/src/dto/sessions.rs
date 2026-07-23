pub use agena_api::resource::{
    ActiveExecutionResource, ExecutionPhase, PendingInteractiveRequestResource,
    RunOptions as SessionRunOptionsRequest, SessionExecutionContextResource,
    SessionExecutionResource, SessionLifecycleState, SessionRelationKind, SessionResource,
    SessionUsageLimitBasis, SessionUsageResource, SubtaskStatus, WorkflowState,
};

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
#[serde(deny_unknown_fields)]
pub struct SessionUpdateRequest {
    pub title: String,
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
    pub parts: Vec<MessagePartContent>,
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
use super::{Deserialize, MessagePartContent, SearchPaginationQuery};
