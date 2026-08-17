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

/// Presentation metadata for a folded assistant activity prefix. The prefix
/// is intentionally absent from `SessionPartsResource.parts`; clients use the
/// opaque cursor to request the next expansion chunk for the adjacent logical
/// assistant reply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionTranscriptFoldResource {
    pub run_id: i64,
    /// All adjacent assistant runs that make up the folded logical reply.
    /// `run_id` remains the run containing the first visible anchor for
    /// compact clients; expansion uses this complete set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run_ids: Vec<i64>,
    /// The first activity part currently present in `parts` for this fold.
    pub anchor_part_id: i64,
    pub hidden_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Current ordered part snapshot used for initial load and reconnect catch-up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionPartsResource {
    pub session_id: i64,
    /// Monotonic `sessions.version` high-water mark.
    pub version: i64,
    pub parts: Vec<PartResource>,
    /// Fold metadata is populated only by the presentation-oriented
    /// transcript endpoint. Raw `/parts` pages leave this empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folds: Vec<SessionTranscriptFoldResource>,
    /// Cursor metadata for newest-first keyset pagination. `parts` itself is
    /// returned chronologically so renderers can append it directly.
    pub page: crate::pagination::PageInfo,
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
        favorite: bool,
        pinned: bool,
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
