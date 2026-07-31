//! cron_create / cron_list / cron_delete / schedule_wakeup plugin tools.

use std::sync::Arc;

use agena_scheduler::{JobKind, MisfirePolicy, RetryPolicy, ScheduledJob, Scheduler};
use agena_tool::CronRunSummary;

use crate::message::{
    CronCreateToolInput, CronDeleteToolInput, CronHistoryToolInput, CronJobControlToolInput,
    CronListToolInput, CronMisfirePolicyInput, CronRetryPolicyInput, CronUpdateToolInput,
    ScheduleWakeupToolInput,
};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};
use agena_tool::CronJobSummary;

pub(super) fn execute_create(
    executor: &ToolExecutor,
    input: &CronCreateToolInput,
    session_id: Option<i64>,
) -> Result<ToolPayloadExecution, ToolError> {
    let scheduler = require_scheduler(executor)?;
    let mut job = ScheduledJob::new_cron(
        input.expression.trim(),
        input.prompt.trim(),
        input.max_age_days,
    )
    .map_err(|e| ToolError::plugin(format!("cron_create: {e}")))?;
    job.set_recovery_policy(
        scheduler_misfire_policy(input.misfire_policy),
        scheduler_retry_policy(&input.retry_policy),
    );
    if let Some(session_id) = session_id {
        job.set_owner(session_id);
    }
    let id = job.id;
    let next = job.next_fire_at.map(|t| t.to_rfc3339());
    super::mcp::block_on(async move { scheduler.add(job).await });

    let view = ToolExecutionView::simple(
        format!("cron_create {id}"),
        format!(
            "scheduled cron `{}` -> {:?}",
            input.expression,
            next.as_deref()
        ),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::CronCreate {
            id: id.to_string(),
            next_fire_at: next,
        },
        view,
    ))
}

pub(super) fn execute_list(
    executor: &ToolExecutor,
    _input: &CronListToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let scheduler = require_scheduler(executor)?;
    let scheduler_for_list = scheduler.clone();
    let jobs = super::mcp::block_on(async move { scheduler_for_list.list().await });
    let summaries: Vec<CronJobSummary> = jobs.into_iter().map(summarize).collect();
    let summary_text = if summaries.is_empty() {
        "no scheduled jobs".to_string()
    } else {
        let mut s = format!("{} job(s):\n", summaries.len());
        for j in &summaries {
            use std::fmt::Write as _;
            let _ = writeln!(
                &mut s,
                "  {} [{}] next={:?}: {}",
                j.id, j.kind, j.next_fire_at, j.prompt
            );
        }
        s
    };
    let view = ToolExecutionView::simple("cron_list", summary_text);
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::CronList { jobs: summaries },
        view,
    ))
}

pub(super) fn execute_delete(
    executor: &ToolExecutor,
    input: &CronDeleteToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let scheduler = require_scheduler(executor)?;
    let id = uuid::Uuid::parse_str(input.id.trim())
        .map_err(|e| ToolError::plugin(format!("cron_delete: invalid id: {e}")))?;
    let scheduler_for_remove = scheduler.clone();
    let removed = super::mcp::block_on(async move { scheduler_for_remove.remove(id).await });
    let view =
        ToolExecutionView::simple(format!("cron_delete {id}"), format!("removed: {removed}"));
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::CronDelete {
            id: id.to_string(),
            removed,
        },
        view,
    ))
}

pub(super) fn execute_update(
    executor: &ToolExecutor,
    input: &CronUpdateToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    if input.prompt.is_none()
        && input.expression.is_none()
        && input.max_age_days.is_none()
        && input.misfire_policy.is_none()
        && input.retry_policy.is_none()
    {
        return Err(ToolError::plugin(
            "cron_update: provide prompt, expression, max_age_days, misfire_policy, or retry_policy"
                .to_string(),
        ));
    }
    let scheduler = require_scheduler(executor)?;
    let id = parse_job_id("cron_update", input.id.as_str())?;
    let prompt = normalized_optional(input.prompt.clone());
    let expression = normalized_optional(input.expression.clone());
    let max_age_days = input.max_age_days;
    let misfire_policy = input.misfire_policy.map(scheduler_misfire_policy);
    let retry_policy = input.retry_policy.as_ref().map(scheduler_retry_policy);
    let updated = super::mcp::block_on(async move {
        scheduler
            .update(
                id,
                prompt,
                expression,
                max_age_days,
                misfire_policy,
                retry_policy,
            )
            .await
    })
    .map_err(|error| ToolError::plugin(format!("cron_update: {error}")))?
    .ok_or_else(|| ToolError::plugin(format!("cron_update: job {id} was not found")))?;
    let summary = summarize(updated);
    let view = ToolExecutionView::simple(
        format!("cron_update {id}"),
        format!("updated job {id}; next={:?}", summary.next_fire_at),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::CronUpdate { job: summary },
        view,
    ))
}

pub(super) fn execute_pause(
    executor: &ToolExecutor,
    input: &CronJobControlToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let scheduler = require_scheduler(executor)?;
    let id = parse_job_id("cron_pause", input.id.as_str())?;
    let job = super::mcp::block_on(async move { scheduler.pause(id).await })
        .map_err(|error| ToolError::plugin(format!("cron_pause: {error}")))?
        .ok_or_else(|| ToolError::plugin(format!("cron_pause: job {id} was not found")))?;
    let summary = summarize(job);
    let view = ToolExecutionView::simple(
        format!("cron_pause {id}"),
        format!("paused job {id}: {}", summary.paused),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::CronPause { job: summary },
        view,
    ))
}

pub(super) fn execute_resume(
    executor: &ToolExecutor,
    input: &CronJobControlToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let scheduler = require_scheduler(executor)?;
    let id = parse_job_id("cron_resume", input.id.as_str())?;
    let job = super::mcp::block_on(async move { scheduler.resume(id).await })
        .map_err(|error| ToolError::plugin(format!("cron_resume: {error}")))?
        .ok_or_else(|| ToolError::plugin(format!("cron_resume: job {id} was not found")))?;
    let summary = summarize(job);
    let view = ToolExecutionView::simple(
        format!("cron_resume {id}"),
        if summary.completed {
            format!("job {id} is terminal and cannot resume")
        } else {
            format!("resumed job {id}; next={:?}", summary.next_fire_at)
        },
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::CronResume { job: summary },
        view,
    ))
}

pub(super) fn execute_history(
    executor: &ToolExecutor,
    input: &CronHistoryToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let scheduler = require_scheduler(executor)?;
    let filter_id = input
        .id
        .as_deref()
        .map(|id| parse_job_id("cron_history", id))
        .transpose()?;
    let scheduler_for_history = scheduler.clone();
    let mut entries = super::mcp::block_on(async move {
        scheduler_for_history
            .history(filter_id, input.limit as usize)
            .await
    })
    .into_iter()
    .map(|entry| CronRunSummary {
        job_id: entry.job_id.to_string(),
        triggered_at: entry.record.triggered_at.to_rfc3339(),
        finished_at: entry.record.finished_at.to_rfc3339(),
        status: format!("{:?}", entry.record.status).to_ascii_lowercase(),
        scheduled_for: entry.record.scheduled_for.map(|time| time.to_rfc3339()),
        delivery_key: entry.record.delivery_key,
        attempt: entry.record.attempt,
        session_id: entry.record.session_id,
        failure: entry.record.failure.map(Into::into),
    })
    .collect::<Vec<_>>();
    // A database-backed ledger has a stable newest-first order.  Keep this
    // deterministic for in-memory/embedded stores as well before returning
    // the JSON payload that callers can export without choosing a host path.
    entries.sort_by(|left, right| right.finished_at.cmp(&left.finished_at));
    let view = ToolExecutionView::simple(
        "cron_history",
        if entries.is_empty() {
            "no scheduler history".to_string()
        } else {
            format!("{} scheduler run record(s)", entries.len())
        },
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::CronHistory { entries },
        view,
    ))
}

pub(super) fn execute_wakeup(
    executor: &ToolExecutor,
    input: &ScheduleWakeupToolInput,
    session_id: Option<i64>,
) -> Result<ToolPayloadExecution, ToolError> {
    let scheduler = require_scheduler(executor)?;
    let when = chrono::Utc::now() + chrono::Duration::seconds(input.delay_seconds as i64);
    let mut job = ScheduledJob::new_once(when, input.prompt.trim());
    if let Some(session_id) = session_id {
        job.set_owner(session_id);
    }
    let id = job.id;
    let next = job.next_fire_at.map(|t| t.to_rfc3339()).unwrap_or_default();
    super::mcp::block_on(async move { scheduler.add(job).await });

    let view = ToolExecutionView::simple(
        format!("schedule_wakeup {id}"),
        format!(
            "wake-up scheduled at {next}{}",
            input
                .reason
                .as_deref()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default()
        ),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::ScheduleWakeup {
            id: id.to_string(),
            next_fire_at: next,
        },
        view,
    ))
}

fn require_scheduler(executor: &ToolExecutor) -> Result<Arc<Scheduler>, ToolError> {
    executor
        .scheduler()
        .cloned()
        .ok_or_else(|| ToolError::plugin("scheduler not configured".to_string()))
}

fn parse_job_id(operation: &str, id: &str) -> Result<uuid::Uuid, ToolError> {
    uuid::Uuid::parse_str(id.trim())
        .map_err(|error| ToolError::plugin(format!("{operation}: invalid id: {error}")))
}

fn normalized_optional(value: Option<String>) -> Option<String> {
    value.map(|value| value.trim().to_string())
}

fn summarize(j: ScheduledJob) -> CronJobSummary {
    let (kind, expression, at) = match &j.kind {
        JobKind::Cron { expression, .. } => ("cron", Some(expression.clone()), None),
        JobKind::Once { at } => ("once", None, Some(at.to_rfc3339())),
    };
    CronJobSummary {
        id: j.id.to_string(),
        kind: kind.to_string(),
        expression,
        at,
        prompt: j.prompt,
        next_fire_at: j.next_fire_at.map(|t| t.to_rfc3339()),
        last_fired_at: j.last_fired_at.map(|t| t.to_rfc3339()),
        paused: j.paused,
        completed: j.completed,
        misfire_policy: format!("{:?}", j.misfire_policy).to_ascii_lowercase(),
        retry_max_attempts: j.retry_policy.max_attempts,
        retry_at: j.retry_at.map(|time| time.to_rfc3339()),
        run_count: j.run_history.len() as u32,
        last_run_status: j
            .last_run
            .as_ref()
            .map(|run| format!("{:?}", run.status).to_ascii_lowercase()),
        last_run_failure: j.last_run.and_then(|run| run.failure).map(Into::into),
    }
}

fn scheduler_misfire_policy(input: CronMisfirePolicyInput) -> MisfirePolicy {
    match input {
        CronMisfirePolicyInput::Skip => MisfirePolicy::Skip,
        CronMisfirePolicyInput::RunOnceNow => MisfirePolicy::RunOnceNow,
        CronMisfirePolicyInput::Reschedule => MisfirePolicy::Reschedule,
    }
}

fn scheduler_retry_policy(input: &CronRetryPolicyInput) -> RetryPolicy {
    RetryPolicy {
        max_attempts: input.max_attempts,
        initial_delay_seconds: input.initial_delay_seconds,
        max_delay_seconds: input.max_delay_seconds,
        multiplier: input.multiplier,
    }
}
