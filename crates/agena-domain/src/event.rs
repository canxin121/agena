use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use uuid::Uuid;

/// Fixed development envelope format (`1.0`).
pub const EVENT_ENVELOPE_SCHEMA_VERSION: u32 = 1;

/// Persistent event tags that create one visible transcript message.
/// Tool-call lifecycle events intentionally remain absent: they update parts
/// of an existing assistant message instead of creating another message.
pub const MESSAGE_CREATED_EVENT_KIND_TAGS: &[&str] = &[
    "user_message_appended",
    "assistant_message_finished",
    "system_notice_appended",
];

/// Stable, snake_case identifier for a concrete event kind variant.
pub type EventKindTag = SmolStr;

/// Trait implemented by concrete event kind enums so envelopes and filters can
/// match a kind without parsing JSON.
pub trait KindMatcher {
    fn tag(&self) -> EventKindTag;
}

/// Per-variant policy for whether an event belongs in durable history.
pub trait KindPersistence: KindMatcher {
    fn is_persistent(&self) -> bool;
}

/// Routing and observability metadata outside an event payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventMeta {
    pub id: Uuid,
    pub seq_global: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq_session: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<Uuid>,
    pub envelope_schema: u32,
}

/// Append-only envelope around a concrete event kind.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventEnvelope<K> {
    #[serde(flatten)]
    pub meta: EventMeta,
    #[serde(flatten)]
    pub kind: K,
}

impl<K> EventEnvelope<K>
where
    K: KindMatcher,
{
    pub fn tag(&self) -> EventKindTag {
        self.kind.tag()
    }
}

/// Subscription scope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventScope {
    #[default]
    Global,
    Workspace {
        workspace_id: i64,
    },
    Session {
        session_id: i64,
    },
}

impl EventScope {
    pub fn matches(&self, meta: &EventMeta) -> bool {
        match self {
            Self::Global => true,
            Self::Workspace { workspace_id } => meta.workspace_id == Some(*workspace_id),
            Self::Session { session_id } => meta.session_id == Some(*session_id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFilter {
    pub scope: EventScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<HashSet<EventKindTag>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_seq_global: Option<i64>,
}

impl EventFilter {
    pub fn new(scope: EventScope) -> Self {
        Self {
            scope,
            kinds: None,
            since_seq_global: None,
        }
    }

    pub fn matches_meta(&self, meta: &EventMeta) -> bool {
        self.scope.matches(meta)
            && self
                .since_seq_global
                .is_none_or(|since| meta.seq_global > since)
    }

    pub fn matches_kind(&self, tag: &EventKindTag) -> bool {
        self.kinds.as_ref().is_none_or(|set| set.contains(tag))
    }
}

#[cfg(test)]
mod tests {
    use super::{EventFilter, EventMeta, EventScope};
    use chrono::Utc;
    use uuid::Uuid;

    fn meta() -> EventMeta {
        EventMeta {
            id: Uuid::nil(),
            seq_global: 4,
            seq_session: Some(2),
            session_id: Some(7),
            workspace_id: Some(9),
            created_at: Utc::now(),
            causation_id: None,
            correlation_id: None,
            envelope_schema: 1,
        }
    }

    #[test]
    fn scope_and_resume_filter_match_metadata() {
        let mut filter = EventFilter::new(EventScope::Session { session_id: 7 });
        filter.since_seq_global = Some(3);
        assert!(filter.matches_meta(&meta()));

        filter.since_seq_global = Some(4);
        assert!(!filter.matches_meta(&meta()));
    }
}
