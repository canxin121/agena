use serde::{Deserialize, Serialize};

use agena_domain::{
    ExecutionStatus, PendingInteractiveRequest, PendingInteractiveRequestKind, UserInputReply,
    UserInputReplyKind, UserInputRequest,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "request_type", rename_all = "snake_case")]
pub enum RequestPart {
    UserInput(InteractiveRequestPart<UserInputRequest, UserInputReply>),
}

impl RequestPart {
    pub const fn kind(&self) -> PendingInteractiveRequestKind {
        match self {
            Self::UserInput(_) => PendingInteractiveRequestKind::UserInput,
        }
    }

    pub const fn status(&self) -> ExecutionStatus {
        match self {
            Self::UserInput(part) => part.status(),
        }
    }

    pub fn summary_text(&self) -> String {
        match self {
            Self::UserInput(part) => part.summary_text(),
        }
    }

    pub fn request_id(&self) -> &str {
        match self {
            Self::UserInput(part) => part.request_id(),
        }
    }

    pub fn pending_interactive_request(&self) -> Option<PendingInteractiveRequest> {
        match self {
            Self::UserInput(part) => part.pending_request().map(PendingInteractiveRequest::from),
        }
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
