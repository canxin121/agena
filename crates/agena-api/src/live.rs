//! Wire contract for the v2 live-update surface.
//!
//! Persisted session history is an ordered list of parts. Live delivery is a
//! best-effort stream of part patches plus ephemeral runtime signals; neither
//! value is persisted or replayed.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One persisted part as exposed by the public API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PartResource {
    pub part_id: i64,
    pub kind: String,
    pub role: String,
    pub state: String,
    pub content: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub visibility: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered_markdown: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_part_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<i64>,
    pub origin_session_id: i64,
    pub revision: i64,
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<Value>,
}

/// Current ordered part snapshot used for initial load and reconnect catch-up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionPartsResource {
    pub session_id: i64,
    /// Monotonic `sessions.version` high-water mark.
    pub version: i64,
    pub parts: Vec<PartResource>,
}

/// A committed session mutation. This is observer notification only: it is
/// never persisted, replayed, or assigned a global sequence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionChangeResource {
    PartAdded {
        session_id: i64,
        part: Box<PartResource>,
    },
    PartUpdated {
        session_id: i64,
        part: Box<PartResource>,
    },
    PartRemoved {
        session_id: i64,
        part_id: i64,
    },
    SessionMetaUpdated {
        session_id: i64,
        version: i64,
        title: String,
        updated_at_ms: i64,
    },
}

impl SessionChangeResource {
    pub fn session_id(&self) -> i64 {
        match self {
            Self::PartAdded { session_id, .. }
            | Self::PartUpdated { session_id, .. }
            | Self::PartRemoved { session_id, .. }
            | Self::SessionMetaUpdated { session_id, .. } => *session_id,
        }
    }
}

/// Ephemeral, non-session runtime signal. `kind` is open-ended; `payload`
/// contains the signal-specific current value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSignalResource {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub payload: Value,
}
