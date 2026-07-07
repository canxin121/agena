//! Pending-message queue for the composer.
//!
//! Pending user input is grouped into three priority bands:
//!
//! * `Now`   — pushed to the front (used for cancel/recovery edge cases).
//! * `Next`  — normal user submissions while the AI is busy. FIFO.
//!
//! The queue is intentionally a plain in-memory structure with no async
//! state — it lives inside `App` and is touched only from the UI thread.

use std::collections::VecDeque;

use crate::app::ComposerDraft;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuePriority {
    Now,
    Next,
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
        }
    }

    pub fn pop_next(&mut self) -> Option<QueuedMessage> {
        self.now.pop_front().or_else(|| self.next.pop_front())
    }

    /// Pull every editable user message back into a single `ComposerDraft`,
    /// preserving order, joined by a blank line. Non-editable system
    /// messages are left in place. Returns `None` if no editable items.
    pub fn pop_all_editable(&mut self) -> Option<ComposerDraft> {
        let mut drafts: Vec<ComposerDraft> = Vec::new();
        for bucket in [&mut self.now, &mut self.next] {
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
        let mut text = String::new();
        let mut elements = Vec::new();
        let mut items = Vec::new();
        for (idx, draft) in drafts.into_iter().enumerate() {
            if idx > 0 {
                text.push_str("\n\n");
            }
            let prev_len = text.len();
            text.push_str(draft.text.as_str());
            // shift element / item ranges by prev_len
            for mut element in draft.elements {
                element.range = (element.range.start + prev_len)..(element.range.end + prev_len);
                elements.push(element);
            }
            items.extend(draft.items);
        }
        Some(ComposerDraft {
            text,
            items,
            elements,
        })
    }

    pub fn len(&self) -> usize {
        self.now.len() + self.next.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&mut self) {
        self.now.clear();
        self.next.clear();
    }

    pub fn first_preview(&self, max_chars: usize) -> Option<String> {
        let head = self.now.front().or_else(|| self.next.front())?;
        let preview = head.draft.text.lines().next().unwrap_or("").trim();
        let truncated: String = preview.chars().take(max_chars).collect();
        if preview.chars().count() > max_chars {
            Some(format!("{truncated}…"))
        } else {
            Some(truncated)
        }
    }
}
