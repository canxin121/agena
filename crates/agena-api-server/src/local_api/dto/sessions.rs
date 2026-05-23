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
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub workspace_id: Option<i64>,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub roots: bool,
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionCreateRequest {
    pub workspace_id: i64,
    pub title: String,
    #[serde(default)]
    pub parent_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionReplaceRequest {
    pub title: String,
    #[serde(default)]
    pub parent_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionMessageRequest {
    #[serde(flatten)]
    pub options: SessionRunOptionsRequest,
    #[serde(default)]
    pub parts: Vec<PartContent>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionContinueRequestBody {
    #[serde(flatten)]
    pub options: SessionRunOptionsRequest,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionPermissionReplyRequestBody {
    #[serde(flatten)]
    pub options: SessionRunOptionsRequest,
    pub reply: PermissionReply,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionUserInputReplyRequestBody {
    #[serde(flatten)]
    pub options: SessionRunOptionsRequest,
    pub reply: UserInputReply,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionRewindRequestBody {
    pub message_id: i64,
}
