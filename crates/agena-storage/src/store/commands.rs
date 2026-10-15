//! Commands and immutable receipts at the Part storage boundary.

use super::{NewPart, Part, PartDelta, RunOutcome, SessionMeta, SessionView};

/// A change prepared from a read snapshot. The revision check prevents a
/// detached working value from replacing facts committed in the meantime.
#[derive(Debug, Clone)]
pub struct PartUpdate {
    pub part_id: i64,
    pub expected_revision: i64,
    pub delta: PartDelta,
}

impl PartUpdate {
    pub fn new(part: &Part, delta: PartDelta) -> Self {
        Self {
            part_id: part.part_id,
            expected_revision: part.revision,
            delta,
        }
    }
}

/// Part changes are submitted as values; callers never borrow stored Parts
/// mutably. Every variant uses the facade's commit/cache/notification path.
#[derive(Debug)]
pub enum PartCommand {
    Update(Vec<PartUpdate>),
    SubmitUser {
        parts: Vec<NewPart>,
        idempotency_key: Option<String>,
        execution_id: Option<String>,
    },
    Append {
        run_id: i64,
        parts: Vec<NewPart>,
    },
    SettleBackground {
        run_id: i64,
        tool_part: Option<(i64, super::PartState, serde_json::Value)>,
        parts: Vec<NewPart>,
    },
    CompleteRun {
        run_id: i64,
        outcome: RunOutcome,
    },
}

/// An authoritative full or bounded snapshot. Only the sealed storage facade
/// can construct one. In particular, a Vec<Part> cannot be installed into a
/// runtime projection by labelling it as committed.
#[derive(Debug, Clone, PartialEq)]
pub struct PartSnapshot {
    meta: SessionMeta,
    parts: Vec<Part>,
    complete: bool,
}

impl PartSnapshot {
    pub(super) fn new(view: SessionView, complete: bool) -> Self {
        Self {
            meta: view.meta,
            parts: view.parts,
            complete,
        }
    }

    pub fn meta(&self) -> &SessionMeta {
        &self.meta
    }
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn into_rows(self) -> (SessionMeta, Vec<Part>, bool) {
        (self.meta, self.parts, self.complete)
    }
}

/// A successful command receipt, issued after commit and notification.
#[derive(Debug)]
pub struct PartCommit {
    pub(super) snapshot: PartSnapshot,
    pub(super) run_id: Option<i64>,
    pub(super) created: bool,
}

impl PartCommit {
    pub fn snapshot(&self) -> &PartSnapshot {
        &self.snapshot
    }
    pub fn run_id(&self) -> Option<i64> {
        self.run_id
    }
    pub fn created(&self) -> bool {
        self.created
    }
    pub fn into_snapshot(self) -> PartSnapshot {
        self.snapshot
    }
}
