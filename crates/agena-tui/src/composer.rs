//! Transport-neutral composer-item interaction state.
//!
//! The application owns staged attachment contents, temporary-file cleanup,
//! persistence, and submission. This module owns only the presentation state
//! for selecting those already-presented items and turns user intent into
//! effects for the application adapter to perform.

/// Current selection in the composer-item strip.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComposerItemSelection {
    selected: Option<usize>,
}

/// A composer-item action independent of the physical key that caused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerItemAction {
    Close,
    Previous,
    Next,
    Delete,
    Open,
}

/// The application-facing consequence of a composer-item action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerItemEffect {
    /// No composer-item interaction owned the action.
    Ignored,
    /// The interaction changed only presentation state.
    Consumed,
    /// Remove the selected item through the application-owned editor and
    /// attachment lifecycle adapter.
    Remove(usize),
    /// Open the selected item through the application-owned path adapter.
    Open(usize),
}

impl ComposerItemSelection {
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn is_active(&self) -> bool {
        self.selected.is_some()
    }

    pub fn is_selected(&self, index: usize) -> bool {
        self.selected == Some(index)
    }

    pub fn clear(&mut self) {
        self.selected = None;
    }

    /// Toggle the item strip. Returns whether there was an item to select.
    pub fn toggle(&mut self, item_count: usize) -> bool {
        if item_count == 0 {
            self.clear();
            return false;
        }
        self.selected = if self.selected.is_some() {
            None
        } else {
            Some(0)
        };
        true
    }

    /// Keep selection valid after the application changes its item collection.
    pub fn clamp(&mut self, item_count: usize) {
        self.selected = self
            .selected
            .and_then(|index| item_count.checked_sub(1).map(|last| index.min(last)));
    }

    /// Reduce a semantic interaction action. Item mutation and I/O remain
    /// outside this TUI state machine and are returned as effects.
    pub fn reduce(&mut self, action: ComposerItemAction, item_count: usize) -> ComposerItemEffect {
        self.clamp(item_count);
        let Some(index) = self.selected else {
            return ComposerItemEffect::Ignored;
        };

        match action {
            ComposerItemAction::Close => {
                self.clear();
                ComposerItemEffect::Consumed
            }
            ComposerItemAction::Previous => {
                self.selected = Some(index.saturating_sub(1));
                ComposerItemEffect::Consumed
            }
            ComposerItemAction::Next => {
                self.selected = Some(index.saturating_add(1).min(item_count.saturating_sub(1)));
                ComposerItemEffect::Consumed
            }
            ComposerItemAction::Delete => ComposerItemEffect::Remove(index),
            ComposerItemAction::Open => ComposerItemEffect::Open(index),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ComposerItemAction, ComposerItemEffect, ComposerItemSelection};

    #[test]
    fn selection_reduces_navigation_without_item_payloads() {
        let mut selection = ComposerItemSelection::default();
        assert!(!selection.toggle(0));
        assert_eq!(
            selection.reduce(ComposerItemAction::Open, 0),
            ComposerItemEffect::Ignored
        );

        assert!(selection.toggle(3));
        assert_eq!(selection.selected(), Some(0));
        assert_eq!(
            selection.reduce(ComposerItemAction::Previous, 3),
            ComposerItemEffect::Consumed
        );
        assert_eq!(selection.selected(), Some(0));
        assert_eq!(
            selection.reduce(ComposerItemAction::Next, 3),
            ComposerItemEffect::Consumed
        );
        assert_eq!(selection.selected(), Some(1));
        assert_eq!(
            selection.reduce(ComposerItemAction::Open, 3),
            ComposerItemEffect::Open(1)
        );
        assert_eq!(
            selection.reduce(ComposerItemAction::Delete, 3),
            ComposerItemEffect::Remove(1)
        );
    }

    #[test]
    fn selection_clamps_and_close_is_presentation_only() {
        let mut selection = ComposerItemSelection::default();
        assert!(selection.toggle(3));
        selection.reduce(ComposerItemAction::Next, 3);
        selection.reduce(ComposerItemAction::Next, 3);
        selection.clamp(1);
        assert_eq!(selection.selected(), Some(0));
        assert_eq!(
            selection.reduce(ComposerItemAction::Close, 1),
            ComposerItemEffect::Consumed
        );
        assert!(!selection.is_active());
    }
}
