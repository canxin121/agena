use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{SchedulerError, SchedulerResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobKind {
    /// Recurring job driven by a cron expression.  After
    /// `max_age_days` we fire one last time and delete the job.
    Cron {
        expression: String,
        max_age_days: u32,
    },
    /// One-shot job firing exactly once at `at`.
    Once { at: DateTime<Utc> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledJob {
    pub id: Uuid,
    pub kind: JobKind,
    pub prompt: String,
    /// Optional session id to dispatch into; when None the sink is free
    /// to spawn a fresh headless session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub next_fire_at: Option<DateTime<Utc>>,
    /// Free-form metadata (e.g. label, source).
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

impl ScheduledJob {
    pub fn new_cron(
        expression: impl Into<String>,
        prompt: impl Into<String>,
        max_age_days: u32,
    ) -> SchedulerResult<Self> {
        let expression = expression.into();
        let now = Utc::now();
        let next = compute_next_fire(&expression, now)?;
        Ok(Self {
            id: Uuid::new_v4(),
            kind: JobKind::Cron {
                expression,
                max_age_days,
            },
            prompt: prompt.into(),
            owner_session_id: None,
            created_at: now,
            last_fired_at: None,
            next_fire_at: Some(next),
            metadata: serde_json::Map::new(),
        })
    }

    pub fn new_once(at: DateTime<Utc>, prompt: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind: JobKind::Once { at },
            prompt: prompt.into(),
            owner_session_id: None,
            created_at: Utc::now(),
            last_fired_at: None,
            next_fire_at: Some(at),
            metadata: serde_json::Map::new(),
        }
    }

    pub fn with_owner(mut self, session_id: i64) -> Self {
        self.owner_session_id = Some(session_id);
        self
    }

    /// Update `next_fire_at` after a successful fire.  Returns
    /// [`JobOutcome::Expired`] when the job is done and should be
    /// removed from the store.
    pub fn advance(&mut self, fired_at: DateTime<Utc>) -> SchedulerResult<JobOutcome> {
        self.last_fired_at = Some(fired_at);
        match &self.kind {
            JobKind::Once { .. } => {
                self.next_fire_at = None;
                Ok(JobOutcome::Expired)
            }
            JobKind::Cron {
                expression,
                max_age_days,
            } => {
                let age = fired_at - self.created_at;
                if age > Duration::days(*max_age_days as i64) {
                    self.next_fire_at = None;
                    return Ok(JobOutcome::Expired);
                }
                let next = compute_next_fire(expression, fired_at)?;
                self.next_fire_at = Some(next);
                Ok(JobOutcome::Continued)
            }
        }
    }

    pub fn due(&self, now: DateTime<Utc>) -> bool {
        matches!(self.next_fire_at, Some(t) if t <= now)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOutcome {
    Continued,
    Expired,
}

/// Receives prompts as their jobs fire.  Implementations route the
/// prompt into the appropriate session.
#[async_trait::async_trait]
pub trait JobSink: Send + Sync {
    async fn deliver(&self, job: &ScheduledJob);
}

fn compute_next_fire(
    expression: &str,
    after: DateTime<Utc>,
) -> SchedulerResult<DateTime<Utc>> {
    let schedule = cron::Schedule::from_str(expression).map_err(|e| {
        SchedulerError::InvalidCron {
            expr: expression.to_string(),
            source: e,
        }
    })?;
    schedule
        .after(&after)
        .next()
        .ok_or_else(|| SchedulerError::NoFutureFire {
            expr: expression.to_string(),
        })
}
