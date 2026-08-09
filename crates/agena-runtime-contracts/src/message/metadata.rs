use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

use agena_domain::MessageSource;

// Migration shim (R6-T1): `PartProviderState` now lives at the crate root
// in `provider_state.rs`. Re-exported here so the historical
// `message::PartProviderState` / `message::metadata::PartProviderState`
// paths keep resolving while consumers migrate (R6-T5/T6).
pub use crate::provider_state::PartProviderState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
/// Metadata attached to a message.
pub struct MessageMetadata {
    pub source: MessageSource,
    /// Optional caller-provided key used to make an externally scheduled or
    /// retried user message idempotent across process restarts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_turn_id: Option<i64>,
    /// The canonical conversation identity this message's run belongs to
    /// (design 19.5). v2 persists the UUID pair on the run marker and
    /// recovers it here so reply wake-up and reply-command matching
    /// (`signal_interaction_for_reply`) work after any reload; the marker
    /// part id is the `model_turn_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_turn_id: Option<agena_domain::TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_reply_id: Option<agena_domain::AssistantReplyId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_by_call_id: Option<i64>,
    /// True for a tool operation initiated directly by a trusted UI/API
    /// surface rather than emitted by a model turn. Permission replies execute
    /// the tool but must not start an unrelated model continuation.
    #[serde(default)]
    pub externally_initiated_tool: bool,
    pub model_provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_adapter_id: Option<String>,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_speed_mode: Option<String>,
}

impl Default for MessageMetadata {
    fn default() -> Self {
        Self {
            source: MessageSource::Assistant,
            idempotency_key: None,
            model_turn_id: None,
            conversation_turn_id: None,
            conversation_reply_id: None,
            parent_message_id: None,
            generated_by_call_id: None,
            externally_initiated_tool: false,
            model_provider_id: String::new(),
            model_adapter_id: None,
            model_id: String::new(),
            model_thinking_mode: None,
            model_speed_mode: None,
        }
    }
}
