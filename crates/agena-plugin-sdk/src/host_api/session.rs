//! Session read/write surface for plugins.
//!
//! A plugin that serves a chat platform must be able to do what the built-in
//! web and TUI clients do: read a transcript, submit a message (including rich
//! `ComposerDocument` content), cancel work, manage the queue, answer a
//! permission card. These types are the plugin-facing shape of that surface.
//!
//! The host still owns authorization. A plugin receives a scoped client bound
//! to one plugin instance; every call is checked against the sessions that
//! instance is allowed to touch. A plugin cannot widen its own scope by
//! naming an id, and it cannot mint [`AuthorKind::InstanceOperator`] for an
//! external speaker — the host fills the author from the verified context.
//!
//! [`AuthorKind::InstanceOperator`]: agena_domain::AuthorKind::InstanceOperator

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One read request against a session.
///
/// `session_id` is resolved by the host: `None` means the callback's current
/// session, and an explicit id must be inside the plugin's authorized scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSessionQueryRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub query: HostSessionQuery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum HostSessionQuery {
    /// Sessions visible to this plugin, optionally within one workspace.
    List {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_id: Option<i64>,
        /// Case-insensitive substring match on the title.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        search: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
    /// One session's metadata.
    Get,
    /// Durable execution and queue state.
    State,
    /// An ordered transcript page. `after` is the last part id already seen;
    /// a `None` cursor reads from the newest end.
    Parts {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
    /// The text/JSON payload behind one content resource id.
    Content { resource_id: String },
    /// Attempts for one execution, oldest generation first.
    Executions,
    /// Token and cost accounting for this session.
    Cost,
    /// The fork/rewind tree rooted at `root_id`.
    Tree { root_id: i64 },
    /// The chat platforms bound to this session's workspace.
    Platforms,
    /// Pending permission and user-input requests.
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSessionQueryResponse {
    /// A serialized resource matching the query, so a plugin can decode only
    /// the variant it asked for without the SDK pinning every web resource.
    pub data: Value,
}

/// One write request against a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSessionCommandRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub command: HostSessionCommand,
    /// Stable key so a retried command after a lost acknowledgement does not
    /// submit the same message or mutation twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum HostSessionCommand {
    /// Submit user input. `document` carries text, attachments and inline-UI
    /// activity nodes, so a plugin can send a plain message or a rich one
    /// without any plugin-specific content type.
    Submit {
        document: agena_domain::ComposerDocument,
        /// Who is speaking. The host overrides this for external speakers it
        /// can verify; a plugin cannot claim `instance_operator`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        author: Option<agena_domain::MessageAuthor>,
        /// Where the message arrived from, for prompt attribution.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<agena_domain::MessageOrigin>,
    },
    /// Stop the current execution for this session.
    Cancel,
    /// Pause or resume the durable input queue.
    SetQueuePaused { paused: bool },
    /// Drop every queued input that has not started.
    ClearQueue,
    /// Remove one queued input, preserving nothing.
    RemoveQueued { execution_id: String },
    /// Take a queued input out of the queue and submit it immediately.
    TakeQueued { execution_id: String },
    /// Answer a pending permission request.
    ReplyPermission {
        request_id: String,
        /// `allow_once`, `allow_always`, `deny_once`, `deny_always` or
        /// `auto_approve`.
        decision: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// `session`, `workspace` or `global`; only used by the `*_always`
        /// decisions.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
    },
    /// Answer a pending user-input request.
    ReplyUserInput { request_id: String, answer: Value },
    /// Change the model or run options for later turns in this session.
    SetSelection { selection: Value },
    /// Rename the conversation.
    Rename { title: String },
    /// Branch this session at a stable boundary.
    Fork {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_part_id: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// Rewind this session's transcript to before `part_id`.
    Rewind { part_id: i64 },
    /// Compact the context now.
    Compact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSessionCommandResponse {
    /// The affected session, when the command produced or targeted one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    /// Ids of the executions this command accepted or stopped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub execution_ids: Vec<String>,
    /// A serialized result resource, when the command returns one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Which live stream to follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSessionStreamKind {
    /// Ordered transcript changes: parts added, updated or removed.
    Parts,
    /// Session metadata, executions and runtime signals.
    Notifications,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSessionStreamRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub kind: HostSessionStreamKind,
}

/// One live event delivered to a plugin stream.
///
/// `Lagged` is not an error: the plugin fell behind and must re-read the
/// transcript from its last cursor instead of assuming nothing changed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HostSessionEvent {
    /// A durable part was appended or replaced.
    Part { part: Value },
    /// A part was removed.
    PartRemoved { part_id: i64 },
    /// Session metadata or execution state changed.
    State { data: Value },
    /// The plugin missed `skipped` events and must re-read.
    Lagged { skipped: u64 },
    /// The stream ended (session deleted, scope revoked, plugin stopped).
    Closed { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_round_trips_text_and_identity() {
        let request = HostSessionCommandRequest {
            session_id: Some(7),
            command: HostSessionCommand::Submit {
                document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                    text: "hello".into(),
                }]),
                author: Some(agena_domain::MessageAuthor::instance_operator()),
                origin: Some(agena_domain::MessageOrigin::local(
                    agena_domain::LocalSurface::Api,
                    "7",
                )),
            },
            idempotency_key: Some("k1".into()),
        };
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(encoded["command"]["kind"], "submit");
        let decoded: HostSessionCommandRequest = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.idempotency_key.as_deref(), Some("k1"));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let value = serde_json::json!({
            "session_id": 1,
            "command": {"kind": "cancel"},
            "extra": true,
        });
        assert!(serde_json::from_value::<HostSessionCommandRequest>(value).is_err());
    }

    #[test]
    fn query_variants_carry_their_cursor() {
        let query = HostSessionQuery::Parts {
            after: Some(42),
            limit: Some(50),
        };
        let encoded = serde_json::to_value(&query).unwrap();
        assert_eq!(encoded["kind"], "parts");
        assert_eq!(encoded["after"], 42);
    }

    #[test]
    fn lagged_is_distinguishable_from_closed() {
        let lagged = serde_json::to_value(HostSessionEvent::Lagged { skipped: 3 }).unwrap();
        let closed = serde_json::to_value(HostSessionEvent::Closed {
            reason: "revoked".into(),
        })
        .unwrap();
        assert_eq!(lagged["event"], "lagged");
        assert_eq!(closed["event"], "closed");
    }
}
