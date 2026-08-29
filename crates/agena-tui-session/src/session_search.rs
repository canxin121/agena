//! Presentation state for the session-search picker.
//!
//! This module deliberately holds only the display projection and pagination
//! policy. The final application maps its effects to Runtime queries and keeps
//! response/error handling at that application boundary.

use std::borrow::Cow;

use agena_tui_components::{
    Editor, SearchPicker, SearchPickerDialogSpec, SearchPickerItem, SearchPickerNoCustom,
    render_search_picker_dialog,
};
use ratatui::{Frame, layout::Rect};

use agena_tui::{i18n::I18n, sanitize_picker_text};

use crate::session_view::SessionViewMode;

#[derive(Debug, Clone)]
/// A session search result item.
pub struct SessionSearchItem {
    pub session_id: i64,
    pub title: String,
    pub favorite: bool,
    pub label: String,
    pub detail: String,
}

impl SessionSearchItem {
    pub fn matches_query(&self, query: &str) -> bool {
        let query = query.to_ascii_lowercase();
        self.title.to_ascii_lowercase().contains(query.as_str())
            || self.session_id.to_string().contains(query.as_str())
    }
}

impl SearchPickerItem for SessionSearchItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Owned(self.session_id.to_string())
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        if self.favorite {
            Cow::Owned(format!("★ {}", self.label))
        } else {
            Cow::Borrowed(&self.label)
        }
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(&self.detail))
    }

    fn search_picker_fill_value(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.title)
    }
}

/// The Runtime query parameters selected by the presentation pagination state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSearchEffect {
    LoadPage {
        page_index: usize,
        cursor: Option<String>,
    },
}

#[derive(Debug, Clone)]
/// Presentation of session search.
pub struct SessionSearchPresentation {
    /// The complete subtree catalog. Visual pagination is owned by the shared
    /// `SearchPicker`; remote modes append backend result batches.
    pub all_items: Vec<SessionSearchItem>,
    pub mode: SessionViewMode,
    pub scope_session_id: Option<i64>,
    /// Index of the latest backend batch, not a user-visible page number.
    pub page_index: usize,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

impl SessionSearchPresentation {
    pub fn new(mode: SessionViewMode, scope_session_id: Option<i64>) -> Self {
        Self {
            all_items: Vec::new(),
            mode,
            scope_session_id,
            page_index: 0,
            next_cursor: None,
            has_more: false,
        }
    }

    /// Restart a query after the picker input changed.
    pub fn reset_for_query(&mut self) -> SessionSearchEffect {
        self.page_index = 0;
        self.next_cursor = None;
        self.has_more = false;
        SessionSearchEffect::LoadPage {
            page_index: 0,
            cursor: None,
        }
    }

    /// Request the next remote page if the presentation has a cursor.
    pub fn request_next_page(&mut self) -> Option<SessionSearchEffect> {
        if !self.has_more {
            return None;
        }
        let cursor = self.next_cursor.clone()?;
        self.page_index = self.page_index.saturating_add(1);
        Some(SessionSearchEffect::LoadPage {
            page_index: self.page_index,
            cursor: Some(cursor),
        })
    }

    /// Accept response pagination after the App has applied its display items.
    pub fn apply_page(&mut self, next_cursor: Option<String>, has_more: bool) {
        self.next_cursor = next_cursor;
        self.has_more = has_more;
    }

    /// Restore the prior page after a failed append request.
    pub fn reject_page(&mut self, page_index: usize) {
        if page_index > 0 && self.page_index == page_index {
            self.page_index = page_index.saturating_sub(1);
        }
    }
}

pub type SessionSearchOverlay =
    SearchPicker<SessionSearchItem, SearchPickerNoCustom, SessionSearchPresentation, Editor>;

/// Renders session search from its TUI-owned pagination and picker state. The
/// App executes the emitted Runtime query/navigation effects only.
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &SessionSearchOverlay,
    i18n: &I18n,
) {
    let spec = SearchPickerDialogSpec::new(
        i18n.text("overlay-picker-loading").into(),
        i18n.text("overlay-attach-matches").into(),
    );
    render_search_picker_dialog(frame, area, dialog, &spec, sanitize_picker_text);
}

#[cfg(test)]
mod tests {
    use super::{SessionSearchEffect, SessionSearchPresentation};
    use crate::session_view::SessionViewMode;

    #[test]
    fn paging_requires_a_cursor_and_rolls_back_failed_append() {
        let mut state = SessionSearchPresentation::new(SessionViewMode::All, None);
        assert_eq!(state.request_next_page(), None);

        state.apply_page(Some("next".to_string()), true);
        assert_eq!(
            state.request_next_page(),
            Some(SessionSearchEffect::LoadPage {
                page_index: 1,
                cursor: Some("next".to_string()),
            })
        );
        state.reject_page(1);
        assert_eq!(state.page_index, 0);
    }

    #[test]
    fn query_reset_drops_stale_pagination() {
        let mut state = SessionSearchPresentation::new(SessionViewMode::Roots, Some(42));
        state.page_index = 3;
        state.next_cursor = Some("stale".to_string());
        state.has_more = true;

        assert_eq!(
            state.reset_for_query(),
            SessionSearchEffect::LoadPage {
                page_index: 0,
                cursor: None,
            }
        );
        assert_eq!(state.page_index, 0);
        assert_eq!(state.next_cursor, None);
        assert!(!state.has_more);
    }
}
