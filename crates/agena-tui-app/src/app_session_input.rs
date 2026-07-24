impl App {
    pub(crate) fn handle_sessions_key(&mut self, key: KeyEvent) {
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
                self.cycle_session_view_mode();
            }
            Some(KeyAction::MoveUp) => {
                let _ = self
                    .sessions
                    .update(agena_tui_session::session_list::SessionListAction::MoveSelection(-1));
            }
            Some(KeyAction::MoveDown) => {
                let _ = self
                    .sessions
                    .update(agena_tui_session::session_list::SessionListAction::MoveSelection(1));
            }
            Some(KeyAction::PageUp) => {
                let _ = self
                    .sessions
                    .update(agena_tui_session::session_list::SessionListAction::MoveSelection(-10));
            }
            Some(KeyAction::PageDown) => {
                let _ = self
                    .sessions
                    .update(agena_tui_session::session_list::SessionListAction::MoveSelection(10));
            }
            Some(KeyAction::Open) => {
                if let agena_tui_session::session_list::SessionListEffect::OpenSession {
                    session_id,
                    title,
                } = self
                    .sessions
                    .update(agena_tui_session::session_list::SessionListAction::OpenSelected)
                {
                    self.open_session(session_id, title);
                    self.focus = Focus::Transcript;
                }
            }
            Some(KeyAction::Home) => {
                let _ = self
                    .sessions
                    .update(agena_tui_session::session_list::SessionListAction::MoveSelectionHome);
            }
            Some(KeyAction::End) => {
                let _ = self
                    .sessions
                    .update(agena_tui_session::session_list::SessionListAction::MoveSelectionEnd);
            }
            _ => {}
        }
    }
}
use crate::{App, KeyEvent};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;
use agena_tui_session::session_view::SessionViewMode;
