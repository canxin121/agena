impl App {
    pub(in crate::app) fn handle_sessions_key(&mut self, key: KeyEvent) {
        match resolve_tui_key(KeyContext::Sessions, key) {
            Some(KeyAction::ModeAll) => {
                self.set_session_view_mode(SessionViewMode::All);
            }
            Some(KeyAction::ModeRoots) => {
                self.set_session_view_mode(SessionViewMode::Roots);
            }
            Some(KeyAction::ModeSubtree) => {
                self.set_session_view_mode(SessionViewMode::Subtree);
            }
            Some(KeyAction::ModeCycle) => {
                self.set_session_view_mode(self.sessions.view_mode.next())
            }
            Some(KeyAction::MoveUp) => {
                self.sessions.move_selection(-1);
                self.maybe_request_more_sessions();
            }
            Some(KeyAction::MoveDown) => {
                self.sessions.move_selection(1);
                self.maybe_request_more_sessions();
            }
            Some(KeyAction::PageUp) => {
                self.sessions.move_selection(-10);
            }
            Some(KeyAction::PageDown) => {
                self.sessions.move_selection(10);
                self.maybe_request_more_sessions();
            }
            Some(KeyAction::Open) => {
                if let Some(session) = self.sessions.current_selected() {
                    self.open_session(session.id, session.title.clone());
                    self.focus = Focus::Transcript;
                }
            }
            Some(KeyAction::Home) => self.sessions.list.move_selection_home(),
            Some(KeyAction::End) if !self.sessions.list.items.is_empty() => {
                self.sessions.list.move_selection_end();
                self.maybe_request_more_sessions();
            }
            _ => {}
        }
    }
}
use crate::app::{App, Focus, KeyEvent, SessionViewMode};
use crate::tui_keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
