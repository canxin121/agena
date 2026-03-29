use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubtaskSession {
    pub session_id: String,
    pub description: String,
    pub prompt: String,
    pub subagent_type: String,
    pub command: Option<String>,
    pub model_provider_id: Option<String>,
    pub model_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct SubtaskSessionRequest {
    pub requested_task_id: Option<String>,
    pub description: String,
    pub prompt: String,
    pub subagent_type: String,
    pub command: Option<String>,
}

#[derive(Debug, Error)]
pub enum SubtaskSessionError {
    #[error("subtask session store lock poisoned")]
    LockPoisoned,
}

pub trait SubtaskSessionManager: Send + Sync {
    fn create_or_resume(
        &self,
        request: SubtaskSessionRequest,
    ) -> Result<SubtaskSession, SubtaskSessionError>;
}

#[derive(Debug, Clone, Default)]
pub struct InMemorySubtaskSessionManager {
    inner: Arc<RwLock<HashMap<String, SubtaskSession>>>,
}

impl InMemorySubtaskSessionManager {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SubtaskSessionManager for InMemorySubtaskSessionManager {
    fn create_or_resume(
        &self,
        request: SubtaskSessionRequest,
    ) -> Result<SubtaskSession, SubtaskSessionError> {
        let mut guard = self
            .inner
            .write()
            .map_err(|_| SubtaskSessionError::LockPoisoned)?;

        let now_ms = Utc::now().timestamp_millis();
        if let Some(task_id) = request.requested_task_id.as_ref()
            && let Some(existing) = guard.get_mut(task_id)
        {
            existing.updated_at_ms = now_ms;
            return Ok(existing.clone());
        }

        let session = SubtaskSession {
            session_id: request
                .requested_task_id
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            description: request.description,
            prompt: request.prompt,
            subagent_type: request.subagent_type,
            command: request.command,
            model_provider_id: None,
            model_id: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        guard.insert(session.session_id.clone(), session.clone());
        Ok(session)
    }
}
