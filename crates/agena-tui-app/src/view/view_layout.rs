impl App {
    pub(crate) fn transcript_header_height(&self, total_height: u16) -> u16 {
        // Reserve a subtitle row only when there is a path to show. The
        // final row is also the inline history control/separator.
        let subtitle = self
            .current_session_path_label()
            .is_some_and(|path| !path.trim().is_empty());
        min(2 + u16::from(subtitle), total_height)
    }

    pub(crate) fn composer_height(&self, width: u16, total_height: u16) -> u16 {
        // The composer surface spends two columns on its border and another
        // two on its inner text inset. Keep this calculation aligned with
        // `render_composer` so wrapped text gets enough vertical room.
        let editor_width = width.saturating_sub(4).max(1);
        let line_count = max(1, self.composer.wrapped_line_count(editor_width));
        // The status chip lives on the top border row, so the only chrome is
        // the border itself.
        let chrome_rows = 2_u16;
        let minimum_height = chrome_rows.saturating_add(1);
        let available_height = total_height.saturating_sub(4).max(minimum_height);
        min(12, available_height).min(
            u16::try_from(line_count)
                .unwrap_or(u16::MAX)
                .saturating_add(chrome_rows)
                .max(minimum_height),
        )
    }
}
use super::{App, max, min};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{I18n, LaunchOptions, Route, TuiBackend};
    use ratatui::{Terminal, backend::TestBackend};

    #[tokio::test]
    async fn transcript_header_reclaims_absent_subtitle_row_without_clipping_content() {
        let mut app = App::new_with_backend(
            TuiBackend::remote_mock(),
            LaunchOptions::default(),
            I18n::english(),
        );
        app.current_route = Route::Main;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();

        assert_eq!(app.layout.transcript_body.y, 2);
        assert_eq!(app.surface_layout.header_subtitle.height, 0);
        let body = app.layout.transcript_body;
        let first_row = (body.x..body.right())
            .map(|x| terminal.backend().buffer()[(x, body.y)].symbol())
            .collect::<String>();
        assert!(first_row.contains(&app.i18n.text("no-session-selected")));

        app.transcript.session_id = Some(7);
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(app.layout.transcript_body.y, 3);
        assert_eq!(app.surface_layout.header_subtitle.height, 1);
        assert!(app.layout.transcript_body.height > 0);
        assert!(app.layout.transcript_body.bottom() <= app.surface_layout.composer_outer.y);
    }
}
