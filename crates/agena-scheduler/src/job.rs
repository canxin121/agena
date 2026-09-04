//! Job definitions (cron / one-shot) and fire state.

use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{SchedulerError, SchedulerResult};

fn scheduler_outcome_failure(
    code: &'static str,
    category: agena_failure::FailureCategory,
    diagnostic: impl std::fmt::Display,
) -> agena_failure::Failure {
    let fallback = match code {
        "scheduler.misfire_skipped" => {
            "The scheduled run was skipped because it was no longer current."
        }
        _ => "The scheduled delivery failed temporarily.",
    };
    let failure = agena_failure::Failure::new(
        agena_failure::FailureCode::new(code),
        category,
        agena_failure::FailureResponsibility::System,
        agena_failure::RetryDirective::AfterRefresh,
        agena_failure::RecoveryDirective::Refresh,
        agena_failure::FailureImpact::OperationFailed,
        agena_failure::UserPresentation::new("scheduler-outcome", fallback),
    );
    tracing::warn!(failure_id = %failure.id, diagnostic = %diagnostic, "scheduler operation failed");
    failure
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Kind of a scheduled job.
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Status of a job run.
pub enum JobRunStatus {
    Submitted,
    Skipped,
    Failed,
}

/// What to do when a job has been overdue long enough to be considered a
/// scheduler misfire (for example, after the runtime was stopped).
///
/// `RunOnceNow` executes one delivery immediately, but never synthesizes a
/// burst of every cron tick that was
/// missed. `Skip` consumes one scheduled occurrence at a time and records it
/// as skipped. `Reschedule` drops all missed occurrences and advances directly
/// to the first future cron tick.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MisfirePolicy {
    Skip,
    #[default]
    RunOnceNow,
    Reschedule,
}

/// Bounded exponential retry policy for a failed delivery attempt.
///
/// `max_attempts` includes the initial delivery; the default therefore means
/// one normal attempt and at most two retries. The policy deliberately applies
/// only to [`JobRunStatus::Failed`]. A `Skipped` result is an intentional
/// decision by the sink (such as a blocked owner session) and is advanced like
/// a normal cron occurrence instead of being retried in a tight loop.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_delay_seconds: u32,
    pub max_delay_seconds: u32,
    pub multiplier: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay_seconds: 15,
            max_delay_seconds: 300,
            multiplier: 2,
        }
    }
}

impl RetryPolicy {
    /// Normalize untrusted/deserialized values to a finite, non-zero retry
    /// policy. This keeps old or hand-written job JSON from creating a busy
    /// retry loop.
    fn normalized(self) -> Self {
        let initial_delay_seconds = self.initial_delay_seconds.max(1);
        Self {
            max_attempts: self.max_attempts.max(1),
            initial_delay_seconds,
            max_delay_seconds: self.max_delay_seconds.max(initial_delay_seconds),
            multiplier: self.multiplier.max(1),
        }
    }

    fn delay_after_attempt(self, attempt: u32) -> Duration {
        let policy = self.normalized();
        // An attempt number of 1 means the first retry uses the initial
        // delay. Cap the exponent to avoid overflow in malformed persisted
        // state; the final value is always capped by `max_delay_seconds`.
        let exponent = attempt.saturating_sub(1).min(30);
        let factor = (policy.multiplier as i64).saturating_pow(exponent);
        let seconds = (policy.initial_delay_seconds as i64)
            .saturating_mul(factor)
            .min(policy.max_delay_seconds as i64);
        Duration::seconds(seconds)
    }
}

/// A durable delivery claim. The same `delivery_key` is retained across
/// retries and restarts for one scheduled occurrence, so a downstream sink
/// can use it as its idempotency key. The scheduler itself offers at-least-once
/// delivery: a crash after a claim but before completion makes the claim
/// eligible for a retry after restart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobDeliveryAttempt {
    pub delivery_key: String,
    pub scheduled_for: DateTime<Utc>,
    pub attempt: u32,
    pub claimed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Record of one job run.
pub struct JobRunRecord {
    pub triggered_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: JobRunStatus,
    /// Original schedule time. This differs from `triggered_at` for a
    /// run-once-now misfire or a delayed retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_for: Option<DateTime<Utc>>,
    /// Stable idempotency key for the scheduled occurrence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_key: Option<String>,
    /// One-based attempt number, including the initial delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::Failure>,
}

/// A durable, scheduler-wide audit entry. It intentionally retains the job
/// identifier alongside a copy of one immutable run record so history remains
/// queryable after a user deletes the job itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerHistoryEntry {
    pub job_id: Uuid,
    pub record: JobRunRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Result of delivering a job to the sink.
pub struct JobDeliveryResult {
    pub status: JobRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::Failure>,
}

impl JobDeliveryResult {
    pub fn submitted(session_id: Option<i64>) -> Self {
        Self {
            status: JobRunStatus::Submitted,
            session_id,
            failure: None,
        }
    }

    pub fn skipped(session_id: Option<i64>, failure: agena_failure::Failure) -> Self {
        Self {
            status: JobRunStatus::Skipped,
            session_id,
            failure: Some(failure),
        }
    }

    pub fn failed(session_id: Option<i64>, failure: agena_failure::Failure) -> Self {
        Self {
            status: JobRunStatus::Failed,
            session_id,
            failure: Some(failure),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
/// Exact assistant tool-call identity that created a scheduled job.
///
/// This is persisted inside the scheduler's canonical job JSON. It is not a
/// mutable delivery cursor: recurring fires intentionally retain the same
/// provenance for their entire lifetime.
pub struct ScheduledJobLaunchProvenance {
    pub session_id: i64,
    pub run_id: i64,
    pub tool_part_id: i64,
    pub call_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// A scheduled job.
pub struct ScheduledJob {
    pub id: Uuid,
    pub kind: JobKind,
    pub prompt: String,
    /// Optional session id to dispatch into; when None the sink is free
    /// to spawn a fresh headless session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<i64>,
    /// Durable provenance for schedules created by an assistant tool call.
    ///
    /// Host-created jobs may have no provenance and are delivered as Runtime
    /// ingress. When present, every fire is attached to
    /// this exact assistant run/tool part instead of creating a new run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_provenance: Option<ScheduledJobLaunchProvenance>,
    pub created_at: DateTime<Utc>,
    pub last_fired_at: Option<DateTime<Utc>>,
    pub next_fire_at: Option<DateTime<Utc>>,
    /// IANA timezone used to evaluate cron wall-clock fields. One-shot jobs use UTC.
    pub timezone: String,
    /// Explicit recovery policy for overdue jobs.
    #[serde(default)]
    pub misfire_policy: MisfirePolicy,
    /// Bounded retry policy for failed deliveries.
    #[serde(default)]
    pub retry_policy: RetryPolicy,
    /// A persisted claim is written before a sink is called. It is retained
    /// across restart until a delivery is successfully finalized or retries
    /// are exhausted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_delivery: Option<JobDeliveryAttempt>,
    /// Earliest time at which a failed `pending_delivery` may be retried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<DateTime<Utc>>,
    /// A paused job remains durable but cannot become due. Resuming a cron
    /// job computes the next future fire instead of replaying missed ticks.
    #[serde(default)]
    pub paused: bool,
    /// Terminal jobs remain available for audit until explicitly deleted.
    #[serde(default)]
    pub completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<JobRunRecord>,
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
        Self::new_cron_in_timezone(expression, prompt, max_age_days, "UTC")
    }

    /// Create a cron schedule whose wall-clock fields are evaluated in an
    /// explicit IANA timezone. The timezone is persisted as part of the job so
    /// resume, update, advance, and restart recovery are deterministic.
    pub fn new_cron_in_timezone(
        expression: impl Into<String>,
        prompt: impl Into<String>,
        max_age_days: u32,
        timezone: &str,
    ) -> SchedulerResult<Self> {
        let expression = expression.into();
        let timezone = parse_timezone(timezone)?;
        let now = Utc::now();
        let next = compute_next_fire(&expression, now, timezone)?;
        Ok(Self {
            id: Uuid::new_v4(),
            kind: JobKind::Cron {
                expression,
                max_age_days,
            },
            prompt: prompt.into(),
            owner_session_id: None,
            launch_provenance: None,
            created_at: now,
            last_fired_at: None,
            next_fire_at: Some(next),
            timezone: timezone.name().to_owned(),
            misfire_policy: MisfirePolicy::default(),
            retry_policy: RetryPolicy::default(),
            pending_delivery: None,
            retry_at: None,
            paused: false,
            completed: false,
            last_run: None,
            metadata: serde_json::Map::new(),
        })
    }

    pub fn cron_timezone(&self) -> &str {
        self.timezone.as_str()
    }

    pub fn new_once(at: DateTime<Utc>, prompt: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind: JobKind::Once { at },
            prompt: prompt.into(),
            owner_session_id: None,
            launch_provenance: None,
            created_at: Utc::now(),
            last_fired_at: None,
            next_fire_at: Some(at),
            timezone: "UTC".to_owned(),
            misfire_policy: MisfirePolicy::default(),
            retry_policy: RetryPolicy::default(),
            pending_delivery: None,
            retry_at: None,
            paused: false,
            completed: false,
            last_run: None,
            metadata: serde_json::Map::new(),
        }
    }

    pub fn set_owner(&mut self, session_id: i64) {
        self.owner_session_id = Some(session_id);
    }

    /// Bind an assistant-created schedule to the exact durable tool receipt
    /// that created it. The owner is derived from the same value so the two
    /// identities cannot drift apart.
    pub fn set_launch_provenance(&mut self, provenance: ScheduledJobLaunchProvenance) {
        self.owner_session_id = Some(provenance.session_id);
        self.launch_provenance = Some(provenance);
    }

    pub fn set_recovery_policy(
        &mut self,
        misfire_policy: MisfirePolicy,
        retry_policy: RetryPolicy,
    ) {
        self.misfire_policy = misfire_policy;
        self.retry_policy = retry_policy.normalized();
    }

    pub fn pause(&mut self) -> bool {
        if self.completed || self.paused {
            return false;
        }
        self.paused = true;
        true
    }

    pub fn resume(&mut self, now: DateTime<Utc>) -> SchedulerResult<bool> {
        if self.completed || !self.paused {
            return Ok(false);
        }
        self.paused = false;
        self.pending_delivery = None;
        self.retry_at = None;
        let timezone = parse_timezone(self.cron_timezone())?;
        if let JobKind::Cron { expression, .. } = &self.kind {
            self.next_fire_at = Some(compute_next_fire(expression, now, timezone)?);
        }
        Ok(true)
    }

    pub fn update(
        &mut self,
        prompt: Option<String>,
        expression: Option<String>,
        max_age_days: Option<u32>,
        misfire_policy: Option<MisfirePolicy>,
        retry_policy: Option<RetryPolicy>,
        now: DateTime<Utc>,
    ) -> SchedulerResult<bool> {
        let mut changed = false;
        let timezone = parse_timezone(self.cron_timezone())?;
        if let Some(prompt) = prompt
            && self.prompt != prompt
        {
            self.prompt = prompt;
            changed = true;
        }
        match &mut self.kind {
            JobKind::Cron {
                expression: current_expression,
                max_age_days: current_max_age_days,
            } => {
                if let Some(expression) = expression
                    && *current_expression != expression
                {
                    let next = compute_next_fire(expression.as_str(), now, timezone)?;
                    *current_expression = expression;
                    if !self.paused {
                        self.next_fire_at = Some(next);
                    }
                    changed = true;
                }
                if let Some(max_age_days) = max_age_days
                    && *current_max_age_days != max_age_days
                {
                    *current_max_age_days = max_age_days;
                    changed = true;
                }
                if let Some(misfire_policy) = misfire_policy
                    && self.misfire_policy != misfire_policy
                {
                    self.misfire_policy = misfire_policy;
                    changed = true;
                }
                if let Some(retry_policy) = retry_policy.map(RetryPolicy::normalized)
                    && self.retry_policy != retry_policy
                {
                    self.retry_policy = retry_policy;
                    changed = true;
                }
            }
            JobKind::Once { .. }
                if expression.is_some()
                    || max_age_days.is_some()
                    || misfire_policy.is_some()
                    || retry_policy.is_some() =>
            {
                return Err(SchedulerError::InvalidUpdate(
                    "once jobs only allow prompt updates".to_string(),
                ));
            }
            JobKind::Once { .. } => {}
        }
        Ok(changed)
    }

    /// Update `next_fire_at` after a successful fire.  Returns
    /// [`JobOutcome::Expired`] when the job is terminal and should remain
    /// available for audit but never run again.
    pub fn advance(&mut self, fired_at: DateTime<Utc>) -> SchedulerResult<JobOutcome> {
        let timezone = parse_timezone(self.cron_timezone())?;
        self.last_fired_at = Some(fired_at);
        self.pending_delivery = None;
        self.retry_at = None;
        match &self.kind {
            JobKind::Once { .. } => {
                self.next_fire_at = None;
                self.completed = true;
                Ok(JobOutcome::Expired)
            }
            JobKind::Cron {
                expression,
                max_age_days,
            } => {
                let age = fired_at - self.created_at;
                if age > Duration::days(*max_age_days as i64) {
                    self.next_fire_at = None;
                    self.completed = true;
                    return Ok(JobOutcome::Expired);
                }
                let next = compute_next_fire(expression, fired_at, timezone)?;
                self.next_fire_at = Some(next);
                Ok(JobOutcome::Continued)
            }
        }
    }

    pub fn due(&self, now: DateTime<Utc>) -> bool {
        if self.paused || self.completed {
            return false;
        }
        if let Some(retry_at) = self.retry_at {
            return retry_at <= now;
        }
        self.pending_delivery.is_none() && matches!(self.next_fire_at, Some(t) if t <= now)
    }

    pub fn record_delivery(&mut self, triggered_at: DateTime<Utc>, result: JobDeliveryResult) {
        self.record_delivery_attempt(triggered_at, None, result);
    }

    fn record_delivery_attempt(
        &mut self,
        triggered_at: DateTime<Utc>,
        delivery: Option<&JobDeliveryAttempt>,
        result: JobDeliveryResult,
    ) {
        let record = JobRunRecord {
            triggered_at,
            finished_at: Utc::now(),
            status: result.status,
            scheduled_for: delivery.map(|delivery| delivery.scheduled_for),
            delivery_key: delivery.map(|delivery| delivery.delivery_key.clone()),
            attempt: delivery.map(|delivery| delivery.attempt),
            session_id: result.session_id,
            failure: result.failure,
        };
        self.last_run = Some(record);
    }

    /// Claim the next delivery before a sink is invoked. Callers must persist
    /// the mutated job before using the returned attempt. This makes an
    /// interrupted runtime recoverable instead of silently losing a fire.
    pub fn claim_due_delivery(&mut self, now: DateTime<Utc>) -> SchedulerResult<ClaimDueDelivery> {
        if self.paused || self.completed {
            return Ok(ClaimDueDelivery::NotDue);
        }

        if let Some(mut delivery) = self.pending_delivery.clone() {
            let retry_at = self.retry_at.unwrap_or(delivery.claimed_at);
            if retry_at > now {
                return Ok(ClaimDueDelivery::NotDue);
            }
            delivery.attempt = delivery.attempt.saturating_add(1).max(1);
            delivery.claimed_at = now;
            self.retry_at = None;
            self.pending_delivery = Some(delivery.clone());
            return Ok(ClaimDueDelivery::Deliver(delivery));
        }

        let Some(scheduled_for) = self.next_fire_at else {
            return Ok(ClaimDueDelivery::NotDue);
        };
        if scheduled_for > now {
            return Ok(ClaimDueDelivery::NotDue);
        }

        if is_misfire(scheduled_for, now) {
            match self.misfire_policy {
                MisfirePolicy::RunOnceNow => {}
                MisfirePolicy::Skip => {
                    self.record_misfire_skip(
                        now,
                        scheduled_for,
                        "misfire policy skipped one scheduled occurrence",
                    );
                    self.advance(scheduled_for)?;
                    return Ok(ClaimDueDelivery::StateUpdated);
                }
                MisfirePolicy::Reschedule => {
                    self.record_misfire_skip(
                        now,
                        scheduled_for,
                        "misfire policy rescheduled missed occurrences",
                    );
                    self.reschedule_after(now)?;
                    return Ok(ClaimDueDelivery::StateUpdated);
                }
            }
        }

        let delivery = JobDeliveryAttempt {
            delivery_key: format!(
                "agena.scheduler/v1/{}/{}",
                self.id,
                scheduled_for.timestamp_nanos_opt().unwrap_or_default()
            ),
            scheduled_for,
            attempt: 1,
            claimed_at: now,
        };
        self.pending_delivery = Some(delivery.clone());
        self.retry_at = None;
        Ok(ClaimDueDelivery::Deliver(delivery))
    }

    /// Finalize one previously persisted claim. A failed result keeps the
    /// claim (and its stable delivery key) until bounded retries are exhausted.
    pub fn finish_delivery(
        &mut self,
        now: DateTime<Utc>,
        delivery: &JobDeliveryAttempt,
        result: JobDeliveryResult,
    ) -> SchedulerResult<JobOutcome> {
        let claimed = self.pending_delivery.as_ref().ok_or_else(|| {
            SchedulerError::InvalidUpdate("finish_delivery without a pending claim".to_string())
        })?;
        if claimed.delivery_key != delivery.delivery_key || claimed.attempt != delivery.attempt {
            return Err(SchedulerError::InvalidUpdate(
                "finish_delivery does not match the pending claim".to_string(),
            ));
        }

        self.record_delivery_attempt(now, Some(delivery), result.clone());
        if result.status == JobRunStatus::Failed
            && delivery.attempt < self.retry_policy.normalized().max_attempts
        {
            self.retry_at = Some(now + self.retry_policy.delay_after_attempt(delivery.attempt));
            return Ok(JobOutcome::RetryScheduled);
        }

        self.advance(now)
    }

    fn record_misfire_skip(
        &mut self,
        now: DateTime<Utc>,
        scheduled_for: DateTime<Utc>,
        reason: &str,
    ) {
        let delivery = JobDeliveryAttempt {
            delivery_key: format!(
                "agena.scheduler/v1/{}/{}",
                self.id,
                scheduled_for.timestamp_nanos_opt().unwrap_or_default()
            ),
            scheduled_for,
            attempt: 0,
            claimed_at: now,
        };
        self.record_delivery_attempt(
            now,
            Some(&delivery),
            JobDeliveryResult::skipped(
                None,
                scheduler_outcome_failure(
                    "scheduler.misfire_skipped",
                    agena_failure::FailureCategory::Conflict,
                    reason,
                ),
            ),
        );
    }

    fn reschedule_after(&mut self, now: DateTime<Utc>) -> SchedulerResult<()> {
        let timezone = parse_timezone(self.cron_timezone())?;
        self.pending_delivery = None;
        self.retry_at = None;
        self.last_fired_at = Some(now);
        match &self.kind {
            JobKind::Once { .. } => {
                self.next_fire_at = None;
                self.completed = true;
            }
            JobKind::Cron { expression, .. } => {
                self.next_fire_at = Some(compute_next_fire(expression, now, timezone)?);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Outcome of delivering a job.
pub enum JobOutcome {
    Continued,
    Expired,
    RetryScheduled,
}

/// Outcome of asking a durable job to prepare its next delivery. The caller
/// persists every variant other than `NotDue`; `Deliver` specifically must be
/// persisted before the [`JobSink`] receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimDueDelivery {
    NotDue,
    StateUpdated,
    Deliver(JobDeliveryAttempt),
}

/// Receives prompts as their jobs fire.  Implementations route the
/// prompt into the appropriate session.
#[async_trait::async_trait]
pub trait JobSink: Send + Sync {
    async fn deliver(&self, job: &ScheduledJob, delivery: &JobDeliveryAttempt)
    -> JobDeliveryResult;
}

// The scheduler polls in seconds, so ordinary event-loop delay is not a
// misfire. A minute is intentionally conservative and keeps the behavior
// deterministic without serializing a host-specific clock tolerance.
const MISFIRE_GRACE: Duration = Duration::seconds(60);

fn is_misfire(scheduled_for: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now.signed_duration_since(scheduled_for) > MISFIRE_GRACE
}

fn parse_timezone(timezone: &str) -> SchedulerResult<Tz> {
    timezone.parse::<Tz>().map_err(|error| {
        SchedulerError::InvalidUpdate(format!("invalid IANA timezone `{timezone}`: {error}"))
    })
}

fn compute_next_fire(
    expression: &str,
    after: DateTime<Utc>,
    timezone: Tz,
) -> SchedulerResult<DateTime<Utc>> {
    let schedule =
        cron::Schedule::from_str(expression).map_err(|e| SchedulerError::InvalidCron {
            expr: expression.to_string(),
            source: e,
        })?;
    schedule
        .after(&after.with_timezone(&timezone))
        .next()
        .map(|time| time.with_timezone(&Utc))
        .ok_or_else(|| SchedulerError::NoFutureFire {
            expr: expression.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};

    use super::{
        ClaimDueDelivery, JobDeliveryResult, JobOutcome, JobRunStatus, MisfirePolicy, ScheduledJob,
        ScheduledJobLaunchProvenance, compute_next_fire, scheduler_outcome_failure,
    };

    #[test]
    fn assistant_launch_provenance_is_durable() {
        let mut job = ScheduledJob::new_once(Utc::now(), "wake");
        let provenance = ScheduledJobLaunchProvenance {
            session_id: 60,
            run_id: 2150,
            tool_part_id: 2195,
            call_id: 2193,
        };
        job.set_launch_provenance(provenance);
        let encoded = serde_json::to_value(&job).expect("encode scheduled job");
        let decoded: ScheduledJob = serde_json::from_value(encoded).expect("decode job");
        assert_eq!(decoded.owner_session_id, Some(60));
        assert_eq!(decoded.launch_provenance, Some(provenance));
    }

    #[test]
    fn scheduled_job_rejects_removed_per_job_run_history() {
        let job = ScheduledJob::new_once(Utc::now(), "wake");
        let mut encoded = serde_json::to_value(job).expect("encode scheduled job");
        encoded
            .as_object_mut()
            .expect("scheduled job is an object")
            .insert("run_history".to_owned(), serde_json::json!([]));
        let error = serde_json::from_value::<ScheduledJob>(encoded)
            .expect_err("removed run_history field must be rejected");
        assert!(error.to_string().contains("unknown field `run_history`"));
    }

    #[test]
    fn cron_wall_clock_is_evaluated_in_the_declared_timezone() {
        let after = Utc
            .with_ymd_and_hms(2026, 8, 14, 15, 27, 23)
            .single()
            .expect("fixed instant");
        let timezone = "Asia/Shanghai".parse().expect("timezone");
        let next = compute_next_fire("20 28 23 14 8 *", after, timezone).expect("next local fire");
        assert_eq!(
            next,
            Utc.with_ymd_and_hms(2026, 8, 14, 15, 28, 20)
                .single()
                .expect("expected UTC instant")
        );
    }

    #[test]
    fn pause_resume_and_update_preserve_durable_schedule_semantics() {
        let now = Utc::now();
        let mut job = ScheduledJob::new_cron("*/5 * * * * *", "first", 7).expect("cron job");
        assert!(job.pause());
        assert!(job.paused);
        assert!(!job.due(now + Duration::days(1)));
        assert!(job.resume(now).expect("resume"));
        assert!(!job.paused);
        assert!(job.next_fire_at.is_some());

        assert!(
            job.update(
                Some("second".to_string()),
                Some("*/10 * * * * *".to_string()),
                Some(14),
                None,
                None,
                now,
            )
            .expect("update")
        );
        assert_eq!(job.prompt, "second");
    }

    #[test]
    fn terminal_jobs_retain_the_last_delivery() {
        let now = Utc::now();
        let mut job = ScheduledJob::new_once(now, "once");
        job.record_delivery(now, JobDeliveryResult::submitted(Some(9)));
        assert_eq!(job.advance(now).expect("advance"), JobOutcome::Expired);
        assert!(job.completed);
        assert!(!job.due(now + Duration::seconds(1)));
        assert_eq!(
            job.last_run.as_ref().and_then(|run| run.session_id),
            Some(9)
        );
    }

    #[test]
    fn failed_delivery_keeps_one_stable_key_across_bounded_retries() {
        let now = Utc::now();
        let mut job = ScheduledJob::new_once(now - Duration::seconds(1), "retry");
        let ClaimDueDelivery::Deliver(first) = job.claim_due_delivery(now).expect("claim") else {
            panic!("job should be deliverable");
        };
        assert_eq!(first.attempt, 1);
        assert_eq!(
            job.finish_delivery(
                now,
                &first,
                JobDeliveryResult::failed(
                    None,
                    scheduler_outcome_failure(
                        "scheduler.delivery_failed",
                        agena_failure::FailureCategory::DependencyUnavailable,
                        "The scheduled delivery failed temporarily.",
                    ),
                ),
            )
            .expect("finish"),
            JobOutcome::RetryScheduled
        );
        let retry_at = job.retry_at.expect("retry time");
        assert!(!job.due(retry_at - Duration::milliseconds(1)));

        let ClaimDueDelivery::Deliver(second) =
            job.claim_due_delivery(retry_at).expect("claim retry")
        else {
            panic!("retry should be deliverable");
        };
        assert_eq!(second.attempt, 2);
        assert_eq!(second.delivery_key, first.delivery_key);
        assert_eq!(
            job.finish_delivery(retry_at, &second, JobDeliveryResult::submitted(Some(7)))
                .expect("finish retry"),
            JobOutcome::Expired
        );
        assert!(job.completed);
        assert_eq!(
            job.last_run.as_ref().map(|run| run.status),
            Some(JobRunStatus::Submitted)
        );
    }

    #[test]
    fn misfire_policy_is_explicit_and_audited() {
        let now = Utc::now();
        let overdue = now - Duration::minutes(5);

        let mut skip = ScheduledJob::new_once(overdue, "skip");
        skip.misfire_policy = MisfirePolicy::Skip;
        assert_eq!(
            skip.claim_due_delivery(now).expect("skip misfire"),
            ClaimDueDelivery::StateUpdated
        );
        assert!(skip.completed);
        assert_eq!(
            skip.last_run.as_ref().map(|run| run.status),
            Some(JobRunStatus::Skipped)
        );

        let mut run_once = ScheduledJob::new_once(overdue, "run");
        assert!(matches!(
            run_once.claim_due_delivery(now).expect("run-once misfire"),
            ClaimDueDelivery::Deliver(_)
        ));
    }
}
