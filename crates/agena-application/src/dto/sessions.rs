use super::{Deserialize, SearchPaginationQuery};

pub use agena_api::resource::{
    ActiveExecutionResource, ExecutionPhase, PendingInteractiveRequestResource,
    RunOptions as SessionRunOptionsRequest, SessionExecutionContextResource,
    SessionExecutionResource, SessionLifecycleState, SessionRelationKind, SessionResource,
    SessionState, SessionUsageLimitBasis, SessionUsageResource, SubtaskStatus, WorkflowState,
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
    /// Hide task child sessions (`relation_kind = 'subagent'`).
    #[serde(default)]
    pub exclude_subagents: bool,
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
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub favorite: Option<bool>,
    #[serde(default)]
    pub pinned: Option<bool>,
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
/// Body of a session rewind request.
pub struct SessionRewindRequestBody {
    pub turn_id: agena_domain::TurnId,
}

#[cfg(test)]
mod tests {
    use super::{
        SessionReplyRequestBody, SessionRunRequest, SessionRunRequestBody, SessionUpdateRequest,
    };

    #[test]
    fn session_update_accepts_flag_only_metadata_patches() {
        let request = serde_json::from_value::<SessionUpdateRequest>(serde_json::json!({
            "favorite": true,
            "pinned": false,
        }))
        .expect("deserialize metadata flags");
        assert_eq!(request.title, None);
        assert_eq!(request.favorite, Some(true));
        assert_eq!(request.pinned, Some(false));
    }

    #[test]
    fn session_run_request_rejects_unknown_fields() {
        let valid = serde_json::from_value::<SessionRunRequest>(serde_json::json!({
            "temperature": 0.25,
            "document": [],
        }))
        .expect("known flattened run options and message fields must remain valid");
        assert_eq!(valid.run.options.temperature, Some(0.25));
        assert!(valid.document.is_empty());

        let error = serde_json::from_value::<SessionRunRequest>(serde_json::json!({
            "obsolete": true,
            "document": []
        }))
        .expect_err("unknown run fields must not be silently accepted");
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn standalone_run_body_rejects_unknown_fields() {
        let valid = serde_json::from_value::<SessionRunRequestBody>(serde_json::json!({
            "temperature": 0.25,
            "parallel_tool_calls": false,
        }))
        .expect("current run options must decode");
        assert_eq!(valid.options.temperature, Some(0.25));
        assert_eq!(valid.options.parallel_tool_calls, Some(false));
        let error = serde_json::from_value::<SessionRunRequestBody>(serde_json::json!({
            "obsolete": true
        }))
        .expect_err("unknown standalone run options must be rejected");
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn reply_envelope_keeps_strict_run_options() {
        type Reply = SessionReplyRequestBody<serde_json::Value>;
        let valid = serde_json::from_value::<Reply>(serde_json::json!({
            "reply": {"approved": true},
            "temperature": 0.25,
        }))
        .expect("reply and current run options must decode together");
        assert_eq!(valid.run.options.temperature, Some(0.25));
        assert_eq!(valid.reply, serde_json::json!({"approved": true}));
        let error = serde_json::from_value::<Reply>(serde_json::json!({
            "reply": {"approved": true},
            "obsolete": true,
        }))
        .expect_err("reply envelope must not hide unknown run options");
        assert!(error.to_string().contains("unknown field"), "{error}");
    }
}
