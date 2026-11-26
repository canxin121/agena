use ratatui::layout::Rect;

use super::super::{App, Frame};

impl App {
    pub(crate) fn render_plan_viewer(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &crate::PlanViewerState,
    ) {
        agena_tui::plan_viewer::render_plan_viewer_with_document(
            frame,
            area,
            &state.presentation,
            state.summary.as_deref(),
            state.markdown.as_deref(),
            state.autorun,
            state.loading,
            state.error.as_deref(),
            &self.i18n,
            state.external_activity,
            |document, width| {
                if state.external_activity {
                    document
                        .lines()
                        .map(|line| ratatui::text::Line::raw(agena_tui::sanitize_picker_text(line)))
                        .collect()
                } else {
                    crate::app_plan_document::render_plan_document(document, width)
                }
            },
        );
    }
}
