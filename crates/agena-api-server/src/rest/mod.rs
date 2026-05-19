//! REST handlers for the Studio/Web surface. These endpoints intentionally
//! return the plain JSON resources the current web client already consumes,
//! while WS/SSE protocol traffic continues to route through `dispatch`.

use std::{collections::BTreeSet, convert::Infallible, sync::Arc};

use crate::local_api::{
    AuthApiKeyWriteRequest, AuthAtomGitBrowserPollRequest, AuthAtomGitBrowserStartRequest,
    AuthBrowserStartRequest, AuthBrowserStartResource, AuthCopilotDevicePollRequest,
    AuthCopilotDeviceStartRequest, AuthCredentialType, AuthDeviceStartResource,
    AuthGitLabBrowserFinishRequest, AuthGitLabBrowserStartRequest, AuthLoginResultResource,
    AuthOpenAiBrowserFinishRequest, AuthOpenAiDevicePollRequest, AuthOpenAiDeviceStartRequest,
    AuthProviderResource, HealthResponse, MarketplaceInstallOutcomeResource,
    MarketplaceInstallRequestBody, MarketplaceInstalledListResponse,
    MarketplaceInstalledPluginResource, MarketplaceOutdatedListResponse,
    MarketplaceOutdatedPluginResource, MarketplacePluginResource, MarketplaceRegistryRequestBody,
    MarketplaceSearchRequestBody, MarketplaceSearchResponse, MarketplaceSyncResponse,
    MarketplaceUninstallOutcomeResource, MarketplaceUninstallRequestBody,
    MarketplaceUninstallResponse, MarketplaceUpgradeOutcomeResource, MarketplaceUpgradeRequestBody,
    MarketplaceUpgradeResponse, MessageListQuery, ModelCatalogEntryWriteRequest,
    ModelCatalogListResponse, ModelCatalogLookupRequest, ModelCatalogLookupResponse,
    ModelCatalogResponse, PartLoadMode, PermissionRuleListQuery, PermissionRuleRevokeRequest,
    PermissionRuleWriteRequest, PluginInspectResponse, PluginLogListQuery, PluginLogListResponse,
    PluginStatusListResponse, PluginUiCatalogResponse, PluginUiInvokeToolRequest,
    PluginUiRunActionRequest, RuntimeReloadResponse, SessionContinueRequestBody,
    SessionCreateRequest, SessionEventStreamQuery, SessionGoalSetRequest, SessionListQuery,
    SessionPermissionReplyRequestBody, SessionReplaceRequest, SessionRewindRequestBody,
    SessionRunOptionsRequest, SessionTurnRequest, SessionUserInputReplyRequestBody,
    WorkspaceFileTreeQuery, WorkspaceListQuery, WorkspaceResolveRequest, WorkspaceWriteRequest,
};
use agena::config::{
    ConfigError, ConfigSettingsDeleteInput, ConfigSettingsEditResponse, ConfigSettingsGetInput,
    ConfigSettingsListInput, ConfigSettingsListResponse, ConfigSettingsPatchInput,
    ConfigSettingsReadResponse, ConfigSettingsReloadResponse, ConfigSettingsSetInput,
    ConfigSettingsSource, ConfigSettingsValidateInput, ProviderAuthConfig,
    ProviderConfigCredentialStore, ResolvedProviderConfig, delete_file_setting, get_json_path,
    list_file_settings, list_json_path, patch_file_settings, provider_auth_data,
    provider_gitlab_instance_url, provider_has_gitlab_adapter, provider_supports_api_key_write,
    provider_supports_atomgit_oauth, provider_supports_copilot_device,
    provider_supports_openai_oauth, read_file_setting, set_file_setting, validate_file_settings,
};
use agena::event::{EventStore, StoreRange};
use agena::provider::auth::{AuthManager, CopilotDeployment};
use agena::session::{UsagePeriod, UsageStatsQuery};
use agena_api::{
    queries::{
        ListEventsParams, ListProviderAdapterModelsParams, ListSavedProviderAdapterModelsParams,
        Query, QueryResult,
    },
    resource::{ProviderAdapterModelsRequest, SavedProviderAdapterModelsRequest},
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
use serde_json::Value as JsonValue;

use crate::{dispatch, error::ServerError, state::AppState};

mod auth;
mod events;
mod git;
mod marketplace;
mod messages;
mod model_catalog;
mod permissions;
mod plugins;
mod providers;
mod sessions;
mod settings;
mod workspaces;

pub use events::*;
pub use messages::*;
pub use permissions::*;
pub use sessions::*;
pub use settings::*;
pub use workspaces::*;

pub use auth::*;
pub use git::*;
pub use marketplace::*;
pub use plugins::*;
pub use providers::*;

pub use model_catalog::*;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionEventListCompatQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub after_seq: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct UsageStatsHttpQuery {
    #[serde(default)]
    pub period: Option<UsagePeriod>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
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

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MessageDetailQuery {
    #[serde(default)]
    pub parts: PartLoadMode,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MessagePartsQuery {
    #[serde(default, alias = "parts")]
    pub mode: PartLoadMode,
}

pub async fn health(State(state): State<AppState>) -> Result<impl IntoResponse, ServerError> {
    let snapshot = state.runtime().current_snapshot();
    Ok(Json(HealthResponse {
        status: "ok",
        generation: snapshot.generation(),
        loaded_at: snapshot.loaded_at(),
        database_connected: true,
    }))
}

/// Lightweight liveness probe — returns 200 OK with a static body.
/// No application state is touched. Suitable for k8s livenessProbe.
pub async fn healthz() -> impl IntoResponse {
    (axum::http::StatusCode::OK, "ok")
}

/// Readiness probe — returns 200 once the runtime snapshot is loaded.
pub async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    let snapshot = state.runtime().current_snapshot();
    if snapshot.generation() > 0 {
        (axum::http::StatusCode::OK, "ready")
    } else {
        (axum::http::StatusCode::SERVICE_UNAVAILABLE, "loading")
    }
}

/// Process-wide counters surfaced via `/metrics`. These are intentionally
/// lightweight (raw atomics + bucket histograms, no real meter provider)
/// until `agena-otel` exposes a meter API.
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
/// tokens, session active counts) should land via `agena-otel` once a
/// real meter provider is wired up.
pub async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    use std::sync::atomic::Ordering;
    let snapshot = state.runtime().current_snapshot();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let uptime = now.saturating_sub(process_start_unix());
    let core_snap = agena::metrics::snapshot();

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
        generation = snapshot.generation(),
        reloads = METRIC_RUNTIME_RELOADS.load(Ordering::Relaxed),
        requests = METRIC_HTTP_REQUESTS.load(Ordering::Relaxed),
        hist = hist,
        provider_calls = core_snap.provider_calls_total,
        provider_errors = core_snap.provider_calls_error,
        provider_streams = core_snap.provider_stream_total,
        tool_total = core_snap.tool_executions_total,
        tool_errors = core_snap.tool_executions_error,
        session_active = core_snap.session_active,
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
    match dispatch::dispatch_query(&state, Query::Runtime).await? {
        QueryResult::Runtime(runtime) => Ok(Json(runtime)),
        _ => unreachable!("runtime query returned unexpected result"),
    }
}

pub async fn get_usage_stats(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<UsageStatsHttpQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let usage_query = usage_stats_query_from_http(query)?;
    Ok(Json(
        manager
            .usage_stats(usage_query)
            .await
            .map_err(ServerError::Core)?,
    ))
}

fn usage_stats_query_from_http(query: UsageStatsHttpQuery) -> Result<UsageStatsQuery, ServerError> {
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
            return Err(ServerError::BadRequest(
                "from must be earlier than or equal to to".to_string(),
            ));
        }
        return Ok(UsageStatsQuery::custom(from, to));
    }

    Ok(UsageStatsQuery::for_period(
        query.period.unwrap_or(UsagePeriod::Last7Days),
        chrono::Utc::now(),
    ))
}

fn parse_usage_datetime(
    raw: &str,
    end_of_day: bool,
) -> Result<chrono::DateTime<chrono::Utc>, ServerError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ServerError::BadRequest(
            "usage date cannot be empty".to_string(),
        ));
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
    Err(ServerError::BadRequest(format!(
        "invalid usage date `{raw}`; expected YYYY-MM-DD or RFC3339"
    )))
}

pub async fn reload_runtime(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let report = state.runtime().reload().await.map_err(ServerError::Core)?;
    METRIC_RUNTIME_RELOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(Json(RuntimeReloadResponse {
        cause: "manual",
        previous_generation: report.previous_generation,
        generation: report.generation,
        loaded_at: report.loaded_at,
    }))
}

pub async fn plugin_rpc(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<agena::plugin::sdk::rpc::Request>,
) -> Result<impl IntoResponse, ServerError> {
    let host = state.runtime().current_snapshot().plugin_manager();
    let response =
        plugin_rpc_response(host, plugin_id.as_str(), bearer_token(&headers), req).await?;
    Ok(Json(response))
}

async fn reload_runtime_from_config(state: &AppState) -> Result<(), ServerError> {
    state.runtime().reload().await.map_err(ServerError::Core)?;
    Ok(())
}

fn resolved_config_json(config: &impl serde::Serialize) -> Result<JsonValue, ServerError> {
    serde_json::to_value(config)
        .map_err(|error| ServerError::Internal(format!("failed to encode settings: {error}")))
}

async fn reload_settings_if_needed(
    state: &AppState,
    response: &mut ConfigSettingsEditResponse,
) -> Result<(), ServerError> {
    if !response.reload_required {
        return Ok(());
    }

    let report = state.runtime().reload().await.map_err(ServerError::Core)?;
    METRIC_RUNTIME_RELOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    response.reload = Some(ConfigSettingsReloadResponse {
        previous_generation: report.previous_generation,
        generation: report.generation,
        loaded_at: report.loaded_at.to_rfc3339(),
    });
    Ok(())
}

fn settings_error(error: ConfigError) -> ServerError {
    let message = error.to_string();
    match error {
        ConfigError::ReadFile { .. }
        | ConfigError::WriteFile { .. }
        | ConfigError::SerializeJson(_)
        | ConfigError::SerializeToml(_) => ServerError::Internal(message),
        ConfigError::App(error) => ServerError::Core(error),
        _ => ServerError::BadRequest(message),
    }
}

fn server_error_from_http(error: crate::local_api::ApiError) -> ServerError {
    match error.status_code() {
        axum::http::StatusCode::BAD_REQUEST => ServerError::BadRequest(error.message().to_owned()),
        axum::http::StatusCode::NOT_FOUND => ServerError::NotFound(error.message().to_owned()),
        axum::http::StatusCode::CONFLICT => ServerError::Conflict(error.message().to_owned()),
        axum::http::StatusCode::SERVICE_UNAVAILABLE => {
            ServerError::ServiceUnavailable(error.message().to_owned())
        }
        _ => ServerError::Internal(error.message().to_owned()),
    }
}

async fn resolve_run_options(
    state: &AppState,
    session_id: i64,
    request: SessionRunOptionsRequest,
) -> Result<agena::session::SessionRunOptions, ServerError> {
    let snapshot = state.runtime().current_snapshot();
    let default_model = configured_default_model(&snapshot)?;
    let manager = state.session_manager()?;
    state
        .service()
        .resolve_run_options(
            snapshot.provider_registry().as_ref(),
            default_model,
            manager.as_ref(),
            session_id,
            request,
        )
        .await
        .map_err(server_error_from_http)
}

fn configured_default_model(
    snapshot: &agena::runtime::RuntimeSnapshot,
) -> Result<Option<agena::model::ModelRef>, ServerError> {
    let default = &snapshot.config_resolution().config.default;
    let Some(provider_id) = default.provider.as_deref() else {
        return Ok(None);
    };
    let registry = snapshot.provider_registry();
    registry
        .resolve_model_selection(
            provider_id,
            default.adapter.as_deref(),
            default.model.as_deref(),
        )
        .map(Some)
        .map_err(ServerError::Core)
}

fn if_match_version(headers: &HeaderMap) -> Result<Option<i64>, ServerError> {
    let Some(value) = headers.get(IF_MATCH) else {
        return Ok(None);
    };
    let raw = value
        .to_str()
        .map_err(|error| ServerError::BadRequest(format!("invalid If-Match header: {error}")))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ServerError::BadRequest(
            "If-Match header cannot be empty".into(),
        ));
    }
    if trimmed == "*" {
        return Err(ServerError::BadRequest(
            "If-Match '*' is not supported for session version checks".into(),
        ));
    }
    if trimmed.contains(',') {
        return Err(ServerError::BadRequest(
            "If-Match must contain exactly one session version".into(),
        ));
    }

    let version_text = trimmed
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(trimmed);
    let version = version_text.parse::<i64>().map_err(|error| {
        ServerError::BadRequest(format!(
            "If-Match must be a numeric session version: {error}"
        ))
    })?;

    Ok(Some(version))
}

fn sse_error_event(message: impl Into<String>) -> Event {
    Event::default().event("error").data(message.into())
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

fn callback_context_present(params: &serde_json::Value) -> bool {
    params
        .as_object()
        .and_then(|object| object.get("context"))
        .and_then(|value| {
            serde_json::from_value::<agena::plugin::sdk::host_api::HostCallbackContext>(
                value.clone(),
            )
            .ok()
        })
        .is_some()
}

async fn plugin_rpc_response(
    host: Arc<agena::plugin::PluginHost>,
    plugin_id: &str,
    callback_token: Option<String>,
    req: agena::plugin::sdk::rpc::Request,
) -> Result<agena::plugin::sdk::rpc::Response, ServerError> {
    use agena::plugin::sdk::rpc::{ErrorObject, JsonRpcVersion, Response, ResponsePayload, codes};

    if !host
        .host_handle()
        .validate_callback_token(plugin_id, callback_token.as_deref())
        .await
    {
        return Err(ServerError::BadRequest(
            "invalid or missing plugin callback bearer token".into(),
        ));
    }

    let id = req.id.clone();
    if host.plugins().iter().all(|plugin| plugin.id != plugin_id) {
        return Ok(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Err {
                error: ErrorObject {
                    code: codes::HOST_UNAVAILABLE,
                    message: format!("unknown plugin id: {plugin_id}"),
                    data: None,
                },
            },
        });
    }

    let params = req.params.unwrap_or(serde_json::Value::Null);
    if !callback_context_present(&params) {
        return Err(ServerError::BadRequest(
            "plugin callback request is missing callback context".into(),
        ));
    }

    let handle = host.host_handle();
    match handle
        .ingest_stream_event_for_plugin(plugin_id, &req.method, params.clone())
        .await
    {
        Ok(true) => {
            return Ok(Response {
                jsonrpc: JsonRpcVersion,
                id,
                payload: ResponsePayload::Ok {
                    result: serde_json::Value::Object(Default::default()),
                },
            });
        }
        Ok(false) => {}
        Err(err) => {
            return Ok(Response {
                jsonrpc: JsonRpcVersion,
                id,
                payload: ResponsePayload::Err {
                    error: ErrorObject {
                        code: codes::PLUGIN_GENERIC,
                        message: err.message.clone(),
                        data: serde_json::to_value(&err).ok(),
                    },
                },
            });
        }
    }

    match handle
        .handle_call_for_plugin(plugin_id, &req.method, params)
        .await
    {
        Ok(result) => Ok(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Ok { result },
        }),
        Err(err) => Ok(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Err {
                error: ErrorObject {
                    code: codes::PLUGIN_GENERIC,
                    message: err.message.clone(),
                    data: serde_json::to_value(&err).ok(),
                },
            },
        }),
    }
}
