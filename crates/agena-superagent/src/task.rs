use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, Result, validate_key, validate_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Starting,
    Running,
    Waiting,
    Review,
    Completed,
    Interrupted,
    Failed,
    Cancelling,
    Cancelled,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Review => "review",
            Self::Completed => "completed",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::Cancelling => "cancelling",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Running | Self::Waiting | Self::Cancelling
        )
    }

    pub fn can_resume(self) -> bool {
        matches!(
            self,
            Self::Review | Self::Interrupted | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTask {
    pub source_session_id: i64,
    pub goal: String,
    /// Keep this key stable across retries, including a lost HTTP response.
    pub request_key: String,
}

impl CreateTask {
    pub fn validate(&self) -> Result<()> {
        if self.source_session_id <= 0 {
            return Err(Error::Invalid("source_session_id must be positive"));
        }
        validate_key(&self.request_key)?;
        validate_text(&self.goal)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResumeTask {
    pub expected_revision: i64,
    pub request_key: String,
    /// Explicit recovery/feedback instructions; no blind tool replay.
    pub instruction: String,
}

impl ResumeTask {
    pub fn validate(&self) -> Result<()> {
        if self.expected_revision < 1 {
            return Err(Error::Invalid("expected_revision must be positive"));
        }
        validate_key(&self.request_key)?;
        validate_text(&self.instruction)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Task {
    #[schemars(with = "String")]
    pub id: Uuid,
    pub source_session_id: i64,
    pub workspace_id: i64,
    pub request_key: String,
    pub goal: String,
    pub instruction: String,
    pub state: TaskState,
    pub revision: i64,
    pub attempt: u32,
    pub worker_session_id: Option<i64>,
    pub execution_id: Option<String>,
    pub attempt_generation: Option<i64>,
    pub resume_key: Option<String>,
    pub resume_from_revision: Option<i64>,
    /// Public diagnostic only. Raw errors belong in structured server logs.
    pub note: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// Absent for legacy/manual tasks. Conversation-owned work is controlled
    /// in the original chat and reports there without requiring the Task UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<crate::ConversationWork>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation: Option<crate::AutomationTask>,
}

impl Task {
    pub fn new(request: CreateTask, workspace_id: i64, now_ms: i64) -> Result<Self> {
        request.validate()?;
        if workspace_id <= 0 {
            return Err(Error::Invalid("workspace_id must be positive"));
        }
        Ok(Self {
            id: Uuid::new_v4(),
            source_session_id: request.source_session_id,
            workspace_id,
            request_key: request.request_key,
            instruction: request.goal.clone(),
            goal: request.goal,
            state: TaskState::Queued,
            revision: 1,
            attempt: 1,
            worker_session_id: None,
            execution_id: None,
            attempt_generation: None,
            resume_key: None,
            resume_from_revision: None,
            note: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            conversation: None,
            automation: None,
        })
    }

    pub fn delivery_key(&self) -> String {
        format!("superagent:{}:{}", self.id, self.attempt)
    }

    pub fn apply_resume(&mut self, request: &ResumeTask) -> Result<bool> {
        request.validate()?;
        if self.resume_key.as_deref() == Some(&request.request_key) {
            if self.instruction != request.instruction
                || self.resume_from_revision != Some(request.expected_revision)
            {
                return Err(Error::Conflict(
                    "resume request key was reused with different input",
                ));
            }
            return Ok(false);
        }
        if self.revision != request.expected_revision || !self.state.can_resume() {
            return Err(Error::Conflict(
                "task changed or cannot be resumed; refresh it first",
            ));
        }
        self.attempt = self
            .attempt
            .checked_add(1)
            .ok_or(Error::Conflict("attempt limit reached"))?;
        self.resume_key = Some(request.request_key.clone());
        self.resume_from_revision = Some(self.revision);
        self.instruction = request.instruction.clone();
        self.state = TaskState::Queued;
        self.execution_id = None;
        self.attempt_generation = None;
        self.note = None;
        Ok(true)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TaskQuery {
    pub source_session_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub state: Option<TaskState>,
    /// Bounded offset pagination for the first single-operator implementation.
    pub offset: u32,
    pub limit: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> Task {
        Task::new(
            CreateTask {
                source_session_id: 1,
                goal: "修复测试".into(),
                request_key: "event-1".into(),
            },
            1,
            0,
        )
        .unwrap()
    }

    #[test]
    fn resume_is_explicit_revision_checked_and_idempotent() {
        let mut task = task();
        let request = ResumeTask {
            expected_revision: 1,
            request_key: "resume-1".into(),
            instruction: "先核对已有结果，再继续".into(),
        };
        assert!(task.apply_resume(&request).is_err());
        task.state = TaskState::Interrupted;
        assert!(task.apply_resume(&request).unwrap());
        assert_eq!(task.attempt, 2);
        task.revision += 1;
        assert!(!task.apply_resume(&request).unwrap());
        assert_eq!(task.attempt, 2);
        let changed = ResumeTask {
            instruction: "different".into(),
            ..request
        };
        assert!(task.apply_resume(&changed).is_err());
    }

    #[test]
    fn validation_bounds_untrusted_input() {
        let mut request = CreateTask {
            source_session_id: 1,
            goal: "valid".into(),
            request_key: "key".into(),
        };
        assert!(request.validate().is_ok());
        request.goal = " ".into();
        assert!(request.validate().is_err());
        request.goal = "a".repeat(32769);
        assert!(request.validate().is_err());
        assert!(!TaskState::Completed.can_resume());
        assert!(!TaskState::Review.is_active());
    }
}
