//! # agena-tui-session
//!
//! Session presentation and runtime-neutral session control contracts.
//!
//! Owns the TUI's session view model: list/navigation/search helpers
//! ([`session_list`], [`session_navigation`], [`session_search`],
//! [`session_view`]), the session page/summary/message shapes, live events,
//! and the [`SessionController`] that tracks the active session from events.

pub mod session_helpers;
pub mod session_hub;
pub mod session_list;
pub mod session_navigation;
pub mod session_search;
pub mod session_view;

pub use session_hub::{
    SessionHubItem, SessionHubPresentation, SessionHubSection, SessionHubSectionKind,
};
pub use session_list::{
    SessionListAction, SessionListEffect, SessionListItem, SessionListPresentation, SessionListView,
};
pub use session_navigation::{
    SessionLineageItem, SessionLineageNode, SessionLineageRelation, SessionLineageSummary,
    SessionNavigationAction, SessionNavigationEffect, SessionNavigationItem, SessionNavigationMode,
    SessionNavigationPresentation,
};
pub use session_search::{SessionSearchEffect, SessionSearchItem, SessionSearchPresentation};
pub use session_view::SessionViewMode;

#[derive(Debug, Clone, PartialEq, Eq)]
/// A page of session items.
pub struct SessionPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Summary of a session.
pub struct SessionSummary {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub title: String,
    pub updated_at_millis: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A message in the session view.
pub struct SessionMessage {
    pub id: i64,
    pub role: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Event emitted by the session controller.
pub enum SessionEvent {
    SessionsLoaded(SessionPage<SessionSummary>),
    SessionCreated(SessionSummary),
    MessagesLoaded {
        session_id: i64,
        page: SessionPage<SessionMessage>,
    },
    SessionRefreshed {
        session_id: i64,
        sequence: Option<i64>,
    },
    SessionEventArrived {
        session_id: i64,
        event: SessionLiveEvent,
    },
    RunCancelled {
        session_id: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Live event of a session.
pub enum SessionLiveEvent {
    UserMessageAppended { message_id: i64 },
    MessagePartChanged { message_id: i64, part_id: i64 },
    AssistantMessageFinished,
    RefreshRequested,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Controller bridging the session view to the runtime.
pub struct SessionController {
    pub current_session_id: Option<i64>,
    pub active: bool,
    pub sequence: Option<i64>,
}

impl SessionController {
    pub fn apply(&mut self, event: &SessionEvent) {
        match event {
            SessionEvent::SessionsLoaded(_) => {}
            SessionEvent::SessionCreated(session) => {
                self.current_session_id = Some(session.id);
                self.active = true;
                self.sequence = None;
            }
            SessionEvent::MessagesLoaded { session_id, .. }
            | SessionEvent::SessionRefreshed { session_id, .. }
            | SessionEvent::SessionEventArrived { session_id, .. }
            | SessionEvent::RunCancelled { session_id } => {
                self.current_session_id = Some(*session_id);
                if let SessionEvent::SessionRefreshed { sequence, .. } = event {
                    self.sequence = *sequence;
                }
                if matches!(event, SessionEvent::RunCancelled { .. }) {
                    self.active = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionController, SessionEvent, SessionSummary};

    #[test]
    fn creation_and_cancellation_update_protocol_state() {
        let mut controller = SessionController::default();
        controller.apply(&SessionEvent::SessionCreated(SessionSummary {
            id: 7,
            parent_id: None,
            title: "test".into(),
            updated_at_millis: 0,
        }));
        assert_eq!(controller.current_session_id, Some(7));
        assert!(controller.active);
        controller.apply(&SessionEvent::RunCancelled { session_id: 7 });
        assert!(!controller.active);
    }
}
