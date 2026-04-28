use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::event::filter::{EventKindTag, KindMatcher};

/// Wire-format version of the envelope itself. Bumped only when envelope
/// fields (not payload schemas) change incompatibly.
pub const ENVELOPE_SCHEMA_VERSION: u32 = 1;

/// Routing / observability metadata that lives outside the payload.
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
    #[serde(default = "default_envelope_schema")]
    pub envelope_schema: u32,
}

fn default_envelope_schema() -> u32 {
    ENVELOPE_SCHEMA_VERSION
}

/// Append-only envelope around a concrete kind enum.
///
/// `K` is expected to be a `serde`-tagged enum (e.g.
/// `#[serde(tag = "kind", content = "payload")]`) implementing [`KindMatcher`]
/// so filters can match without forcing a JSON round-trip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DomainEvent<K> {
    #[serde(flatten)]
    pub meta: EventMeta,
    #[serde(flatten)]
    pub kind: K,
}

impl<K> DomainEvent<K>
where
    K: KindMatcher,
{
    pub fn tag(&self) -> EventKindTag {
        self.kind.tag()
    }
}
