//! cron_create / cron_list / cron_delete / schedule_wakeup bundled tools.

use std::sync::Arc;

use agena_scheduler::{JobKind, ScheduledJob, Scheduler};

use crate::message::{
    CronCreateToolInput, CronDeleteToolInput, CronListToolInput, ScheduleWakeupToolInput,
};

use super::{
    BundledExecution, BundledToolOutput, CronJobSummary, ToolError, ToolExecutionView, ToolExecutor,
};

pub(super) fn execute_create(
    executor: &ToolExecutor,
    input: &CronCreateToolInput,
    session_id: Option<i64>,
) -> Result<BundledExecution, ToolError> {
    let scheduler = require_scheduler(executor)?;
    let mut job = ScheduledJob::new_cron(
        input.expression.trim(),
        input.prompt.trim(),
        input.max_age_days,
    )
    .map_err(|e| ToolError::Plugin(format!("cron_create: {e}")))?;
    if let Some(session_id) = session_id {
        job = job.with_owner(session_id);
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
    Ok(BundledExecution::new(
        BundledToolOutput::CronCreate {
            id: id.to_string(),
            next_fire_at: next,
        },
        view,
    ))
}

pub(super) fn execute_list(
    executor: &ToolExecutor,
    _input: &CronListToolInput,
) -> Result<BundledExecution, ToolError> {
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
    Ok(BundledExecution::new(
        BundledToolOutput::CronList { jobs: summaries },
        view,
    ))
}

pub(super) fn execute_delete(
    executor: &ToolExecutor,
    input: &CronDeleteToolInput,
) -> Result<BundledExecution, ToolError> {
    let scheduler = require_scheduler(executor)?;
    let id = uuid::Uuid::parse_str(input.id.trim())
        .map_err(|e| ToolError::Plugin(format!("cron_delete: invalid id: {e}")))?;
    let scheduler_for_remove = scheduler.clone();
    let removed = super::mcp::block_on(async move { scheduler_for_remove.remove(id).await });
    let view =
        ToolExecutionView::simple(format!("cron_delete {id}"), format!("removed: {removed}"));
    Ok(BundledExecution::new(
        BundledToolOutput::CronDelete {
            id: id.to_string(),
            removed,
        },
        view,
    ))
}

pub(super) fn execute_wakeup(
    executor: &ToolExecutor,
    input: &ScheduleWakeupToolInput,
    session_id: Option<i64>,
) -> Result<BundledExecution, ToolError> {
    let scheduler = require_scheduler(executor)?;
    let when = chrono::Utc::now() + chrono::Duration::seconds(input.delay_seconds as i64);
    let mut job = ScheduledJob::new_once(when, input.prompt.trim());
    if let Some(session_id) = session_id {
        job = job.with_owner(session_id);
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
    Ok(BundledExecution::new(
        BundledToolOutput::ScheduleWakeup {
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
        .ok_or_else(|| ToolError::Plugin("scheduler not configured".to_string()))
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
    }
}
