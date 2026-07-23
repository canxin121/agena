impl App {
    pub(in crate::app) fn render_context_help(&mut self, frame: &mut Frame, area: Rect) {
        let Some(help) = self.context_help.as_mut() else {
            return;
        };
        render_help_dialog(frame, area, help, |text| sanitize_display_text(text));
    }
}

use super::{App, Frame, Rect, render_help_dialog, sanitize_display_text};
