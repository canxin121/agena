//! Typed live-event projections for presentation consumers.
//!
//! The generic Runtime event stream remains the transport/API boundary. This
//! parallel surface exists for consumers that need live transcript updates
//! without reconstructing a private Runtime event envelope from JSON.

use async_trait::async_trait;
#[derive(Debug, Clone)]
pub enum RuntimePresentationEventKind {
    TranscriptPatch(Box<agena_domain::TranscriptPatch>),
    /// A background activity started, updated, or finished. Carries the
    /// mutated activity so presentation consumers can refresh a management
    /// panel without a full persisted-state replay.
    ActivityChanged {
        activity: Box<agena_domain::BackgroundActivity>,
        reason: agena_domain::BackgroundActivityEventReason,
    },
    /// A session transition that has no incremental transcript projection but
    /// requires the presentation to reload persisted state.
    Refresh {
        force_refresh: bool,
    },
    /// A live activity-v2 event (detail delta, title change, state change,
    /// upsert, removal). Broadcast in memory only, never persisted.
    ActivityV2(Box<crate::activity::ActivityLiveEvent>),
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
