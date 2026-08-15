//! Backend adapters for the unified background-activities panel.
//!
//! The TUI is a pure HTTP client; these adapters route activity operations
//! through the server's query/command surface.

use agena_api::{
    commands::{
        Command, CommandResult, DismissActivityParams, StopActivityParams,
    },
    queries::{ActivityLogsParams, ListActivitiesParams, Query, QueryResult},
    resource::{BackgroundActivityLogResource, BackgroundActivityResource},
};
use anyhow::{Result, anyhow, bail};

pub(crate) async fn list_activities(
    application: &crate::TuiBackend,
    filter: agena_domain::BackgroundActivityFilter,
) -> Result<Vec<BackgroundActivityResource>> {
    let kinds = if filter.kinds.is_empty() {
        None
    } else {
        Some(
            filter
                .kinds
                .iter()
                .map(|kind| kind.as_str())
                .collect::<Vec<_>>()
                .join(","),
        )
    };
    let statuses = if filter.statuses.is_empty() {
        None
    } else {
        Some(
            filter
                .statuses
                .iter()
                .map(|status| status.as_str())
                .collect::<Vec<_>>()
                .join(","),
        )
    };
    let result = application
        .client()
        .query(Query::ListActivities(ListActivitiesParams {
            kinds,
            statuses,
            session_id: filter.session_id,
            active_only: filter.active_only,
        }))
        .await?;
    let QueryResult::Activities(activities) = result else {
        bail!("server returned the wrong activity-list result");
    };
    Ok(activities)
}

pub(crate) async fn activity_logs(
    application: &crate::TuiBackend,
    activity_id: &str,
    since_seq: u64,
    limit: Option<u32>,
    wait_ms: u64,
) -> Result<BackgroundActivityLogResource> {
    let result = application
        .client()
        .query(Query::ActivityLogs(ActivityLogsParams {
            activity_id: activity_id.to_owned(),
            since_seq,
            limit,
            wait_ms,
        }))
        .await
        .map_err(|error| anyhow!("failed to read background activity logs: {error}"))?;
    let QueryResult::ActivityLogs(logs) = result else {
        bail!("server returned the wrong activity-log result");
    };
    Ok(logs)
}

pub(crate) async fn control_activity(
    application: &crate::TuiBackend,
    activity_id: &str,
    action: &str,
) -> Result<BackgroundActivityResource> {
    match action {
        "stop" => {
            let result = application
                .client()
                .command(Command::StopActivity(StopActivityParams {
                    activity_id: activity_id.to_owned(),
                }))
                .await
                .map_err(|error| anyhow!("{error}"))?;
            let CommandResult::Activity(activity) = result else {
                bail!("server returned the wrong activity-control result");
            };
            Ok(activity)
        }
        "dismiss" => {
            let result = application
                .client()
                .command(Command::DismissActivity(DismissActivityParams {
                    activity_id: activity_id.to_owned(),
                }))
                .await
                .map_err(|error| anyhow!("{error}"))?;
            let CommandResult::Activity(activity) = result else {
                bail!("server returned the wrong activity-control result");
            };
            Ok(activity)
        }
        "pause" | "resume" | "delete" => application
            .client()
            .control_activity(activity_id, action)
            .await
            .map_err(|error| anyhow!("{error}")),
        _ => Err(anyhow!(
            "unsupported background activity control `{action}`"
        )),
    }
}

pub(crate) async fn clear_finished_activities(application: &crate::TuiBackend) -> Result<usize> {
    let result = application
        .client()
        .command(Command::ClearFinishedActivities)
        .await
        .map_err(|error| anyhow!("{error}"))?;
    let CommandResult::ActivitiesCleared { count } = result else {
        bail!("server returned the wrong clear-finished result");
    };
    Ok(count)
}
