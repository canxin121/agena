//! In-flight interrupt-and-send documents, keyed by their draft slot.
//!
//! A send always leaves as one request carrying the whole multi-part document,
//! so nothing is parked for a later turn. The only reason a document waits
//! here is that the runtime refuses a new user run while another one is live:
//! sending during an active generation cancels that run first and submits the
//! captured document as soon as the session is released. Inside that window the
//! user can still pull it back into the composer (Ctrl+P) or drop it (Ctrl+X).
//!
//! The slot is intentionally a plain in-memory structure with no async state
//! — it lives inside `App` and is touched only from the UI thread.

use std::collections::BTreeMap;

use crate::{ComposerDraft, DraftSlot};

#[derive(Debug, Default)]
/// Queue of pending composer messages, keyed by their original draft slot.
pub struct ComposerQueue {
    drafts: BTreeMap<DraftSlot, ComposerDraft>,
}

impl ComposerQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Hold `draft` until the interrupted run releases the session. Returns
    /// `true` when an earlier in-flight send was replaced.
    pub fn set(&mut self, slot: DraftSlot, draft: ComposerDraft) -> bool {
        self.drafts.insert(slot, draft).is_some()
    }

    /// Remove and return the pending message, if any.
    pub fn take(&mut self, slot: DraftSlot) -> Option<ComposerDraft> {
        self.drafts.remove(&slot)
    }

    /// Slots that currently hold an in-flight send. The handoff is keyed by
    /// the session it was sent from, so delivery does not depend on which
    /// session the user happens to be viewing when the run releases.
    pub fn slots(&self) -> impl Iterator<Item = DraftSlot> + '_ {
        self.drafts.keys().copied()
    }

    pub fn is_empty(&self, slot: DraftSlot) -> bool {
        !self.drafts.contains_key(&slot)
    }

    pub fn clear(&mut self, slot: DraftSlot) {
        self.drafts.remove(&slot);
    }

    pub fn rebind_new_session(&mut self, session_id: i64) {
        if let Some(draft) = self.take(DraftSlot::NewSession) {
            self.set(DraftSlot::Session(session_id), draft);
        }
    }

    /// First line of the pending message, truncated for a compact footer
    /// hint. Returns `None` when nothing is pending.
    pub fn preview(&self, slot: DraftSlot, max_chars: usize) -> Option<String> {
        let text = self.drafts.get(&slot)?.text();
        let preview = text.lines().next().unwrap_or("").trim();
        let truncated: String = preview.chars().take(max_chars).collect();
        if preview.chars().count() > max_chars {
            Some(format!("{truncated}…"))
        } else {
            Some(truncated)
        }
    }
}
