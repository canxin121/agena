//! Typed live presentation updates for in-process consumers.
//!
//! Persisted session data arrives as facade part patches. Activity updates are
//! ephemeral. Neither path is a history/replay mechanism.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Routing and observability metadata outside a presentation update payload.
///
/// v2 has no event log, so these fields are derived from the operation that
/// produced the notification (part id / session version / created-at) rather
/// than a persisted envelope (design 14.3). Kept shape-compatible with v1 so
/// live subscribers can order and scope notifications without a full reload.
#[derive(Debug, Clone)]
pub struct RuntimePresentationEventMeta {
    pub id: Uuid,
    pub seq_global: i64,
    pub seq_session: Option<i64>,
    pub session_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub causation_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>,
    pub envelope_schema: u32,
}

#[derive(Debug, Clone)]
/// Kind of a runtime presentation update.
pub enum RuntimePresentationEventKind {
    /// A committed v2 part/meta patch from the sealed session facade. The TUI
    /// currently treats this as an incremental invalidation and reloads the
    /// marker-grouped transcript projection; the raw patch remains available
    /// to consumers that can apply it directly.
    PartPatch(Box<agena_storage::store::SessionChange>),
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
/// A presentation update delivered to live subscribers.
///
/// `durable` means the update reflects a committed part/meta mutation. Live
/// activity signals set it to false. Catch-up always reloads current parts;
/// these updates are never replayed.
pub struct RuntimePresentationEvent {
    pub meta: RuntimePresentationEventMeta,
    pub invalidates_ancestor_projection: bool,
    pub durable: bool,
    pub kind: RuntimePresentationEventKind,
}

#[derive(Debug, Clone)]
/// Item received on a live presentation subscription.
pub enum RuntimeLivePresentationSubscriptionItem {
    Event(Box<RuntimePresentationEvent>),
    Lagged(u64),
}

#[async_trait]
/// A live subscription to presentation updates.
pub trait RuntimeLivePresentationSubscription: Send {
    async fn recv(&mut self) -> Option<RuntimeLivePresentationSubscriptionItem>;
}
