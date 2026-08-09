/// Input for creating a persisted session.
#[derive(Debug, Clone)]
/// Request to create a session.
pub struct SessionCreateRequest {
    pub title: String,
    pub parent_session_id: Option<i64>,
}

/// Provider/model options for one session execution.
#[derive(Debug, Clone)]
/// Options of a session run.
pub struct SessionRunOptions {
    pub model: agena_domain::ModelRef,
    pub thinking_mode: Option<String>,
    pub speed_mode: Option<String>,
    pub verbosity: Option<String>,
    pub thinking: Option<agena_domain::ThinkingRequest>,
    pub request_override: agena_domain::ModelSpeedModeRequestOverride,
    pub system: Option<String>,
    pub temperature: Option<f32>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
/// Request to execute a session run.
pub struct SessionExecutionRequest {
    pub session_id: i64,
    pub options: SessionRunOptions,
}

impl SessionExecutionRequest {
    pub fn new(session_id: i64, options: SessionRunOptions) -> Self {
        Self {
            session_id,
            options,
        }
    }
}

#[derive(Debug, Clone)]
/// Request to reply to a pending interaction.
pub struct SessionExecutionReplyRequest<T> {
    pub session_id: i64,
    pub options: SessionRunOptions,
    pub reply: T,
}

impl<T> SessionExecutionReplyRequest<T> {
    pub fn new(session_id: i64, options: SessionRunOptions, reply: T) -> Self {
        Self {
            session_id,
            options,
            reply,
        }
    }
}

#[derive(Debug, Clone)]
/// Request to submit a user message to a session.
pub struct SessionUserRunRequest {
    pub run: SessionExecutionRequest,
    pub document: agena_domain::ComposerDocument,
    /// An opaque, stable delivery key supplied by an external scheduler or
    /// connector. It is persisted with the resulting user message so callers
    /// can detect replay after an interrupted acknowledgement.
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone)]
/// Request to reply to a pending permission request.
pub struct SessionPermissionReplyRequest {
    pub request: SessionExecutionReplyRequest<agena_domain::PermissionReply>,
    pub operator: Option<String>,
}

impl SessionPermissionReplyRequest {
    pub fn new(
        session_id: i64,
        options: SessionRunOptions,
        reply: agena_domain::PermissionReply,
        operator: Option<String>,
    ) -> Self {
        Self {
            request: SessionExecutionReplyRequest::new(session_id, options, reply),
            operator,
        }
    }
}

impl SessionUserRunRequest {
    pub fn new(
        session_id: i64,
        options: SessionRunOptions,
        document: agena_domain::ComposerDocument,
    ) -> Self {
        Self {
            run: SessionExecutionRequest::new(session_id, options),
            document,
            idempotency_key: None,
        }
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        let key = key.into();
        self.idempotency_key = (!key.trim().is_empty()).then_some(key);
        self
    }
}

/// Input for forking a session's persisted history.
#[derive(Debug, Clone)]
/// Request to fork a session.
pub struct SessionForkRequest {
    pub session_id: i64,
    pub at_message_id: Option<i64>,
    pub title: Option<String>,
    #[doc(hidden)]
    pub expected_version: Option<i64>,
}

/// Input for rewinding a session to a canonical user turn boundary.
#[derive(Debug, Clone)]
/// Request to rewind a session.
pub struct SessionRewindRequest {
    pub session_id: i64,
    pub turn_id: agena_domain::TurnId,
    #[doc(hidden)]
    pub expected_version: Option<i64>,
}
