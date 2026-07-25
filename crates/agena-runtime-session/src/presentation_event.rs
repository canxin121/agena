//! Typed live-event projections for presentation consumers.
//!
//! The generic Runtime event stream remains the transport/API boundary. This
//! parallel surface exists for consumers that need live transcript updates
//! without reconstructing a private Runtime event envelope from JSON.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::SessionProjectedMessagePart;

#[derive(Debug, Clone)]
pub struct RuntimeMessageMetadata {
    pub source: agena_domain::MessageSource,
    pub idempotency_key: Option<String>,
    pub turn_id: Option<i64>,
    pub parent_message_id: Option<i64>,
    pub generated_by_call_id: Option<i64>,
    pub model_provider_id: String,
    pub model_adapter_id: Option<String>,
    pub model_id: String,
    pub model_thinking_mode: Option<String>,
    pub model_speed_mode: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeMessagePartCheckpoint {
    pub session_id: i64,
    pub execution_id: Option<agena_domain::ExecutionId>,
    pub run_id: Option<agena_domain::RunId>,
    pub message_id: i64,
    pub message_role: agena_domain::Role,
    pub message_state: agena_domain::ExecutionStatus,
    pub message_created_at: DateTime<Utc>,
    pub message_metadata: RuntimeMessageMetadata,
    pub part: SessionProjectedMessagePart,
    pub ts_ms: i64,
}

#[derive(Debug, Clone)]
pub enum RuntimePresentationEventKind {
    MessagePartCheckpointed(Box<RuntimeMessagePartCheckpoint>),
    MessagePartDelta(agena_domain::MessagePartDeltaEvent),
    UserMessageAppended {
        message_id: i64,
    },
    AssistantMessageFinished,
    /// A session transition that has no incremental transcript projection but
    /// requires the presentation to reload persisted state.
    Refresh {
        force_refresh: bool,
    },
}

#[derive(Debug, Clone)]
pub struct RuntimePresentationEvent {
    pub meta: agena_domain::EventMeta,
    pub invalidates_ancestor_projection: bool,
    pub kind: RuntimePresentationEventKind,
}

#[derive(Debug, Clone)]
pub enum RuntimeLivePresentationSubscriptionItem {
    Event(Box<RuntimePresentationEvent>),
    Lagged(u64),
}

#[async_trait]
pub trait RuntimeLivePresentationSubscription: Send {
    async fn recv(&mut self) -> Option<RuntimeLivePresentationSubscriptionItem>;
}
