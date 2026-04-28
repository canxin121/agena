//! Strong-typed identifiers for session/history domain values.
//!
//! These newtypes prevent the historical mistake of passing `i64` / `String`
//! around as ad-hoc business keys. They serialize transparently so the
//! on-disk and wire formats are unchanged.

use derive_more::{Display, From, Into};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use uuid::Uuid;

/// Stable identifier of a single message inside a session.
#[derive(
    Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, Display, From,
    Into,
)]
#[serde(transparent)]
pub struct MessageId(pub i64);

impl MessageId {
    pub const fn raw(self) -> i64 {
        self.0
    }
}

/// Stable identifier of a single message part (chunk) inside a message.
#[derive(
    Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, Display, From,
    Into,
)]
#[serde(transparent)]
pub struct PartId(pub i64);

impl PartId {
    pub const fn raw(self) -> i64 {
        self.0
    }
}

/// Identifier of a single LLM "turn" (request/response cycle).
///
/// All append-only history events emitted as part of one LLM call carry the
/// same `TurnId`. A turn that is never closed by a `TurnCompleted` /
/// `TurnAborted` marker is treated as in-flight on load and discarded by
/// projection.
#[derive(
    Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, Display, From,
    Into,
)]
#[serde(transparent)]
pub struct TurnId(pub Uuid);

impl TurnId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TurnId {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool call identifier as supplied by the upstream provider.
///
/// Backed by `SmolStr` so the common short-id case stays inline and cheap to
/// clone — these IDs flow through every projection.
#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, Display, From)]
#[serde(transparent)]
pub struct ToolCallId(pub SmolStr);

impl ToolCallId {
    pub fn new(id: impl Into<SmolStr>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<&str> for ToolCallId {
    fn from(value: &str) -> Self {
        Self(SmolStr::new(value))
    }
}

impl From<String> for ToolCallId {
    fn from(value: String) -> Self {
        Self(SmolStr::new(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_id_round_trip_serde() {
        let id = MessageId(42);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "42");
        let back: MessageId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn turn_id_round_trip_serde() {
        let id = TurnId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: TurnId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn tool_call_id_accepts_str_and_string() {
        let a: ToolCallId = "call_abc".into();
        let b: ToolCallId = String::from("call_abc").into();
        assert_eq!(a, b);
        assert_eq!(a.as_str(), "call_abc");
    }
}
