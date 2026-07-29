/// Input for creating a persisted session.
#[derive(Debug, Clone)]
pub struct SessionCreateRequest {
    pub title: String,
    pub parent_session_id: Option<i64>,
}

/// Provider/model options for one session execution.
#[derive(Debug, Clone)]
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

/// A session execution request carrying caller-owned message-part values.
#[derive(Debug, Clone)]
pub struct SessionUserMessageRequest<T> {
    pub run: SessionExecutionRequest,
    pub parts: Vec<T>,
    /// An opaque, stable delivery key supplied by an external scheduler or
    /// connector. It is persisted with the resulting user message so callers
    /// can detect replay after an interrupted acknowledgement.
    pub idempotency_key: Option<String>,
}

/// Stable user-authored content accepted by session execution. Attachments are
/// plugin-SDK values, which are already the canonical cross-host transport
/// contract; core converts this value once into its persisted message part.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionUserMessagePart {
    Text(agena_domain::TextPart),
    Attachment(agena_plugin_host::sdk::attachment::AttachmentPart),
}

#[derive(Debug, Clone)]
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

impl<T> SessionUserMessageRequest<T> {
    pub fn new(session_id: i64, options: SessionRunOptions, parts: Vec<T>) -> Self {
        Self {
            run: SessionExecutionRequest::new(session_id, options),
            parts,
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
pub struct SessionForkRequest {
    pub session_id: i64,
    pub at_message_id: Option<i64>,
    pub title: Option<String>,
    #[doc(hidden)]
    pub expected_version: Option<i64>,
}

/// Input for rewinding a session to a message boundary.
#[derive(Debug, Clone)]
pub struct SessionRewindRequest {
    pub session_id: i64,
    pub message_id: i64,
    #[doc(hidden)]
    pub expected_version: Option<i64>,
}
