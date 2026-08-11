use ratatui::layout::Rect;

use super::super::{App, Frame};

impl App {
    pub(crate) fn render_activities_panel(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &crate::ActivitiesState,
    ) {
        let now_ms = crate::Utc::now().timestamp_millis();
        agena_tui::activities::render_activities_panel(
            frame,
            area,
            &state.presentation,
            state.rows.as_slice(),
            agena_tui::activities::ActivitiesPanelContext {
                loading: state.loading || state.mutation_loading,
                error: state.error.as_deref(),
                log_tail: state.log_tail.as_ref(),
                log_error: state.log_error.as_deref(),
                now_ms,
            },
        );
    }
}
