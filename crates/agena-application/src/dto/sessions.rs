use super::{Deserialize, SearchPaginationQuery};

pub use agena_api::resource::{
    ActiveExecutionResource, ExecutionPhase, PendingInteractiveRequestResource,
    RunOptions as SessionRunOptionsRequest, SessionExecutionContextResource,
    SessionExecutionResource, SessionLifecycleState, SessionRelationKind, SessionResource,
    SessionUsageLimitBasis, SessionUsageResource, SubtaskStatus, WorkflowState,
};

#[derive(Debug, Clone, Deserialize, Default)]
/// Query for listing sessions.
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
/// Request to create a session in the session hierarchy.
pub struct SessionHierarchyRequest {
    pub title: String,
    #[serde(default)]
    pub parent_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// Request to update session metadata.
pub struct SessionUpdateRequest {
    pub title: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// Request to update the permission configuration of a session.
pub struct SessionPermissionUpdateRequest {
    pub permission: agena_api::resource::PermissionConfigResource,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to create a session.
pub struct SessionCreateRequest {
    pub workspace_id: i64,
    #[serde(flatten)]
    pub session: SessionHierarchyRequest,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to submit a message to a session.
pub struct SessionMessageRequest {
    #[serde(flatten)]
    pub run: SessionRunRequestBody,
    #[serde(default)]
    pub document: agena_domain::ComposerDocument,
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Body of a session run request.
pub struct SessionRunRequestBody {
    #[serde(flatten)]
    pub options: SessionRunOptionsRequest,
    #[allow(dead_code)]
    #[serde(flatten)]
    removed_agent_selection: RemovedAgentSelectionFields,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Default)]
struct RemovedAgentSelectionFields {
    #[serde(default, deserialize_with = "reject_removed_agent_selection")]
    agent_profile: (),
    #[serde(default, deserialize_with = "reject_removed_agent_selection")]
    profile: (),
    #[serde(default, deserialize_with = "reject_removed_agent_selection")]
    subagent_type: (),
}

fn reject_removed_agent_selection<'de, D>(deserializer: D) -> Result<(), D::Error>
where
    D: serde::Deserializer<'de>,
{
    let _ = serde::de::IgnoredAny::deserialize(deserializer)?;
    Err(<D::Error as serde::de::Error>::custom(
        "agent selection fields were removed; Agena has one fixed identity",
    ))
}

#[derive(Debug, Clone, Deserialize)]
/// Body of a session reply request.
pub struct SessionReplyRequestBody<T> {
    #[serde(flatten)]
    pub run: SessionRunRequestBody,
    pub reply: T,
}

#[derive(Debug, Clone, Deserialize)]
/// Body of a session rewind request.
pub struct SessionRewindRequestBody {
    pub turn_id: agena_domain::TurnId,
}

#[cfg(test)]
mod tests {
    use super::SessionMessageRequest;

    #[test]
    fn session_run_request_rejects_removed_agent_selection_fields() {
        let valid = serde_json::from_value::<SessionMessageRequest>(serde_json::json!({
            "temperature": 0.25,
            "document": [],
        }))
        .expect("known flattened run options and message fields must remain valid");
        assert_eq!(valid.run.options.temperature, Some(0.25));
        assert!(valid.document.is_empty());

        for field in ["agent_profile", "profile", "subagent_type"] {
            let mut request = serde_json::Map::new();
            request.insert(field.to_owned(), serde_json::json!("build"));
            let error =
                serde_json::from_value::<SessionMessageRequest>(serde_json::Value::Object(request))
                    .expect_err("removed agent selection must not be silently accepted");
            assert!(error.to_string().contains("one fixed identity"), "{error}");
        }
    }
}
