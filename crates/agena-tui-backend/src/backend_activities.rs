//! Backend adapters for the unified background-activities panel.
//!
//! The TUI is an in-process consumer of the application dispatch surface; it
//! maps `Query`/`Command` wire types to the presentation resources without
//! going through HTTP.

use anyhow::{Context, Result, anyhow};

use agena_api::queries::{ActivityLogsParams, GetActivityParams, ListActivitiesParams};
use agena_api::resource::{
    BackgroundActivityLogResource, BackgroundActivityResource,
};
use agena_application::dispatch;

use super::{Backend, Query, QueryResult, api_error};

impl Backend {
    pub async fn list_activities(
        &self,
        filter: agena_domain::BackgroundActivityFilter,
    ) -> Result<Vec<BackgroundActivityResource>> {
        let params = ListActivitiesParams {
            kinds: csv_or_none(filter.kinds.iter().map(|kind| kind.as_str())),
            statuses: csv_or_none(filter.statuses.iter().map(|status| status.as_str())),
            session_id: filter.session_id,
            active_only: filter.active_only,
        };
        match dispatch::dispatch_query(
            &self.application,
            Query::ListActivities(params),
        )
        .await
        {
            Ok(QueryResult::Activities(items)) => Ok(items),
            Ok(other) => Err(anyhow!("unexpected query result: {:?}", other))
                .context("failed to list background activities"),
            Err(error) => Err(api_error(error)).context("failed to list background activities"),
        }
    }

    pub async fn get_activity(&self, activity_id: &str) -> Result<BackgroundActivityResource> {
        match dispatch::dispatch_query(
            &self.application,
            Query::GetActivity(GetActivityParams {
                activity_id: activity_id.to_owned(),
            }),
        )
        .await
        {
            Ok(QueryResult::Activity(activity)) => Ok(activity),
            Ok(other) => Err(anyhow!("unexpected query result: {:?}", other))
                .context("failed to get background activity"),
            Err(error) => Err(api_error(error)).context("failed to get background activity"),
        }
    }

    pub async fn activity_logs(
        &self,
        activity_id: &str,
        since_seq: u64,
        limit: Option<u32>,
        wait_ms: u64,
    ) -> Result<BackgroundActivityLogResource> {
        match dispatch::dispatch_query(
            &self.application,
            Query::ActivityLogs(ActivityLogsParams {
                activity_id: activity_id.to_owned(),
                since_seq,
                limit,
                wait_ms,
            }),
        )
        .await
        {
            Ok(QueryResult::ActivityLogs(logs)) => Ok(logs),
            Ok(other) => Err(anyhow!("unexpected query result: {:?}", other))
                .context("failed to read background activity logs"),
            Err(error) => Err(api_error(error)).context("failed to read background activity logs"),
        }
    }

    pub async fn stop_activity(&self, activity_id: &str) -> Result<BackgroundActivityResource> {
        let result = dispatch::dispatch_command(
            &self.application,
            agena_api::commands::Command::StopActivity(
                agena_api::commands::StopActivityParams {
                    activity_id: activity_id.to_owned(),
                },
            ),
        )
        .await;
        match result {
            Ok(agena_api::commands::CommandResult::Activity(activity)) => Ok(activity),
            Ok(other) => Err(anyhow!("unexpected command result: {:?}", other))
                .context("failed to stop background activity"),
            Err(error) => Err(api_error(error)).context("failed to stop background activity"),
        }
    }

    pub async fn dismiss_activity(&self, activity_id: &str) -> Result<()> {
        let result = dispatch::dispatch_command(
            &self.application,
            agena_api::commands::Command::DismissActivity(
                agena_api::commands::DismissActivityParams {
                    activity_id: activity_id.to_owned(),
                },
            ),
        )
        .await;
        match result {
            Ok(agena_api::commands::CommandResult::ActivityDeleted { .. }) => Ok(()),
            Ok(other) => Err(anyhow!("unexpected command result: {:?}", other))
                .context("failed to dismiss background activity"),
            Err(error) => Err(api_error(error)).context("failed to dismiss background activity"),
        }
    }

    pub async fn clear_finished_activities(&self) -> Result<usize> {
        let result = dispatch::dispatch_command(
            &self.application,
            agena_api::commands::Command::ClearFinishedActivities,
        )
        .await;
        match result {
            Ok(agena_api::commands::CommandResult::ActivitiesCleared { count }) => Ok(count),
            Ok(other) => Err(anyhow!("unexpected command result: {:?}", other))
                .context("failed to clear finished background activities"),
            Err(error) => {
                Err(api_error(error)).context("failed to clear finished background activities")
            }
        }
    }
}

fn csv_or_none<'a>(values: impl Iterator<Item = &'a str>) -> Option<String> {
    let joined: Vec<&str> = values.collect();
    if joined.is_empty() {
        None
    } else {
        Some(joined.join(","))
    }
}
