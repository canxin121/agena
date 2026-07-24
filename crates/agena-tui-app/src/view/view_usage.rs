use ratatui::layout::Rect;

use super::{App, Frame, SurfaceMode};
use crate::{UsageDashboardState, usage_period_short_label};

impl App {
    /// The App owns the Runtime request and the real session-open effect. The
    /// complete display projection, row ordering, selection, and rendering
    /// are owned by `agena_tui::usage`.
    pub(crate) fn render_usage_dashboard(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &UsageDashboardState,
        _surface: SurfaceMode,
    ) {
        agena_tui::usage::render_usage_dashboard(
            frame,
            area,
            usage_period_short_label(state.period),
            state.loading,
            state.error.as_deref(),
            &state.presentation,
            state.data.as_ref(),
        );
    }
}
