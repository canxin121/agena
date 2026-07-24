//! Session presentation and runtime-neutral session control contracts.

pub mod session_list;
pub mod session_navigation;
pub mod session_search;
pub mod session_view;

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

/// Text and opaque attachment identifiers are the only composer data that
/// crosses the session protocol. Platform/editor state stays in the app.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionDraft {
    pub text: String,
    pub attachment_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionReply {
    Allow,
    AllowForSession,
    Deny,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserInputReply {
    Answers(Vec<String>),
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCommand {
    CreateSession,
    SubmitMessage { draft: SessionDraft },
    Continue,
    Compact,
    Rewind { message_id: i64 },
    Cancel,
    ReplyPermission { reply: PermissionReply },
    ReplyUserInput { reply: UserInputReply },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub title: String,
    pub updated_at_millis: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMessage {
    pub id: i64,
    pub role: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
pub enum SessionLiveEvent {
    UserMessageAppended { message_id: i64 },
    MessagePartChanged { message_id: i64, part_id: i64 },
    AssistantMessageFinished,
    RefreshRequested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEffect {
    LoadSessions {
        cursor: Option<String>,
    },
    LoadMessages {
        session_id: i64,
        cursor: Option<String>,
    },
    RefreshSession {
        session_id: i64,
        sequence: Option<i64>,
    },
    SubmitMessage {
        session_id: Option<i64>,
        draft: SessionDraft,
    },
    Continue {
        session_id: i64,
    },
    Compact {
        session_id: i64,
    },
    Rewind {
        session_id: i64,
        message_id: i64,
    },
    Cancel {
        session_id: i64,
    },
    ReplyPermission {
        session_id: i64,
        reply: PermissionReply,
    },
    ReplyUserInput {
        session_id: i64,
        reply: UserInputReply,
    },
    SubscribeSessionEvents {
        session_id: i64,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionController {
    pub current_session_id: Option<i64>,
    pub active: bool,
    pub sequence: Option<i64>,
}

impl SessionController {
    pub fn reduce(&self, command: SessionCommand) -> Option<SessionEffect> {
        match command {
            SessionCommand::CreateSession => Some(SessionEffect::SubmitMessage {
                session_id: None,
                draft: SessionDraft::default(),
            }),
            SessionCommand::SubmitMessage { draft } => Some(SessionEffect::SubmitMessage {
                session_id: self.current_session_id,
                draft,
            }),
            SessionCommand::Continue => self
                .current_session_id
                .map(|session_id| SessionEffect::Continue { session_id }),
            SessionCommand::Compact => self
                .current_session_id
                .map(|session_id| SessionEffect::Compact { session_id }),
            SessionCommand::Rewind { message_id } => {
                self.current_session_id
                    .map(|session_id| SessionEffect::Rewind {
                        session_id,
                        message_id,
                    })
            }
            SessionCommand::Cancel => self
                .current_session_id
                .map(|session_id| SessionEffect::Cancel { session_id }),
            SessionCommand::ReplyPermission { reply } => self
                .current_session_id
                .map(|session_id| SessionEffect::ReplyPermission { session_id, reply }),
            SessionCommand::ReplyUserInput { reply } => self
                .current_session_id
                .map(|session_id| SessionEffect::ReplyUserInput { session_id, reply }),
        }
    }

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
    use super::{
        SessionCommand, SessionController, SessionDraft, SessionEffect, SessionEvent,
        SessionSummary,
    };

    #[test]
    fn controller_keeps_transport_out_of_the_protocol() {
        let controller = SessionController::default();
        assert_eq!(controller.reduce(SessionCommand::Continue), None);
        assert_eq!(
            controller.reduce(SessionCommand::SubmitMessage {
                draft: SessionDraft {
                    text: "hello".into(),
                    attachment_ids: vec![]
                }
            }),
            Some(SessionEffect::SubmitMessage {
                session_id: None,
                draft: SessionDraft {
                    text: "hello".into(),
                    attachment_ids: vec![]
                }
            })
        );
    }

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
