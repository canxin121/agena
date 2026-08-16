//! Session hub route: home screen listing sessions needing attention, running,
//! and recent, with a create-new-session action.
//!
//! The display projection and rendering live in `agena_tui_session::session_hub`;
//! this module owns the overview request/response plumbing and the session-open
//! / create / session-list effects that only the App can perform.

use agena_api::resource::SessionOverviewResource;

use super::{App, AppMessage, HubState, KeyEvent, Route};
use crate::{SessionHubItem, SessionHubSection, SessionHubSectionKind, SessionResource, ui_text};
use agena_tui::keymap::{KeyAction, KeyContext, resolve as resolve_tui_key};
use agena_tui::main_focus::Focus;

/// Number of most-recently-used sessions the hub asks the server to include.
const HUB_RECENT_LIMIT: u64 = 20;

impl App {
    /// Opens the session hub as the current route and kicks off the overview
    /// load. Used as the bootstrap landing view and from the hub itself.
    pub(crate) fn open_hub(&mut self) {
        let mut state = HubState::new();
        self.spawn_hub_overview_request(&mut state);
        self.route_stack.clear();
        self.current_route = Route::Hub(state);
    }

    pub(crate) fn spawn_hub_overview_request(&mut self, state: &mut HubState) {
        self.next_hub_request_id = self.next_hub_request_id.saturating_add(1);
        state.request_id = self.next_hub_request_id;
        state.loading = true;
        state.error = None;
        let request_id = state.request_id;
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            // Search is applied client-side via `SessionHubPresentation::set_query`
            // after the overview lands, so the server call stays un-filtered.
            let result = application
                .session_overview(None, HUB_RECENT_LIMIT)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::HubOverviewLoaded { request_id, result })
                .await;
        });
    }

    pub(crate) fn handle_hub_overview_loaded(
        &mut self,
        request_id: u64,
        result: super::UiResult<SessionOverviewResource>,
    ) {
        let (valid, loading, error, sections) = match result {
            Ok(overview) => (
                true,
                false,
                None,
                Some(vec![
                    // The new-session action row is a synthetic first option:
                    // Enter on it creates a fresh session, so the hub supports
                    // "Enter → new session" right from the landing screen.
                    SessionHubSection::new(
                        SessionHubSectionKind::New,
                        vec![self.hub_new_session_item()],
                    ),
                    SessionHubSection::new(
                        SessionHubSectionKind::Running,
                        overview
                            .running
                            .iter()
                            .map(|session| self.hub_session_item(session))
                            .collect(),
                    ),
                    SessionHubSection::new(
                        SessionHubSectionKind::Attention,
                        overview
                            .attention
                            .iter()
                            .map(|session| self.hub_session_item(session))
                            .collect(),
                    ),
                    SessionHubSection::new(
                        SessionHubSectionKind::Recent,
                        overview
                            .recent
                            .iter()
                            .map(|session| self.hub_session_item(session))
                            .collect(),
                    ),
                ]),
            ),
            Err(error) => (false, false, Some(error.to_string()), None),
        };
        let Route::Hub(state) = &mut self.current_route else {
            return;
        };
        if !valid || request_id != state.request_id {
            return;
        }
        state.loading = loading;
        if let Some(error) = error {
            state.error = Some(error);
        }
        if let Some(sections) = sections {
            state.presentation.set_sections(sections);
            state.presentation.set_query(&state.query);
            state.presentation.clamp_selection();
        }
    }

    pub(crate) fn handle_hub_key(&mut self, key: KeyEvent, state: &mut HubState) -> bool {
        // While search is active, printable characters edit the query instead
        // of firing the single-letter navigation bindings, so words like
        // "attention" or "recent" can be typed uninterrupted. Arrow keys,
        // PgUp/PgDn, Home/End, Tab, Enter, Esc and Ctrl+* still navigate.
        let typing = state.search_active
            && matches!(key.code, crossterm::event::KeyCode::Char(_));
        match resolve_tui_key(KeyContext::Hub, key) {
            Some(KeyAction::Close) => {
                // Esc with an active search clears the filter and leaves
                // search mode instead of closing the hub; a second Esc closes
                // the hub.
                if state.search_active || !state.query.is_empty() {
                    state.search_active = false;
                    state.query.clear();
                    self.spawn_hub_overview_request(state);
                    return false;
                }
                return true;
            }
            Some(KeyAction::HubCreateSession) if !typing => {
                // `create_session(None)` creates a fresh session and routes
                // into it as soon as the server confirms, so the hub closes
                // immediately and the new session opens on Main.
                self.create_session(None);
                return true;
            }
            Some(KeyAction::HubOpenSessionList) if !typing => {
                // Reuses the existing resume picker; it replaces the hub as
                // the current route.
                self.open_resume_session_picker();
                return true;
            }
            Some(KeyAction::Refresh) => {
                self.spawn_hub_overview_request(state);
            }
            Some(KeyAction::MoveUp) if !typing => state.presentation.move_selection(-1),
            Some(KeyAction::MoveDown) if !typing => state.presentation.move_selection(1),
            Some(KeyAction::PageUp) => state.presentation.move_selection_page(-1, 10),
            Some(KeyAction::PageDown) => state.presentation.move_selection_page(1, 10),
            Some(KeyAction::Home) => state.presentation.move_selection_home(),
            Some(KeyAction::End) => state.presentation.move_selection_end(),
            Some(KeyAction::NextTab | KeyAction::PreviousTab) if !typing => {
                state.presentation.toggle_focus();
            }
            Some(KeyAction::Open) => {
                let Some(selected) = state.presentation.selected_session().cloned() else {
                    return false;
                };
                if selected.is_new_session {
                    // Enter on the first (action) row creates a fresh session.
                    self.create_session(None);
                    return true;
                }
                self.open_session(selected.session_id, selected.title);
                self.focus = Focus::Composer;
                return true;
            }
            // A letter bound to a Hub action types into the search instead
            // while search is active.
            Some(_) if typing => {}
            Some(_) | None => {}
        }
        // `/` (or any unbound printable key) starts search mode. While active,
        // printable keys and Backspace edit the filter; each edit issues a
        // fresh overview request (the server call itself is un-filtered; the
        // query is applied client-side by the presentation).
        match key.code {
            crossterm::event::KeyCode::Char('/') if !state.search_active => {
                state.search_active = true;
            }
            crossterm::event::KeyCode::Char(c) if c.is_ascii_graphic() || c == ' ' => {
                state.search_active = true;
                state.query.push(c);
                self.spawn_hub_overview_request(state);
            }
            crossterm::event::KeyCode::Backspace if state.search_active => {
                state.query.pop();
                self.spawn_hub_overview_request(state);
            }
            _ => {}
        }
        false
    }

    /// Builds the display projection of one session row. Detail mirrors the
    /// session-search picker so the hub reads consistently with the rest of
    /// the TUI.
    pub(crate) fn hub_session_item(&self, session: &SessionResource) -> SessionHubItem {
        let mut detail_parts = vec![
            ui_text::session_state_label(&self.i18n, session.state),
            ui_text::session_meta(
                &self.i18n,
                session.id,
                session.message_count,
                session.updated_at,
            ),
        ];
        if self.transcript.session_id == Some(session.id) {
            detail_parts.push(ui_text::t(&self.i18n, "session-tag-current"));
        }
        if let Some(parent_id) = session.parent_id {
            detail_parts.push(self.i18n.text_args(
                "session-summary-parent",
                &agena_tui::fl_args!("id" => parent_id),
            ));
        }
        if session.child_session_count > 0 {
            detail_parts.push(self.i18n.text_args(
                "session-summary-children",
                &agena_tui::fl_args!("count" => session.child_session_count as i64),
            ));
        }
        SessionHubItem {
            session_id: session.id,
            title: session.title.clone(),
            label: session.title.clone(),
            detail: detail_parts.join(" | "),
            is_new_session: false,
        }
    }

    /// The synthetic first row of the hub: Entering on it creates a fresh
    /// session, so the hub supports "Enter → new session" directly.
    fn hub_new_session_item(&self) -> SessionHubItem {
        SessionHubItem {
            session_id: 0,
            title: String::new(),
            label: self.i18n.text("hub-item-new"),
            detail: self.i18n.text("hub-item-new-detail"),
            is_new_session: true,
        }
    }
}
