//! Single pending-message slot for the composer.
//!
//! While the AI is busy, at most one user message can be parked here. It is
//! delivered when the active run finishes (or immediately after a successful
//! cancel). The user can pull it back into the composer for editing (Ctrl+P)
//! or discard it entirely (Ctrl+X).
//!
//! The slot is intentionally a plain in-memory structure with no async state
//! — it lives inside `App` and is touched only from the UI thread.

use crate::ComposerDraft;

#[derive(Debug, Default)]
pub struct ComposerQueue {
    draft: Option<ComposerDraft>,
}

impl ComposerQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Park `draft` as the single pending message. Returns `true` when an
    /// earlier pending message was replaced.
    pub fn set(&mut self, draft: ComposerDraft) -> bool {
        let replaced = self.draft.is_some();
        self.draft = Some(draft);
        replaced
    }

    /// Remove and return the pending message, if any.
    pub fn take(&mut self) -> Option<ComposerDraft> {
        self.draft.take()
    }

    pub fn is_empty(&self) -> bool {
        self.draft.is_none()
    }

    pub fn clear(&mut self) {
        self.draft = None;
    }

    /// First line of the pending message, truncated for a compact footer
    /// hint. Returns `None` when nothing is pending.
    pub fn preview(&self, max_chars: usize) -> Option<String> {
        let text = self.draft.as_ref()?.text();
        let preview = text.lines().next().unwrap_or("").trim();
        let truncated: String = preview.chars().take(max_chars).collect();
        if preview.chars().count() > max_chars {
            Some(format!("{truncated}…"))
        } else {
            Some(truncated)
        }
    }
}
