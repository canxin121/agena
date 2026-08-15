use ratatui::layout::Rect;

use super::super::{App, Frame, SurfaceMode};

impl App {
    /// Renders the session hub home screen. Presentation state and the actual
    /// drawing live in `agena_tui_session::session_hub`; this wrapper only
    /// forwards the App-owned loading/error state.
    pub(crate) fn render_hub(
        &self,
        frame: &mut Frame,
        area: Rect,
        state: &crate::HubState,
        _surface: SurfaceMode,
    ) {
        agena_tui_session::session_hub::render_session_hub(
            frame,
            area,
            &state.presentation,
            state.loading,
            state.error.as_deref(),
            &self.i18n,
        );
    }
}
