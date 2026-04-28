use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::envelope::EventMeta;

/// Stable, snake_case identifier for a concrete event kind variant.
///
/// We use [`SmolStr`] rather than a closed enum so that [`agena_event`] does
/// not need to know every kind that downstream crates might add. The agena
/// core crate provides a `pub const TAGS: &[EventKindTag]` table for typed
/// access.
pub type EventKindTag = SmolStr;

/// Trait implemented by concrete `EventKind` enums so the bus can filter by
/// kind without parsing JSON.
pub trait KindMatcher {
    fn tag(&self) -> EventKindTag;
}

/// Subscription scope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    #[default]
    Global,
    Workspace { workspace_id: i64 },
    Session { session_id: i64 },
}

impl Scope {
    pub fn matches(&self, meta: &EventMeta) -> bool {
        match self {
            Scope::Global => true,
            Scope::Workspace { workspace_id } => meta.workspace_id == Some(*workspace_id),
            Scope::Session { session_id } => meta.session_id == Some(*session_id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFilter {
    pub scope: Scope,
    /// `None` means "all kinds"; otherwise the event's tag must be in the set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<HashSet<EventKindTag>>,
    /// Resume from a previous position. Subscribers see only events whose
    /// `seq_global > since_seq_global`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_seq_global: Option<i64>,
}

impl EventFilter {
    pub fn new(scope: Scope) -> Self {
        Self {
            scope,
            kinds: None,
            since_seq_global: None,
        }
    }

    pub fn with_kinds<I, S>(mut self, kinds: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<EventKindTag>,
    {
        self.kinds = Some(kinds.into_iter().map(Into::into).collect());
        self
    }

    pub fn since(mut self, seq_global: i64) -> Self {
        self.since_seq_global = Some(seq_global);
        self
    }

    pub fn matches_meta(&self, meta: &EventMeta) -> bool {
        if !self.scope.matches(meta) {
            return false;
        }
        if let Some(since) = self.since_seq_global
            && meta.seq_global <= since
        {
            return false;
        }
        true
    }

    pub fn matches_kind(&self, tag: &EventKindTag) -> bool {
        match &self.kinds {
            None => true,
            Some(set) => set.contains(tag),
        }
    }
}

// Re-export SmolStr for downstream crates that build TAGS tables.
pub use smol_str;
