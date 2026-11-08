//! Durable conversation-owned work. The assistant chooses the routing; the
//! host supplies the actual user-message identity and fences replays.
use serde::{Deserialize, Serialize};

use crate::{Error, Result, Task, TaskState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkUpdate {
    pub message_id: i64,
    pub instruction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConversationWork {
    /// Real user input that authorized this work; never supplied by the model.
    pub origin_message_id: i64,
    /// Most recently admitted direct user message and its exact intent.
    pub last_message_id: i64,
    pub last_intent: String,
    /// Ordinary work completes without a separate UI acceptance operation.
    pub require_review: bool,
    #[serde(default)]
    pub updates: Vec<WorkUpdate>,
    #[serde(default)]
    pub attention_id: Option<String>,
    #[serde(default)]
    pub cancel_acknowledged_in_chat: bool,
    #[serde(default)]
    pub completion_acknowledged_in_chat: bool,
}

impl ConversationWork {
    pub fn new(message: i64, intent: String, require_review: bool) -> Self {
        Self {
            origin_message_id: message,
            last_message_id: message,
            last_intent: intent,
            require_review,
            updates: Vec::new(),
            attention_id: None,
            cancel_acknowledged_in_chat: false,
            completion_acknowledged_in_chat: false,
        }
    }

    /// Repeated calls in one user turn cannot create another effect. Older
    /// turns may not overwrite a newer instruction after a restart/retry.
    pub fn admit(&mut self, message: i64, intent: &str) -> Result<bool> {
        if message < self.last_message_id {
            return Err(Error::Conflict(
                "a newer user instruction already changed this work",
            ));
        }
        if message == self.last_message_id {
            return if self.last_intent == intent {
                Ok(false)
            } else {
                Err(Error::Conflict(
                    "this user turn already applied a different instruction to this work",
                ))
            };
        }
        self.last_message_id = message;
        self.last_intent = intent.to_owned();
        Ok(true)
    }
}

/// A durable notice is inserted in the same transaction as its task state.
/// The rendered payload is frozen before delivery, so a lost acknowledgement
/// never replays a different message under the same inbox key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkNotice {
    pub key: String,
    pub task: Task,
    pub prepared_text: Option<String>,
    pub failures: u32,
}

impl Task {
    pub fn work_notice_key(&self) -> Option<String> {
        let work = self.conversation.as_ref()?;
        if self.state == TaskState::Cancelled && work.cancel_acknowledged_in_chat {
            return None;
        }
        if self.state == TaskState::Completed && work.completion_acknowledged_in_chat {
            return None;
        }
        let stage = match self.state {
            TaskState::Completed
            | TaskState::Review
            | TaskState::Failed
            | TaskState::Interrupted
            | TaskState::Cancelled => self.state.as_str().to_owned(),
            TaskState::Waiting => format!("waiting:{}", work.attention_id.as_deref()?),
            _ => return None,
        };
        Some(format!("work:{}:{}:{stage}", self.id, self.attempt))
    }

    /// Continue explicitly queued follow-up work only after a successful turn.
    /// Interrupted/failed execution requires a fresh user instruction, not a
    /// blind automatic replay of uncertain external actions.
    pub fn advance_conversation(&mut self) -> Result<bool> {
        let Some(work) = self.conversation.as_mut() else {
            return Ok(false);
        };
        if work.updates.is_empty() {
            return Ok(false);
        }
        let update = work.updates.remove(0);
        self.attempt = self
            .attempt
            .checked_add(1)
            .ok_or(Error::Conflict("attempt limit reached"))?;
        self.instruction = update.instruction;
        self.state = TaskState::Queued;
        self.execution_id = None;
        self.attempt_generation = None;
        self.note = None;
        work.attention_id = None;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_user_identity_fences_duplicates_and_old_turns() {
        let mut work = ConversationWork::new(10, "start".into(), false);
        assert!(!work.admit(10, "start").unwrap());
        assert!(work.admit(11, "add test").unwrap());
        assert!(!work.admit(11, "add test").unwrap());
        assert!(work.admit(11, "cancel").is_err());
        assert!(work.admit(10, "start").is_err());
    }
}
