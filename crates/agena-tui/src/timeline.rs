//! Presentation rows for the session timeline picker.
//!
//! The App queries and formats ordered session parts. This module owns their
//! terminal picker projection; it is a detail view over the same transcript
//! data, not a second history store.

use agena_tui_components::text::sanitize_terminal_text as sanitize_display_text;
use std::borrow::Cow;

use agena_tui_components::{
    Editor, SearchPicker, SearchPickerDialogSpec, SearchPickerItem, SearchPickerNoCustom,
    SearchPickerViewState, WorkbenchTextSection, render_search_picker_dialog_with_preview,
};
use ratatui::{Frame, layout::Rect, text::Text};

use crate::i18n::I18n;

#[derive(Debug, Clone)]
/// An item in the timeline.
pub struct TimelineItem {
    pub summary: String,
    pub detail_body: Text<'static>,
    pub search_text: String,
}

impl SearchPickerItem for TimelineItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Owned(format!("part:{}", self.summary))
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.summary)
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        None
    }

    fn search_picker_search_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.search_text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Presentation state of the timeline.
pub struct TimelinePresentation {
    /// Session query scope retained by the App overlay adapter.
    pub session_id: i64,
}

impl TimelinePresentation {
    pub fn new(session_id: i64) -> Self {
        Self { session_id }
    }
}

pub type TimelineOverlay =
    SearchPicker<TimelineItem, SearchPickerNoCustom, TimelinePresentation, Editor>;

/// Renders the complete Timeline picker from the TUI-owned presentation state.
/// Session-part loading remains in the App adapter; the TUI owns the terminal
/// preview/dialog composition.
pub fn render_overlay(frame: &mut Frame<'_>, area: Rect, dialog: &TimelineOverlay, i18n: &I18n) {
    let spec = SearchPickerDialogSpec::new(
        i18n.text("overlay-picker-loading").into(),
        i18n.text("overlay-timeline-events").into(),
    )
    .with_text(i18n);
    render_search_picker_dialog_with_preview(
        frame,
        area,
        dialog,
        &spec,
        sanitize_display_text,
        |state| {
            let detail = match state {
                SearchPickerViewState::Loading { message }
                | SearchPickerViewState::Empty { message }
                | SearchPickerViewState::Error { message } => {
                    Text::from(sanitize_display_text(message))
                }
                SearchPickerViewState::Selected(item) => item.detail_body.clone(),
            };
            vec![WorkbenchTextSection::new(
                i18n.text("overlay-timeline-detail").into(),
                detail,
                4,
                12,
            )]
        },
    );
}
