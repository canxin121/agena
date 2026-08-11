//! Backend adapters for the unified background-activities panel.
//!
//! The TUI is an in-process consumer of the runtime activity service; these
//! adapters map runtime activity results to the presentation resources
//! without going through HTTP.

use agena_api::resource::{BackgroundActivityLogResource, BackgroundActivityResource};
use agena_application::Application;
use anyhow::{Context, Result, anyhow};

pub(crate) async fn list_activities(
    application: &Application,
    filter: agena_domain::BackgroundActivityFilter,
) -> Result<Vec<BackgroundActivityResource>> {
    let service = application.runtime_activities()?;
    let activities = service.list_activities(&filter);
    Ok(activities
        .iter()
        .map(agena_api::resource::BackgroundActivityResource::from)
        .collect())
}

pub(crate) async fn activity_logs(
    application: &Application,
    activity_id: &str,
    since_seq: u64,
    limit: Option<u32>,
    wait_ms: u64,
) -> Result<BackgroundActivityLogResource> {
    let service = application.runtime_activities()?;
    let read = service
        .activity_logs(activity_id, since_seq, limit, wait_ms)
        .await
        .map_err(|error| anyhow!("{error}"))
        .context("failed to read background activity logs")?;
    Ok(read.into())
}

pub(crate) async fn stop_activity(
    application: &Application,
    activity_id: &str,
) -> Result<BackgroundActivityResource> {
    let service = application.runtime_activities()?;
    let activity = service
        .stop_activity(activity_id)
        .await
        .map_err(|error| anyhow!("{error}"))
        .context("failed to stop background activity")?;
    Ok(agena_api::resource::BackgroundActivityResource::from(
        &activity,
    ))
}

pub(crate) async fn dismiss_activity(application: &Application, activity_id: &str) -> Result<()> {
    let service = application.runtime_activities()?;
    service
        .dismiss_activity(activity_id)
        .map_err(|error| anyhow!("{error}"))
        .context("failed to dismiss background activity")?;
    Ok(())
}

pub(crate) async fn clear_finished_activities(application: &Application) -> Result<usize> {
    let service = application.runtime_activities()?;
    Ok(service.clear_finished())
}
