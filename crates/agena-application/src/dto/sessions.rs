use super::{Deserialize, SearchPaginationQuery};

pub use agena_api::resource::{
    ActiveExecutionResource, AgentStatus, ExecutionPhase, PendingInteractiveRequestResource,
    RunOptions as SessionRunOptionsRequest, SessionExecutionContextResource,
    SessionExecutionResource, SessionLifecycleState, SessionRelationKind, SessionResource,
    SessionState, SessionUsageLimitBasis, SessionUsageResource, WorkflowState,
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
    /// Hide agent child sessions (`relation_kind = 'subagent'`).
    #[serde(default)]
    pub exclude_subagents: bool,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub bucket: Option<agena_api::queries::SessionListBucket>,
    #[serde(default)]
    pub include_total: bool,
    /// Return the count with no session rows or per-session state projection.
    #[serde(default)]
    pub count_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
/// Request to create a session in the session hierarchy.
pub struct SessionHierarchyRequest {
    pub title: String,
    #[serde(default)]
    pub auto_title: bool,
    #[serde(default)]
    pub parent_id: Option<i64>,
    /// None inherits; an empty string disables inherited additional instructions.
    #[serde(default)]
    pub additional_system_prompt: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// Request to update session metadata and additional instructions.
pub struct SessionUpdateRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub favorite: Option<bool>,
    #[serde(default)]
    pub pinned: Option<bool>,
    /// Resume automatic naming, or lock the current title. Takes effect on the
    /// next user turn; assigning a title alone always locks it.
    #[serde(default)]
    pub auto_title: Option<bool>,
    /// Omit to keep the current value, use null to inherit, or a string to override.
    #[serde(
        default,
        deserialize_with = "deserialize_additional_system_prompt_update"
    )]
    pub additional_system_prompt: Option<Option<String>>,
}

fn deserialize_additional_system_prompt_update<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
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

#[derive(Debug, Clone)]
/// Request to submit a message to a session.
pub struct SessionRunRequest {
    pub run: SessionRunRequestBody,
    pub document: agena_domain::ComposerDocument,
}

impl<'de> serde::Deserialize<'de> for SessionRunRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut object = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;
        let document = object
            .remove("document")
            .map(|value| {
                serde_json::from_value(value).map_err(<D::Error as serde::de::Error>::custom)
            })
            .transpose()?
            .unwrap_or_default();
        let options = serde_json::from_value(serde_json::Value::Object(object))
            .map_err(<D::Error as serde::de::Error>::custom)?;
        Ok(Self {
            run: SessionRunRequestBody { options },
            document,
        })
    }
}

#[derive(Debug, Clone, Default)]
/// Body of a session run request.
pub struct SessionRunRequestBody {
    pub options: SessionRunOptionsRequest,
}

impl<'de> serde::Deserialize<'de> for SessionRunRequestBody {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        SessionRunOptionsRequest::deserialize(deserializer).map(|options| Self { options })
    }
}

#[derive(Debug, Clone)]
/// Body of a session reply request.
pub struct SessionReplyRequestBody<T> {
    pub run: SessionRunRequestBody,
    pub reply: T,
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for SessionReplyRequestBody<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut object = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;
        let reply = object
            .remove("reply")
            .ok_or_else(|| <D::Error as serde::de::Error>::missing_field("reply"))?;
        let reply = T::deserialize(reply).map_err(<D::Error as serde::de::Error>::custom)?;
        let options = serde_json::from_value(serde_json::Value::Object(object))
            .map_err(<D::Error as serde::de::Error>::custom)?;
        Ok(Self {
            run: SessionRunRequestBody { options },
            reply,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// Body of a session rewind request.
pub struct SessionRewindRequestBody {
    pub at_message_id: i64,
}

#[cfg(test)]
mod additional_system_prompt_tests {
    use super::*;

    #[test]
    fn update_preserves_missing_null_empty_and_text() {
        let cases = [
            (serde_json::json!({}), None),
            (
                serde_json::json!({"additional_system_prompt": null}),
                Some(None),
            ),
            (
                serde_json::json!({"additional_system_prompt": ""}),
                Some(Some(String::new())),
            ),
            (
                serde_json::json!({"additional_system_prompt": "  Keep this spacing.\n"}),
                Some(Some("  Keep this spacing.\n".to_owned())),
            ),
        ];
        for (json, expected) in cases {
            let request: SessionUpdateRequest = serde_json::from_value(json).unwrap();
            assert_eq!(request.additional_system_prompt, expected);
        }
        assert!(
            serde_json::from_value::<SessionUpdateRequest>(
                serde_json::json!({"additional_system_prompt": false})
            )
            .is_err()
        );
    }

    #[test]
    fn creation_and_per_request_system_preserve_empty_strings() {
        let request: SessionCreateRequest = serde_json::from_value(serde_json::json!({
            "workspace_id": 1,
            "title": "Test",
            "additional_system_prompt": ""
        }))
        .unwrap();
        assert_eq!(
            request.session.additional_system_prompt.as_deref(),
            Some("")
        );
        let run: SessionRunRequestBody =
            serde_json::from_value(serde_json::json!({"system": ""})).unwrap();
        assert_eq!(run.options.system.as_deref(), Some(""));
    }
}
