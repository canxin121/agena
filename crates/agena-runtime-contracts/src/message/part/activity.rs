use serde::{Deserialize, Serialize};

use agena_domain::{
    ExecutionFailureKind, ExecutionId, ExecutionSource, ExecutionStatus, PendingInteractiveRequest,
    PendingInteractiveRequestKind, PermissionReply, PermissionReplyKind, PermissionRequest,
    PromptCompactionActivity, TimeRange, UserInputReply, UserInputReplyKind, UserInputRequest,
};

/// Typed user-only activity. Unlike an operation part, this value has no
/// model-projection semantics and is rejected by every provider boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityPart {
    pub activity_id: String,
    pub kind: ActivityKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ActivityError>,
    #[serde(default)]
    pub lifecycle: TimeRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "activity_type", rename_all = "snake_case")]
pub enum ActivityKind {
    Execution {
        execution_id: ExecutionId,
        source: ExecutionSource,
    },
    Compaction {
        execution_id: ExecutionId,
        activity: PromptCompactionActivity,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<ExecutionFailureKind>,
}

impl ActivityPart {
    pub fn execution(
        execution_id: ExecutionId,
        source: ExecutionSource,
        started_at_ms: i64,
    ) -> Self {
        Self {
            activity_id: execution_id.to_string(),
            kind: ActivityKind::Execution {
                execution_id,
                source,
            },
            title: execution_activity_title(source, false).to_owned(),
            summary: String::new(),
            error: None,
            lifecycle: TimeRange {
                start_ms: started_at_ms,
                end_ms: None,
            },
        }
    }

    pub fn execution_source(&self) -> Option<ExecutionSource> {
        match self.kind {
            ActivityKind::Execution { source, .. } => Some(source),
            ActivityKind::Compaction { .. } => Some(ExecutionSource::Compaction),
        }
    }

    pub fn complete_execution(&mut self, finished_at_ms: i64) {
        let source = self.execution_source().unwrap_or_default();
        self.title = execution_activity_title(source, true).to_owned();
        self.error = None;
        self.lifecycle.end_ms = Some(finished_at_ms);
    }

    pub fn fail_execution(
        &mut self,
        finished_at_ms: i64,
        failure_kind: ExecutionFailureKind,
        message: String,
    ) {
        let source = self.execution_source().unwrap_or_default();
        self.title = execution_activity_failure_title(source).to_owned();
        self.summary = message.clone();
        self.error = Some(ActivityError {
            message,
            failure_kind: Some(failure_kind),
        });
        self.lifecycle.end_ms = Some(finished_at_ms);
    }

    pub fn cancel_execution(&mut self, finished_at_ms: i64) {
        let source = self.execution_source().unwrap_or_default();
        self.title = execution_activity_cancelled_title(source).to_owned();
        self.error = None;
        self.lifecycle.end_ms = Some(finished_at_ms);
    }

    pub fn apply_compaction(
        &mut self,
        execution_id: ExecutionId,
        activity: PromptCompactionActivity,
    ) {
        self.title = "Context compacted".to_owned();
        self.summary = format!(
            "Reduced context from {} to {} tokens",
            activity.before_tokens, activity.after_tokens
        );
        self.kind = ActivityKind::Compaction {
            execution_id,
            activity,
        };
    }
}

fn execution_activity_title(source: ExecutionSource, completed: bool) -> &'static str {
    match (source, completed) {
        (ExecutionSource::User, false) => "Generating response",
        (ExecutionSource::User, true) => "Response completed",
        (ExecutionSource::Continue, false) => "Continuing response",
        (ExecutionSource::Continue, true) => "Continuation completed",
        (ExecutionSource::Compaction, false) => "Compacting context",
        (ExecutionSource::Compaction, true) => "Context compacted",
        (ExecutionSource::PermissionReply, false) => "Applying permission reply",
        (ExecutionSource::PermissionReply, true) => "Permission reply applied",
        (ExecutionSource::UserInputReply, false) => "Submitting user input",
        (ExecutionSource::UserInputReply, true) => "User input submitted",
    }
}

fn execution_activity_failure_title(source: ExecutionSource) -> &'static str {
    match source {
        ExecutionSource::User => "Response failed",
        ExecutionSource::Continue => "Continuation failed",
        ExecutionSource::Compaction => "Context compaction failed",
        ExecutionSource::PermissionReply => "Permission reply failed",
        ExecutionSource::UserInputReply => "User input submission failed",
    }
}

fn execution_activity_cancelled_title(source: ExecutionSource) -> &'static str {
    match source {
        ExecutionSource::User => "Response cancelled",
        ExecutionSource::Continue => "Continuation cancelled",
        ExecutionSource::Compaction => "Context compaction cancelled",
        ExecutionSource::PermissionReply => "Permission reply cancelled",
        ExecutionSource::UserInputReply => "User input submission cancelled",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "request_type", rename_all = "snake_case")]
pub enum RequestPart {
    Permission(InteractiveRequestPart<PermissionRequest, PermissionReply>),
    UserInput(InteractiveRequestPart<UserInputRequest, UserInputReply>),
}

macro_rules! map_request_part {
    ($value:expr, |$part:ident| $body:expr) => {
        match $value {
            RequestPart::Permission($part) => $body,
            RequestPart::UserInput($part) => $body,
        }
    };
}

impl RequestPart {
    pub const fn kind(&self) -> PendingInteractiveRequestKind {
        match self {
            Self::Permission(_) => PendingInteractiveRequestKind::Permission,
            Self::UserInput(_) => PendingInteractiveRequestKind::UserInput,
        }
    }

    pub const fn status(&self) -> ExecutionStatus {
        map_request_part!(self, |part| part.status())
    }

    pub fn summary_text(&self) -> String {
        map_request_part!(self, |part| part.summary_text())
    }

    pub fn request_id(&self) -> &str {
        map_request_part!(self, |part| part.request_id())
    }

    pub fn pending_interactive_request(&self) -> Option<PendingInteractiveRequest> {
        map_request_part!(self, |part| {
            part.pending_request().map(PendingInteractiveRequest::from)
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteractiveRequestPart<Request, Reply> {
    pub request: Request,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<Reply>,
}

impl<Request, Reply> InteractiveRequestPart<Request, Reply> {
    pub fn pending(request: Request) -> Self {
        Self {
            request,
            reply: None,
        }
    }

    pub fn replied(request: Request, reply: Reply) -> Self {
        Self {
            request,
            reply: Some(reply),
        }
    }

    pub const fn status(&self) -> ExecutionStatus {
        if self.reply.is_some() {
            ExecutionStatus::Completed
        } else {
            ExecutionStatus::Pending
        }
    }

    pub fn pending_request(&self) -> Option<Request>
    where
        Request: Clone,
    {
        self.reply.is_none().then_some(self.request.clone())
    }
}

impl InteractiveRequestPart<PermissionRequest, PermissionReply> {
    pub fn request_id(&self) -> &str {
        self.request.request_id.as_str()
    }

    pub fn summary_text(&self) -> String {
        match self.reply.as_ref() {
            None => format!("Awaiting permission: {}", self.request.reason),
            Some(reply) => {
                let reason = reply
                    .reason
                    .as_deref()
                    .unwrap_or(self.request.reason.as_str());
                let prefix = match reply.kind {
                    PermissionReplyKind::AllowOnce => "Permission allowed once",
                    PermissionReplyKind::AllowAlways => "Permission allowed always",
                    PermissionReplyKind::DenyOnce => "Permission denied once",
                    PermissionReplyKind::DenyAlways => "Permission denied always",
                };
                format!("{prefix}: {reason}")
            }
        }
    }
}

impl InteractiveRequestPart<UserInputRequest, UserInputReply> {
    pub fn request_id(&self) -> &str {
        self.request.request_id.as_str()
    }

    pub fn summary_text(&self) -> String {
        match self.reply.as_ref() {
            None => {
                let count = self.request.questions.len();
                match count {
                    0 => "Ask user".to_string(),
                    1 => "Waiting for answer".to_string(),
                    _ => format!("Waiting for {count} answers"),
                }
            }
            Some(reply) => match reply.kind {
                UserInputReplyKind::Submit => {
                    format!("Answered {} question(s)", reply.answers.len())
                }
                UserInputReplyKind::Cancel => {
                    let reason = reply
                        .reason
                        .as_deref()
                        .filter(|reason| !reason.trim().is_empty())
                        .unwrap_or("user declined to answer");
                    format!("Ask cancelled: {reason}")
                }
                UserInputReplyKind::Timeout => "Ask timed out; continued automatically".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod activity_tests {
    use super::{ActivityKind, ActivityPart};
    use agena_domain::{
        ExecutionFailureKind, ExecutionId, ExecutionSource, PromptCompactionActivity,
        PromptCompactionStrategy, PromptCompactionTrigger,
    };

    #[test]
    fn execution_activity_keeps_one_identity_across_terminal_updates() {
        let execution_id = ExecutionId::new();
        let mut activity = ActivityPart::execution(execution_id, ExecutionSource::Compaction, 10);
        let identity = activity.activity_id.clone();
        activity.apply_compaction(
            execution_id,
            PromptCompactionActivity {
                checkpoint_id: "checkpoint".to_owned(),
                generation: 2,
                compacted_through_message_id: 40,
                trigger: PromptCompactionTrigger::Manual,
                strategy: PromptCompactionStrategy::LocalSummary,
                before_tokens: 10_000,
                after_tokens: 2_500,
            },
        );
        activity.complete_execution(20);

        assert_eq!(activity.activity_id, identity);
        assert_eq!(activity.lifecycle.start_ms, 10);
        assert_eq!(activity.lifecycle.end_ms, Some(20));
        assert!(matches!(activity.kind, ActivityKind::Compaction { .. }));
        assert!(activity.error.is_none());
    }

    #[test]
    fn execution_failure_is_user_visible_typed_detail() {
        let mut activity = ActivityPart::execution(ExecutionId::new(), ExecutionSource::User, 10);
        activity.fail_execution(
            20,
            ExecutionFailureKind::Provider,
            "provider unavailable".to_owned(),
        );

        let error = activity.error.expect("typed failure");
        assert_eq!(error.failure_kind, Some(ExecutionFailureKind::Provider));
        assert_eq!(error.message, "provider unavailable");
        assert_eq!(activity.lifecycle.end_ms, Some(20));
    }
}
