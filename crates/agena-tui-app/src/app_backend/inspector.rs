//! Inspector presentation and runtime/usage summaries for the settings studios.

use super::TuiBackend;
use anyhow::{Context, Result};

/// A row in the inspector view.
#[derive(Debug, Clone)]
pub struct InspectorRow {
    pub label: String,
    pub detail: String,
}

/// Joins an optional mode display name and description into a single
/// human-readable inspector detail line.
pub(crate) fn summarize_named_mode(
    display_name: Option<&str>,
    description: Option<&str>,
) -> String {
    match (
        display_name
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        description.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        (Some(display_name), Some(description)) => format!("{display_name} · {description}"),
        (Some(display_name), None) => display_name.to_owned(),
        (None, Some(description)) => description.to_owned(),
        (None, None) => "configured mode".to_owned(),
    }
}

pub(crate) async fn usage_stats(
    application: &TuiBackend,
    query: agena_domain::UsageStatsQuery,
) -> Result<agena_domain::UsageStats> {
    application
        .client()
        .usage_stats(
            query.period,
            query.from,
            query.to,
            &query.provider_ids,
            &query.model_ids,
            &query.session_ids,
            query.include_subagents,
            query.timezone_offset_minutes,
        )
        .await
        .context("failed to load usage statistics")
}

pub(crate) async fn runtime_snapshot_summary(application: &TuiBackend) -> Result<String> {
    let status = application.refresh_runtime_status_cache().await?;
    Ok(format!(
        "generation {} · loaded {} · {} providers · {} plugins",
        status.generation,
        status.loaded_at.to_rfc3339(),
        status.provider_ids.len(),
        status.plugin_count,
    ))
}
