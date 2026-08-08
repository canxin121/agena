//! REST handlers for the Studio/Web surface. These endpoints intentionally
//! return the plain JSON resources the current web client already consumes,
//! while WS/SSE protocol traffic continues to route through `dispatch`.

use std::{convert::Infallible, future::Future};

use agena_api::{
    queries::{
        ListEventsParams, ListProviderAdapterModelsParams, ListSavedProviderAdapterModelsParams,
        Query, QueryResult,
    },
    resource::{
        PermissionReply, ProviderAdapterModelsRequest, SavedProviderAdapterModelsRequest,
        UserInputReply,
    },
};
use agena_application::dto::{
    AuthApiKeyWriteRequest, AuthBrowserStartResource, AuthCodeExchangeRequest,
    AuthDeviceStartResource, AuthEnterpriseDevicePollRequest, AuthEnterpriseDeviceRequest,
    AuthLoginResultResource, AuthProviderRequest, AuthProviderResource, AuthRedirectRequest,
    AuthUserCodeDevicePollRequest, GitCommitRequest, GitPullRequestCreateRequest, GitStageRequest,
    HealthResponse, ItemsResponse, MarketplaceInstallRequest, MarketplaceInstalledPluginResource,
    MarketplaceOutdatedPluginResource, MarketplacePluginResource, MarketplaceRegistryRequest,
    MarketplaceSearchRequest, MarketplaceSearchResponse, MarketplaceUninstallRequestBody,
    MarketplaceUpgradeRequest, MemoryWriteRequest, PermissionRuleRevokeRequest,
    PermissionRuleWriteRequest, PluginInspectResponse, PluginLogListQuery, PluginLogListResponse,
    PluginUiCatalogResponse, PluginUiInvokeToolRequest, PluginUiRequestContext,
    RuntimeBackgroundTaskCancelResponse, RuntimeBackgroundTaskStartResponse, SearchPaginationQuery,
    SessionCreateRequest, SessionEventStreamQuery, SessionListQuery, SessionMessageRequest,
    SessionReplyRequestBody, SessionRewindRequestBody, SessionRunRequestBody, SessionUpdateRequest,
    WorkspaceFileDownloadQuery, WorkspaceFileTreeQuery, WorkspaceListQuery, WorkspacePathRequest,
    WorkspaceResolveRequest,
};
use agena_domain::{UsagePeriod, UsageStatsQuery, get_json_path};
use agena_runtime::{
    ConfigSettingsDeleteInput, ConfigSettingsEditResponse, ConfigSettingsGetInput,
    ConfigSettingsListInput, ConfigSettingsListResponse, ConfigSettingsPatchInput,
    ConfigSettingsReadResponse, ConfigSettingsReloadResponse, ConfigSettingsSetInput,
    ConfigSettingsSource, ConfigSettingsValidateInput, RuntimeConfigSettingsError, list_json_path,
};
use async_stream::stream;
use axum::{
    Json,
    extract::{Path, Query as AxumQuery, State},
    http::{HeaderMap, header::IF_MATCH},
    response::{
        IntoResponse,
        sse::{Event, Sse},
    },
};
use serde::Deserialize;

use crate::{error::ServerError, state::AppState};
use agena_application::ApplicationError;
use agena_application::dispatch;

pub(crate) fn server_error_from_application(error: ApplicationError) -> ServerError {
    ServerError::from(error)
}

mod activities;
mod auth;
mod events;
mod git;
mod marketplace;
mod memory;
mod model_catalog;
pub(crate) mod notifications;
mod permissions;
mod plugins;
mod providers;
mod sessions;
mod settings;
mod workspaces;

pub use events::*;
pub use permissions::*;
pub use sessions::*;
pub use settings::*;
pub use workspaces::*;

pub use activities::*;
pub use auth::*;
pub use git::*;
pub use marketplace::*;
pub use memory::*;
pub use notifications::*;
pub use plugins::*;
pub use providers::*;

pub use model_catalog::*;

#[derive(Debug, Clone, Deserialize, Default)]
/// Compatibility query for the session event list.
pub struct SessionEventListCompatQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub after_seq: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Query for HTTP usage statistics.
pub struct UsageStatsHttpQuery {
    #[serde(default)]
    pub period: Option<UsagePeriod>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    /// Comma-separated provider ids to include.
    #[serde(default)]
    pub provider: Option<String>,
    /// Comma-separated model ids to include.
    #[serde(default)]
    pub model: Option<String>,
    /// Comma-separated session ids to include.
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub include_subagents: Option<bool>,
    /// Fixed UTC offset used for preset boundaries and daily buckets.
    #[serde(default)]
    pub timezone_offset_minutes: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
/// Body of a session fork request.
pub struct SessionForkRequestBody {
    /// Fork point. `None` clones the entire history; otherwise clones
    /// every event up to and including the last one tied to this message id.
    #[serde(default)]
    pub at_message_id: Option<i64>,
    #[serde(default)]
    pub at_event_seq: Option<i64>,
    #[serde(default)]
    pub title: Option<String>,
}

pub async fn health(State(state): State<AppState>) -> Result<impl IntoResponse, ServerError> {
    let status = state.runtime_snapshot_summary().await;
    Ok(Json(HealthResponse {
        status: "ok",
        generation: status.generation,
        loaded_at: status.loaded_at,
        database_connected: true,
    }))
}

async fn json_http<T>(
    future: impl Future<Output = Result<T, ApplicationError>>,
) -> Result<Json<T>, ServerError> {
    Ok(Json(future.await.map_err(server_error_from_application)?))
}

async fn json_http_found<T>(
    future: impl Future<Output = Result<Option<T>, ApplicationError>>,
    not_found: impl FnOnce() -> String,
) -> Result<Json<T>, ServerError> {
    let value = future
        .await
        .map_err(server_error_from_application)?
        .ok_or_else(|| {
            ServerError::not_found_with_diagnostic("The resource was not found.", not_found())
        })?;
    Ok(Json(value))
}

async fn query_json<T>(
    state: &AppState,
    query: Query,
    select: impl FnOnce(QueryResult) -> Option<T>,
    unexpected: &'static str,
) -> Result<Json<T>, ServerError> {
    let result = dispatch::dispatch_query(state, query).await?;
    match select(result) {
        Some(value) => Ok(Json(value)),
        None => unreachable!("{unexpected}"),
    }
}

fn items_json<T>(items: Vec<T>) -> Json<ItemsResponse<T>> {
    Json(ItemsResponse { items })
}

fn runtime_background_task_start_response(
    start: agena_runtime::RuntimeBackgroundTaskStart,
) -> RuntimeBackgroundTaskStartResponse {
    RuntimeBackgroundTaskStartResponse {
        started: start.started,
        task: start.task.into(),
    }
}

fn runtime_background_task_cancel_response(
    task: agena_runtime::RuntimeBackgroundTask,
) -> RuntimeBackgroundTaskCancelResponse {
    RuntimeBackgroundTaskCancelResponse { task: task.into() }
}

/// Lightweight liveness probe — returns 200 OK with a static body.
/// No application state is touched. Suitable for k8s livenessProbe.
pub async fn healthz() -> impl IntoResponse {
    (axum::http::StatusCode::OK, "ok")
}

/// Readiness probe — returns 200 once the runtime snapshot is loaded.
pub async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    if state.runtime_snapshot_summary().await.generation > 0 {
        (axum::http::StatusCode::OK, "ready")
    } else {
        (axum::http::StatusCode::SERVICE_UNAVAILABLE, "loading")
    }
}

/// Process-wide counters surfaced via `/metrics`. These are intentionally
/// lightweight (raw atomics + bucket histograms, no real meter provider)
/// until a richer metrics backend is wired up.
pub(crate) static METRIC_HTTP_REQUESTS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
pub(crate) static METRIC_RUNTIME_RELOADS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
pub(crate) static METRIC_HTTP_DURATION_SUM_MICROS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
/// Buckets in microseconds: 1ms, 5ms, 10ms, 50ms, 100ms, 500ms, 1s, 5s, 10s, +Inf
pub(crate) static HTTP_LATENCY_BUCKETS_US: [u64; 9] = [
    1_000, 5_000, 10_000, 50_000, 100_000, 500_000, 1_000_000, 5_000_000, 10_000_000,
];
pub(crate) static METRIC_HTTP_LATENCY_BUCKETS: [std::sync::atomic::AtomicU64; 10] = [
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
];
pub(crate) static METRIC_PROCESS_START_UNIX: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

/// Record one HTTP request latency observation. Called by the
/// request-counting middleware in `lib.rs`.
pub(crate) fn record_http_latency(elapsed_us: u64) {
    use std::sync::atomic::Ordering;
    METRIC_HTTP_DURATION_SUM_MICROS.fetch_add(elapsed_us, Ordering::Relaxed);
    let mut bucket = HTTP_LATENCY_BUCKETS_US.len();
    for (idx, threshold) in HTTP_LATENCY_BUCKETS_US.iter().enumerate() {
        if elapsed_us <= *threshold {
            bucket = idx;
            break;
        }
    }
    METRIC_HTTP_LATENCY_BUCKETS[bucket].fetch_add(1, Ordering::Relaxed);
}

fn process_start_unix() -> u64 {
    *METRIC_PROCESS_START_UNIX.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default()
    })
}

/// Minimal Prometheus-style metrics endpoint. Exposes a handful of process
/// counters today plus an HTTP latency histogram; richer metrics (provider
/// tokens, session active counts) can move to a real meter provider later.
pub async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    use std::sync::atomic::Ordering;
    let runtime_status = state.runtime_snapshot_summary().await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let uptime = now.saturating_sub(process_start_unix());
    let runtime_metrics = state.runtime_metrics();

    // Histogram body — Prometheus expects cumulative counts.
    let mut hist = String::new();
    let mut cumulative: u64 = 0;
    for (idx, threshold) in HTTP_LATENCY_BUCKETS_US.iter().enumerate() {
        cumulative += METRIC_HTTP_LATENCY_BUCKETS[idx].load(Ordering::Relaxed);
        let upper_seconds = (*threshold as f64) / 1_000_000.0;
        hist.push_str(&format!(
            "agena_http_request_duration_seconds_bucket{{le=\"{:}\"}} {}\n",
            upper_seconds, cumulative
        ));
    }
    cumulative +=
        METRIC_HTTP_LATENCY_BUCKETS[HTTP_LATENCY_BUCKETS_US.len()].load(Ordering::Relaxed);
    hist.push_str(&format!(
        "agena_http_request_duration_seconds_bucket{{le=\"+Inf\"}} {}\n",
        cumulative
    ));
    let total_count = cumulative;
    let total_sum_us = METRIC_HTTP_DURATION_SUM_MICROS.load(Ordering::Relaxed);
    hist.push_str(&format!(
        "agena_http_request_duration_seconds_sum {}\n",
        (total_sum_us as f64) / 1_000_000.0
    ));
    hist.push_str(&format!(
        "agena_http_request_duration_seconds_count {}\n",
        total_count
    ));

    let body = format!(
        "# HELP agena_runtime_generation monotonically-increasing config generation\n\
         # TYPE agena_runtime_generation counter\n\
         agena_runtime_generation {generation}\n\
         # HELP agena_runtime_reloads_total total runtime reloads observed\n\
         # TYPE agena_runtime_reloads_total counter\n\
         agena_runtime_reloads_total {reloads}\n\
         # HELP agena_http_requests_total total HTTP requests handled by api-server\n\
         # TYPE agena_http_requests_total counter\n\
         agena_http_requests_total {requests}\n\
         # HELP agena_http_request_duration_seconds HTTP request duration histogram\n\
         # TYPE agena_http_request_duration_seconds histogram\n\
         {hist}\
         # HELP agena_provider_calls_total total provider complete/complete_stream calls\n\
         # TYPE agena_provider_calls_total counter\n\
         agena_provider_calls_total {provider_calls}\n\
         # HELP agena_provider_calls_error_total provider calls that returned an error\n\
         # TYPE agena_provider_calls_error_total counter\n\
         agena_provider_calls_error_total {provider_errors}\n\
         # HELP agena_provider_stream_total provider streaming calls observed\n\
         # TYPE agena_provider_stream_total counter\n\
         agena_provider_stream_total {provider_streams}\n\
         # HELP agena_tool_executions_total total tool invocations\n\
         # TYPE agena_tool_executions_total counter\n\
         agena_tool_executions_total {tool_total}\n\
         # HELP agena_tool_executions_error_total tool invocations that failed\n\
         # TYPE agena_tool_executions_error_total counter\n\
         agena_tool_executions_error_total {tool_errors}\n\
         # HELP agena_session_active sessions currently being processed\n\
         # TYPE agena_session_active gauge\n\
         agena_session_active {session_active}\n\
         # HELP agena_process_uptime_seconds process uptime in seconds\n\
         # TYPE agena_process_uptime_seconds gauge\n\
         agena_process_uptime_seconds {uptime}\n\
         # HELP agena_build_info build info (always 1)\n\
         # TYPE agena_build_info gauge\n\
         agena_build_info{{version=\"{version}\"}} 1\n",
        generation = runtime_status.generation,
        reloads = METRIC_RUNTIME_RELOADS.load(Ordering::Relaxed),
        requests = METRIC_HTTP_REQUESTS.load(Ordering::Relaxed),
        hist = hist,
        provider_calls = runtime_metrics.provider_calls_total,
        provider_errors = runtime_metrics.provider_calls_error,
        provider_streams = runtime_metrics.provider_stream_total,
        tool_total = runtime_metrics.tool_executions_total,
        tool_errors = runtime_metrics.tool_executions_error,
        session_active = runtime_metrics.session_active,
        uptime = uptime,
        version = env!("CARGO_PKG_VERSION"),
    );
    (
        axum::http::StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        body,
    )
}

pub async fn get_runtime_status(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    query_json(
        &state,
        Query::Runtime,
        |result| match result {
            QueryResult::Runtime(runtime) => Some(runtime),
            _ => None,
        },
        "runtime query returned unexpected result",
    )
    .await
}

pub async fn get_usage_stats(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<UsageStatsHttpQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let usage_query = usage_stats_query_from_http(query)?;
    let session_services = state.application().session_execution_services()?;
    Ok(Json(
        session_services
            .queries
            .usage_stats(usage_query)
            .await
            .map_err(|error| ServerError::internal(error.to_string()))?,
    ))
}

fn usage_stats_query_from_http(query: UsageStatsHttpQuery) -> Result<UsageStatsQuery, ServerError> {
    let timezone_offset_minutes = query.timezone_offset_minutes.unwrap_or_default();
    if !(-1_439..=1_439).contains(&timezone_offset_minutes) {
        return Err(ServerError::bad_request(
            "The timezone offset must be between -1439 and 1439 minutes.",
        ));
    }
    let provider_ids = parse_usage_csv(query.provider.as_deref());
    let model_ids = parse_usage_csv(query.model.as_deref());
    let session_ids = parse_usage_session_ids(query.session.as_deref())?;
    let include_subagents = query.include_subagents.unwrap_or(true);
    let has_custom_range = query.from.is_some() || query.to.is_some();
    if has_custom_range {
        let from = query
            .from
            .as_deref()
            .map(|value| parse_usage_datetime(value, false))
            .transpose()?;
        let to = query
            .to
            .as_deref()
            .map(|value| parse_usage_datetime(value, true))
            .transpose()?
            .or_else(|| Some(chrono::Utc::now()));
        if let (Some(from), Some(to)) = (from.as_ref(), to.as_ref())
            && from > to
        {
            return Err(ServerError::bad_request(
                "The start date must be earlier than or equal to the end date.",
            ));
        }
        return Ok(UsageStatsQuery::custom(from, to)
            .with_timezone_offset(timezone_offset_minutes)
            .with_filters(provider_ids, model_ids, session_ids, include_subagents));
    }

    Ok(UsageStatsQuery::for_period_with_offset(
        query.period.unwrap_or(UsagePeriod::Last7Days),
        chrono::Utc::now(),
        timezone_offset_minutes,
    )
    .with_filters(provider_ids, model_ids, session_ids, include_subagents))
}

fn parse_usage_csv(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_usage_session_ids(raw: Option<&str>) -> Result<Vec<i64>, ServerError> {
    parse_usage_csv(raw)
        .into_iter()
        .map(|value| {
            value.parse::<i64>().map_err(|_| {
                ServerError::bad_request_with_diagnostic(
                    "A session ID in the usage filter is invalid.",
                    format!("invalid session id `{value}` in usage filter"),
                )
            })
        })
        .collect()
}

fn parse_usage_datetime(
    raw: &str,
    end_of_day: bool,
) -> Result<chrono::DateTime<chrono::Utc>, ServerError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ServerError::bad_request("The usage date cannot be empty."));
    }
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Ok(parsed.with_timezone(&chrono::Utc));
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        let datetime = if end_of_day {
            date.and_hms_milli_opt(23, 59, 59, 999)
        } else {
            date.and_hms_milli_opt(0, 0, 0, 0)
        }
        .expect("valid date boundary");
        return Ok(datetime.and_utc());
    }
    Err(ServerError::bad_request_with_diagnostic(
        "The usage date must use YYYY-MM-DD or RFC3339 format.",
        format!("invalid usage date `{raw}`"),
    ))
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod usage_query_tests {
    use super::{UsagePeriod, UsageStatsHttpQuery, usage_stats_query_from_http};

    #[test]
    fn http_usage_query_maps_filters_and_timezone() {
        let query = usage_stats_query_from_http(UsageStatsHttpQuery {
            period: Some(UsagePeriod::Last14Days),
            provider: Some("openai, anthropic".to_string()),
            model: Some("gpt-5".to_string()),
            session: Some("12,35".to_string()),
            include_subagents: Some(false),
            timezone_offset_minutes: Some(480),
            ..UsageStatsHttpQuery::default()
        })
        .expect("valid usage query");

        assert_eq!(query.period, UsagePeriod::Last14Days);
        assert_eq!(query.provider_ids, ["anthropic", "openai"]);
        assert_eq!(query.model_ids, ["gpt-5"]);
        assert_eq!(query.session_ids, [12, 35]);
        assert!(!query.include_subagents);
        assert_eq!(query.timezone_offset_minutes, 480);
    }

    #[test]
    fn http_usage_query_rejects_invalid_timezone_and_session() {
        let timezone_error = usage_stats_query_from_http(UsageStatsHttpQuery {
            timezone_offset_minutes: Some(1_440),
            ..UsageStatsHttpQuery::default()
        })
        .expect_err("invalid timezone");
        assert_eq!(timezone_error.status(), axum::http::StatusCode::BAD_REQUEST);

        let session_error = usage_stats_query_from_http(UsageStatsHttpQuery {
            session: Some("12,nope".to_string()),
            ..UsageStatsHttpQuery::default()
        })
        .expect_err("invalid session id");
        assert_eq!(session_error.status(), axum::http::StatusCode::BAD_REQUEST);
    }
}

pub async fn reload_runtime(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let task = state
        .runtime_control()
        .start_runtime_reload_task(
            agena_runtime::RuntimeReloadCause::Manual,
            agena_runtime::RuntimeBackgroundTaskOrigin::User,
        )
        .map_err(server_error_from_runtime_background_task)?;
    if task.started {
        METRIC_RUNTIME_RELOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(Json(runtime_background_task_start_response(task)))
}

pub async fn list_runtime_background_tasks(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(items_json(
        state
            .runtime_control()
            .background_tasks()
            .into_iter()
            .map(Into::into)
            .collect::<Vec<agena_application::dto::RuntimeBackgroundTaskResource>>(),
    ))
}

pub async fn cancel_runtime_background_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let task = state
        .runtime_control()
        .cancel_background_task(task_id.trim())
        .map_err(server_error_from_runtime_background_task)?;
    Ok(Json(runtime_background_task_cancel_response(task)))
}

pub async fn plugin_rpc(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<agena_plugin_host::sdk::rpc::Request>,
) -> Result<impl IntoResponse, ServerError> {
    let response = state
        .plugin_runtime()
        .plugin_rpc(plugin_id.as_str(), bearer_token(&headers), req)
        .await
        .map_err(|error| {
            ServerError::bad_request_with_diagnostic(
                "The plugin request could not be completed.",
                error,
            )
        })?;
    Ok(Json(response))
}

fn server_error_from_runtime_background_task(
    error: agena_runtime::RuntimeBackgroundTaskControlError,
) -> ServerError {
    match error {
        agena_runtime::RuntimeBackgroundTaskControlError::Shutdown => {
            ServerError::service_unavailable("runtime is shutting down".to_owned())
        }
        agena_runtime::RuntimeBackgroundTaskControlError::NotFound(task_id) => {
            ServerError::not_found_with_diagnostic(
                "The background task was not found.",
                format!("background task `{task_id}` not found"),
            )
        }
        agena_runtime::RuntimeBackgroundTaskControlError::NotRunning(task_id) => {
            ServerError::conflict_with_diagnostic(
                "The background task is not running.",
                format!("background task `{task_id}` is not running"),
            )
        }
        agena_runtime::RuntimeBackgroundTaskControlError::NotCancellable(task_id) => {
            ServerError::conflict_with_diagnostic(
                "The background task cannot be cancelled.",
                format!("background task `{task_id}` cannot be cancelled"),
            )
        }
    }
}

async fn reload_settings_if_needed(
    state: &AppState,
    response: &mut ConfigSettingsEditResponse,
) -> Result<(), ServerError> {
    if !response.reload_required {
        return Ok(());
    }

    let report = state
        .runtime_control()
        .reload()
        .await
        .map_err(|error| ServerError::internal(error.to_string()))?;
    METRIC_RUNTIME_RELOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    response.reload = Some(ConfigSettingsReloadResponse {
        previous_generation: report.previous_generation,
        generation: report.generation,
        loaded_at: report.loaded_at.to_rfc3339(),
    });
    Ok(())
}

fn settings_error(error: RuntimeConfigSettingsError) -> ServerError {
    ServerError::from_failure_with_diagnostic(
        error.failure().clone(),
        error.diagnostic().to_owned(),
    )
}

fn if_match_version(headers: &HeaderMap) -> Result<Option<i64>, ServerError> {
    let Some(value) = headers.get(IF_MATCH) else {
        return Ok(None);
    };
    let raw = value.to_str().map_err(|error| {
        ServerError::bad_request_with_diagnostic("The If-Match header is invalid.", error)
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ServerError::bad_request("If-Match header cannot be empty"));
    }
    if trimmed == "*" {
        return Err(ServerError::bad_request(
            "If-Match '*' is not supported for session version checks",
        ));
    }
    if trimmed.contains(',') {
        return Err(ServerError::bad_request(
            "If-Match must contain exactly one session version",
        ));
    }

    let version_text = trimmed
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(trimmed);
    let version = version_text.parse::<i64>().map_err(|error| {
        ServerError::bad_request_with_diagnostic(
            "The If-Match header must contain a numeric session version.",
            error,
        )
    })?;

    Ok(Some(version))
}

fn sse_error_event(diagnostic: impl std::fmt::Display) -> Event {
    let error = agena_api::ApiError::internal("SSE event serialization failed");
    tracing::error!(
        failure_id = %error.problem.id,
        failure_code = %error.problem.code,
        diagnostic = %diagnostic,
        "failed to serialize SSE event"
    );
    let data = serde_json::to_string(&error).unwrap_or_else(|serialization_error| {
        tracing::error!(
            diagnostic = %serialization_error,
            "failed to serialize safe SSE failure envelope"
        );
        "{\"problem\":{\"user\":{\"fallback\":\"The response could not be delivered.\"}}}"
            .to_owned()
    });
    Event::default().event("error").data(data)
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .trim();
    let mut parts = raw.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if !scheme.eq_ignore_ascii_case("bearer") || parts.next().is_some() {
        return None;
    }
    Some(token.to_string())
}
