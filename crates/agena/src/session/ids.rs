//! Strong-typed identifiers for session/history domain values.
//!
//! These newtypes prevent the historical mistake of passing `i64` / `String`
//! around as ad-hoc business keys. They serialize transparently so the
//! on-disk and wire formats are unchanged.

use std::borrow::Borrow;

use derive_more::{Display, From, Into};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use uuid::Uuid;

/// Stable identifier of a single message inside a session.
#[derive(
    Debug,
    Clone,
    Copy,
    Eq,
    PartialEq,
    Hash,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    Display,
    From,
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
    Debug,
    Clone,
    Copy,
    Eq,
    PartialEq,
    Hash,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    Display,
    From,
    Into,
)]
#[serde(transparent)]
pub struct PartId(pub i64);

impl PartId {
    pub const fn raw(self) -> i64 {
        self.0
    }
}

/// Stable identity of one user-visible execution of a session.
///
/// An execution spans the complete model/tool loop started by one command.
/// Individual provider calls within the execution use [`RunId`]. Keeping the
/// two identities separate prevents a stopped worker, a suspended permission
/// request, and a model attempt from being mistaken for the same lifecycle.
#[derive(
    Debug,
    Clone,
    Copy,
    Eq,
    PartialEq,
    Hash,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    Display,
    From,
    Into,
)]
#[serde(transparent)]
pub struct ExecutionId(pub Uuid);

impl ExecutionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ExecutionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Identifier of a single LLM attempt (request/response cycle).
///
/// All append-only history events emitted as part of one LLM call carry the
/// same `RunId`. The enclosing user-visible lifecycle is identified by an
/// [`ExecutionId`].
#[derive(
    Debug,
    Clone,
    Copy,
    Eq,
    PartialEq,
    Hash,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
    Display,
    From,
    Into,
)]
#[serde(transparent)]
pub struct RunId(pub Uuid);

impl RunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool call identifier as supplied by the upstream provider.
///
/// Backed by `SmolStr` so the common short-id case stays inline and cheap to
/// clone — these IDs flow through every projection.
#[derive(
    Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize, Display, From,
)]
#[serde(transparent)]
pub struct ToolCallId(pub SmolStr);

impl ToolCallId {
    pub fn new(id: impl Into<SmolStr>) -> Self {
        Self(id.into())
    }
}

impl Borrow<str> for ToolCallId {
    fn borrow(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for ToolCallId {
    fn as_ref(&self) -> &str {
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
