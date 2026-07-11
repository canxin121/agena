impl App {
    pub(in crate::app) fn handle_sessions_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('1') => {
                self.set_session_view_mode(SessionViewMode::All);
            }
            KeyCode::Char('2') => {
                self.set_session_view_mode(SessionViewMode::Roots);
            }
            KeyCode::Char('3') => {
                self.set_session_view_mode(SessionViewMode::Subtree);
            }
            KeyCode::Char('m') => {
                self.cycle_session_view_mode();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.sessions.move_selection(-1);
                self.maybe_request_more_sessions();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.sessions.move_selection(1);
                self.maybe_request_more_sessions();
            }
            KeyCode::PageUp => {
                self.sessions.move_selection(-10);
            }
            KeyCode::PageDown => {
                self.sessions.move_selection(10);
                self.maybe_request_more_sessions();
            }
            KeyCode::Enter => {
                if let Some(session) = self.sessions.current_selected() {
                    self.open_session(session.id, session.title.clone());
                    self.focus = Focus::Transcript;
                }
            }
            KeyCode::Home => self.sessions.list.move_selection_home(),
            KeyCode::End => {
                if !self.sessions.list.items.is_empty() {
                    self.sessions.list.move_selection_end();
                    self.maybe_request_more_sessions();
                }
            }
            _ => {}
        }
    }
}
use crate::app::{App, Focus, KeyCode, KeyEvent, SessionViewMode};
