//! Stable persisted-event projections for presentation/query consumers.
//!
//! The concrete event enum and its storage adapter stay behind the Runtime
//! boundary. Consumers receive the same envelope shape as the public event
//! protocol without naming that enum as a generic parameter.

use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq)]
/// A runtime event.
pub struct RuntimeEvent {
    pub meta: agena_domain::EventMeta,
    pub kind: String,
    pub payload: serde_json::Value,
    pub invalidates_ancestor_projection: bool,
}

/// Presentation-ready event data for local timeline consumers.
///
/// Unlike [`RuntimeEvent`], this projection intentionally has no JSON payload
/// that a UI would need to deserialize into a concrete Runtime event enum. The
/// concrete adapter formats its event once into stable display fields.
#[derive(Debug, Clone, PartialEq)]
/// An event in a runtime timeline.
pub struct RuntimeTimelineEvent {
    pub meta: agena_domain::EventMeta,
    pub kind: String,
    pub type_key: String,
    pub summary: String,
    pub detail_lines: Vec<RuntimeTimelineDetailLine>,
    pub search_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A detail line of a timeline event.
pub struct RuntimeTimelineDetailLine {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A range of runtime events.
pub struct RuntimeEventRange {
    pub after_seq_global: i64,
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A reverse range of runtime events.
pub struct RuntimeReverseEventRange {
    pub before_seq_global: Option<i64>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Error of a runtime event query.
pub struct RuntimeEventQueryError {
    pub failure: Box<agena_failure::Failure>,
}

impl RuntimeEventQueryError {
    pub fn internal(diagnostic: impl std::fmt::Display) -> Self {
        Self {
            failure: Box::new(crate::service_failure::unexpected_service_failure(
                "event.query_failed",
                "Event history could not be loaded.",
                diagnostic,
            )),
        }
    }
}

impl std::fmt::Display for RuntimeEventQueryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::service_failure::display_service_failure(&self.failure, formatter)
    }
}

impl std::error::Error for RuntimeEventQueryError {}

#[async_trait]
/// Service that queries runtime events.
pub trait RuntimeEventQueryService: Send + Sync {
    async fn list_events(
        &self,
        filter: &agena_domain::EventFilter,
        range: RuntimeEventRange,
    ) -> Result<Vec<RuntimeEvent>, RuntimeEventQueryError>;

    async fn list_events_before(
        &self,
        filter: &agena_domain::EventFilter,
        range: RuntimeReverseEventRange,
    ) -> Result<Vec<RuntimeEvent>, RuntimeEventQueryError>;

    /// A typed local-presentation projection. Generic event protocol clients
    /// continue to use `list_events*`; terminal consumers use this method and
    /// therefore never rebuild a private `DomainEvent` from JSON.
    async fn list_timeline_events_before(
        &self,
        filter: &agena_domain::EventFilter,
        range: RuntimeReverseEventRange,
    ) -> Result<Vec<RuntimeTimelineEvent>, RuntimeEventQueryError>;
}

#[derive(Debug, Clone, PartialEq)]
/// Item received on a live event subscription.
pub enum RuntimeLiveEventSubscriptionItem {
    Event(RuntimeEvent),
    Lagged(u64),
}

#[async_trait]
/// A live subscription to runtime events.
pub trait RuntimeLiveEventSubscription: Send {
    async fn recv(&mut self) -> Option<RuntimeLiveEventSubscriptionItem>;
}

/// Stable live-event subscription boundary. The concrete broadcast bus and
/// its private event enum remain adapter details.
pub trait RuntimeEventStreamService: Send + Sync {
    fn subscribe_events(
        &self,
        filter: agena_domain::EventFilter,
    ) -> Box<dyn RuntimeLiveEventSubscription>;

    /// Optional typed presentation projection. Transport consumers retain the
    /// generic subscription above; UI consumers can opt into this surface to
    /// avoid reconstructing legacy event enums from JSON.
    fn subscribe_presentation_events(
        &self,
        _filter: agena_domain::EventFilter,
    ) -> Option<Box<dyn crate::RuntimeLivePresentationSubscription>> {
        None
    }
}
