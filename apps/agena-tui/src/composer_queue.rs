//! Pending-message queue for the composer.
//!
//! Modeled after Claude Code's `messageQueueManager` and Codex's
//! `queued_user_messages`. Three priority bands:
//!
//! * `Now`   — pushed to the front (used for cancel/recovery edge cases).
//! * `Next`  — normal user submissions while the AI is busy. FIFO.
//! * `Later` — system notifications / side-channel inputs that must never
//!             starve real user intent.
//!
//! The queue is intentionally a plain in-memory structure with no async
//! state — it lives inside `App` and is touched only from the UI thread.

use std::collections::VecDeque;

use crate::app::ComposerDraft;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // `Later` is reserved for system-notification queueing in M3+
pub enum QueuePriority {
    Now,
    Next,
    Later,
}

#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub draft: ComposerDraft,
    pub priority: QueuePriority,
    pub editable: bool,
}

#[derive(Debug, Default)]
pub struct ComposerQueue {
    now: VecDeque<QueuedMessage>,
    next: VecDeque<QueuedMessage>,
    later: VecDeque<QueuedMessage>,
}

impl ComposerQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(&mut self, draft: ComposerDraft) {
        self.push(QueuedMessage {
            draft,
            priority: QueuePriority::Next,
            editable: true,
        });
    }

    pub fn push(&mut self, msg: QueuedMessage) {
        match msg.priority {
            QueuePriority::Now => self.now.push_back(msg),
            QueuePriority::Next => self.next.push_back(msg),
            QueuePriority::Later => self.later.push_back(msg),
        }
    }

    pub fn pop_next(&mut self) -> Option<QueuedMessage> {
        self.now
            .pop_front()
            .or_else(|| self.next.pop_front())
            .or_else(|| self.later.pop_front())
    }

    /// Pull every editable user message back into a single `ComposerDraft`,
    /// preserving order, joined by a blank line. Non-editable system
    /// messages are left in place. Returns `None` if no editable items.
    pub fn pop_all_editable(&mut self) -> Option<ComposerDraft> {
        let mut drafts: Vec<ComposerDraft> = Vec::new();
        for bucket in [&mut self.now, &mut self.next, &mut self.later] {
            bucket.retain(|msg| {
                if msg.editable {
                    drafts.push(msg.draft.clone());
                    false
                } else {
                    true
                }
            });
        }
        if drafts.is_empty() {
            return None;
        }
        let mut combined = ComposerDraft::default();
        for (idx, draft) in drafts.into_iter().enumerate() {
            if idx > 0 {
                combined.text.push_str("\n\n");
            }
            let prev_len = combined.text.len();
            combined.text.push_str(draft.text.as_str());
            // shift element / item ranges by prev_len
            for mut element in draft.elements {
                element.range = (element.range.start + prev_len)..(element.range.end + prev_len);
                combined.elements.push(element);
            }
            combined.items.extend(draft.items);
        }
        Some(combined)
    }

    pub fn len(&self) -> usize {
        self.now.len() + self.next.len() + self.later.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.now.clear();
        self.next.clear();
        self.later.clear();
    }

    pub fn first_preview(&self, max_chars: usize) -> Option<String> {
        let head = self.now.front().or_else(|| self.next.front()).or_else(|| self.later.front())?;
        let preview = head.draft.text.lines().next().unwrap_or("").trim();
        let truncated: String = preview.chars().take(max_chars).collect();
        if preview.chars().count() > max_chars {
            Some(format!("{truncated}…"))
        } else {
            Some(truncated)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(text: &str) -> ComposerDraft {
        ComposerDraft {
            text: text.to_owned(),
            items: Vec::new(),
            elements: Vec::new(),
        }
    }

    #[test]
    fn fifo_within_priority() {
        let mut q = ComposerQueue::new();
        q.enqueue(draft("a"));
        q.enqueue(draft("b"));
        q.enqueue(draft("c"));
        assert_eq!(q.pop_next().unwrap().draft.text, "a");
        assert_eq!(q.pop_next().unwrap().draft.text, "b");
        assert_eq!(q.pop_next().unwrap().draft.text, "c");
        assert!(q.pop_next().is_none());
    }

    #[test]
    fn priority_ordering() {
        let mut q = ComposerQueue::new();
        q.push(QueuedMessage { draft: draft("later"), priority: QueuePriority::Later, editable: false });
        q.push(QueuedMessage { draft: draft("next"), priority: QueuePriority::Next, editable: true });
        q.push(QueuedMessage { draft: draft("now"), priority: QueuePriority::Now, editable: true });
        assert_eq!(q.pop_next().unwrap().draft.text, "now");
        assert_eq!(q.pop_next().unwrap().draft.text, "next");
        assert_eq!(q.pop_next().unwrap().draft.text, "later");
    }

    #[test]
    fn pop_all_editable_skips_non_editable() {
        let mut q = ComposerQueue::new();
        q.enqueue(draft("hello"));
        q.push(QueuedMessage { draft: draft("system"), priority: QueuePriority::Later, editable: false });
        q.enqueue(draft("world"));
        let combined = q.pop_all_editable().unwrap();
        assert_eq!(combined.text, "hello\n\nworld");
        assert_eq!(q.len(), 1);
        assert_eq!(q.pop_next().unwrap().draft.text, "system");
    }

    #[test]
    fn preview_truncates() {
        let mut q = ComposerQueue::new();
        q.enqueue(draft("a long preview that should get truncated for the UI"));
        assert!(q.first_preview(10).unwrap().ends_with('…'));
    }
}
