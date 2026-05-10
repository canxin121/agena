//! REST handlers for the Studio/Web surface. These endpoints intentionally
//! return the plain JSON resources the current web client already consumes,
//! while WS/SSE protocol traffic continues to route through `dispatch`.

use std::{
    collections::{BTreeSet, HashMap},
    convert::Infallible,
    sync::Arc,
};

use crate::local_api::{
    AuthApiKeyWriteRequest, AuthCredentialType, AuthProviderResource, HealthResponse,
    MarketplaceInstallOutcomeResource, MarketplaceInstallRequestBody,
    MarketplaceInstalledListResponse, MarketplaceInstalledPluginResource,
    MarketplaceOutdatedListResponse, MarketplaceOutdatedPluginResource,
    MarketplacePluginResource, MarketplaceRegistryRequestBody, MarketplaceSearchRequestBody,
    MarketplaceSearchResponse, MarketplaceSyncResponse, MarketplaceUninstallOutcomeResource,
    MarketplaceUninstallRequestBody, MarketplaceUninstallResponse,
    MarketplaceUpgradeOutcomeResource, MarketplaceUpgradeRequestBody,
    MarketplaceUpgradeResponse, MessageListQuery, PartLoadMode, PermissionRuleListQuery,
    PermissionRuleRevokeRequest, PermissionRuleWriteRequest, PluginInspectResponse,
    PluginLogListQuery, PluginLogListResponse, PluginStatusListResponse,
    RuntimeReloadResponse, SessionContinueRequestBody, SessionCreateRequest,
    SessionEventStreamQuery, SessionListQuery, SessionPermissionReplyRequestBody,
    SessionReplaceRequest, SessionRewindRequestBody, SessionRunOptionsRequest,
    SessionTurnRequest, SessionUserInputReplyRequestBody, WorkspaceFileTreeQuery,
    WorkspaceListQuery, WorkspaceResolveRequest, WorkspaceWriteRequest,
};
use agena::event::{EventStore, StoreRange};
use agena_api::queries::{ListEventsParams, Query, QueryResult};
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

use crate::{dispatch, error::ServerError, state::AppState};

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

pub async fn get_git_status(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .git_status(state.runtime())
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn list_plugins(State(state): State<AppState>) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(PluginStatusListResponse {
        entries: state
            .runtime()
            .current_snapshot()
            .plugin_manager()
            .plugin_statuses(),
    }))
}

pub async fn get_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin = state
        .runtime()
        .current_snapshot()
        .plugin_manager()
        .plugin_inspect(plugin_id.as_str())
        .ok_or_else(|| ServerError::NotFound(format!("plugin not found: {plugin_id}")))?;
    Ok(Json(PluginInspectResponse { plugin }))
}

pub async fn list_plugin_logs(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    AxumQuery(query): AxumQuery<PluginLogListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin_manager = state.runtime().current_snapshot().plugin_manager();
    if plugin_manager.plugin_status(plugin_id.as_str()).is_none() {
        return Err(ServerError::NotFound(format!(
            "plugin not found: {plugin_id}"
        )));
    }
    Ok(Json(PluginLogListResponse {
        plugin_id: plugin_id.clone(),
        entries: plugin_manager.plugin_logs(
            plugin_id.as_str(),
            query.after_seq,
            query.limit.unwrap_or(50),
        ),
    }))
}

pub async fn search_marketplace_plugins(
    State(_state): State<AppState>,
    Json(request): Json<MarketplaceSearchRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let registry_id = request
        .registry_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let registry_url = request.registry_url.trim().to_string();
    if registry_url.is_empty() {
        return Err(ServerError::BadRequest(
            "registry_url cannot be empty".to_string(),
        ));
    }

    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let registry = client.registry(agena_plugin_marketplace::RegistrySpec {
        id: registry_id.clone(),
        url: registry_url.clone(),
        require_signature: false,
    });
    let index = registry
        .fetch_index(request.refresh)
        .map_err(|error| ServerError::BadRequest(error.to_string()))?;
    let needle = request
        .query
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let mut entries = index
        .plugins
        .into_iter()
        .filter(|plugin| {
            if needle.is_empty() {
                return true;
            }
            let blob = format!("{} {} {}", plugin.id, plugin.name, plugin.description)
                .to_ascii_lowercase();
            blob.contains(&needle)
        })
        .map(|plugin| {
            let latest = plugin.versions.iter().max_by(|left, right| {
                let left_semver = semver::Version::parse(&left.version).ok();
                let right_semver = semver::Version::parse(&right.version).ok();
                match (left_semver, right_semver) {
                    (Some(left_version), Some(right_version)) => left_version.cmp(&right_version),
                    _ => left.version.cmp(&right.version),
                }
            });
            MarketplacePluginResource {
                plugin_id: plugin.id,
                name: plugin.name,
                description: plugin.description,
                homepage: plugin.homepage,
                version_count: plugin.versions.len(),
                latest_version: latest.map(|version| version.version.clone()),
                latest_kind: latest.map(|version| version.kind.as_str().to_string()),
                latest_platform: latest.map(|version| version.platform.clone()),
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));

    Ok(Json(MarketplaceSearchResponse {
        registry_id,
        registry_url,
        entries,
    }))
}

pub async fn sync_marketplace_registry(
    State(_state): State<AppState>,
    Json(request): Json<MarketplaceRegistryRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let registry_id = request
        .registry_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let registry_url = request.registry_url.trim().to_string();
    if registry_url.is_empty() {
        return Err(ServerError::BadRequest(
            "registry_url cannot be empty".to_string(),
        ));
    }

    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let registry = client.registry(agena_plugin_marketplace::RegistrySpec {
        id: registry_id.clone(),
        url: registry_url.clone(),
        require_signature: false,
    });
    let index = registry
        .fetch_index(true)
        .map_err(|error| ServerError::BadRequest(error.to_string()))?;

    Ok(Json(MarketplaceSyncResponse {
        registry_id,
        registry_url,
        plugin_count: index.plugins.len(),
    }))
}

pub async fn list_marketplace_installed_plugins(
    State(_state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let mut entries = client
        .list_installed()
        .map_err(|error| ServerError::BadRequest(error.to_string()))?
        .into_iter()
        .map(|record| MarketplaceInstalledPluginResource {
            plugin_id: record.plugin_id,
            version: record.version,
            kind: record.kind.as_str().to_string(),
            platform: record.platform,
            binary_path: record.binary_path.display().to_string(),
            config_path: record.config_path.display().to_string(),
            sha256: record.sha256,
            installed_at: record.installed_at,
            registry_id: record.registry_id,
            registry_url: record.registry_url,
            archive_extracted: record.archive_extracted,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));
    Ok(Json(MarketplaceInstalledListResponse { entries }))
}

pub async fn list_marketplace_outdated_plugins(
    State(_state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let entries = client
        .list_outdated()
        .map_err(|error| ServerError::BadRequest(error.to_string()))?
        .into_iter()
        .map(|record| MarketplaceOutdatedPluginResource {
            plugin_id: record.plugin_id,
            installed_version: record.installed_version,
            latest_version: record.latest_version,
        })
        .collect::<Vec<_>>();
    Ok(Json(MarketplaceOutdatedListResponse { entries }))
}

pub async fn install_marketplace_plugin(
    State(state): State<AppState>,
    Json(request): Json<MarketplaceInstallRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let registry_url = request.registry_url.trim().to_string();
    if registry_url.is_empty() {
        return Err(ServerError::BadRequest(
            "registry_url cannot be empty".to_string(),
        ));
    }
    let spec = request.spec.trim().to_string();
    if spec.is_empty() {
        return Err(ServerError::BadRequest("spec cannot be empty".to_string()));
    }
    let (plugin_id, version) = match spec.split_once('@') {
        Some((id, ver)) => (id.to_string(), Some(ver.to_string())),
        None => (spec, None),
    };
    let registry_id = request
        .registry_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let config_path = request
        .config_path
        .clone()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            state
                .runtime()
                .current_snapshot()
                .config_resolution()
                .meta
                .config_path
                .clone()
        });

    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let outcome = client
        .install(agena_plugin_marketplace::InstallRequest {
            registry: agena_plugin_marketplace::RegistrySpec {
                id: registry_id,
                url: registry_url,
                require_signature: request.require_signature,
            },
            plugin_id,
            version,
            config_path,
            force: request.force,
            dry_run: request.dry_run,
            allow_unverified: request.allow_unverified,
            refresh_index: request.refresh,
        })
        .map_err(|error| ServerError::BadRequest(error.to_string()))?;

    Ok(Json(MarketplaceInstallOutcomeResource {
        plugin_id: outcome.plugin_id,
        version: outcome.version,
        kind: outcome.kind.as_str().to_string(),
        artifact_path: outcome.artifact_path.display().to_string(),
        config_path: outcome.config_path.display().to_string(),
        dry_run: outcome.dry_run,
    }))
}

pub async fn uninstall_marketplace_plugin(
    State(_state): State<AppState>,
    Json(request): Json<MarketplaceUninstallRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin_id = request.plugin_id.trim().to_string();
    if plugin_id.is_empty() {
        return Err(ServerError::BadRequest(
            "plugin_id cannot be empty".to_string(),
        ));
    }
    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let entries = client
        .uninstall_with(plugin_id.as_str(), request.cascade)
        .map_err(|error| ServerError::BadRequest(error.to_string()))?
        .into_iter()
        .map(|outcome| MarketplaceUninstallOutcomeResource {
            plugin_id: outcome.plugin_id,
            version: outcome.version,
            config_path: outcome.config_path.display().to_string(),
        })
        .collect::<Vec<_>>();
    Ok(Json(MarketplaceUninstallResponse { entries }))
}

pub async fn upgrade_marketplace_plugins(
    State(_state): State<AppState>,
    Json(request): Json<MarketplaceUpgradeRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if !request.all && request.plugin_id.as_deref().unwrap_or("").trim().is_empty() {
        return Err(ServerError::BadRequest(
            "plugin_id is required unless all=true".to_string(),
        ));
    }
    let cache = agena_plugin_marketplace::MarketplaceCache::new(
        agena_plugin_marketplace::default_cache_root(),
    );
    let client = agena_plugin_marketplace::MarketplaceClient::with_default_fetcher(
        cache,
        std::collections::BTreeMap::new(),
    );
    let override_spec = request.registry_url.as_ref().map(|registry_url| {
        agena_plugin_marketplace::RegistrySpec {
            id: request
                .registry_id
                .clone()
                .unwrap_or_else(|| "default".to_string()),
            url: registry_url.trim().to_string(),
            require_signature: false,
        }
    });

    let targets = if request.all {
        client
            .list_installed()
            .map_err(|error| ServerError::BadRequest(error.to_string()))?
            .into_iter()
            .map(|record| record.plugin_id)
            .collect::<Vec<_>>()
    } else {
        vec![request.plugin_id.clone().unwrap_or_default().trim().to_string()]
    };

    let mut entries = Vec::new();
    for plugin_id in targets.into_iter().filter(|item| !item.is_empty()) {
        let outcome = client
            .upgrade(plugin_id.as_str(), override_spec.clone())
            .map_err(|error| ServerError::BadRequest(error.to_string()))?;
        entries.push(MarketplaceUpgradeOutcomeResource {
            plugin_id: outcome.plugin_id,
            previous_version: outcome.previous_version,
            installed_version: outcome.installed_version,
            upgraded: outcome.upgraded,
            outcome: outcome.outcome.map(|inner| MarketplaceInstallOutcomeResource {
                plugin_id: inner.plugin_id,
                version: inner.version,
                kind: inner.kind.as_str().to_string(),
                artifact_path: inner.artifact_path.display().to_string(),
                config_path: inner.config_path.display().to_string(),
                dry_run: inner.dry_run,
            }),
        });
    }

    Ok(Json(MarketplaceUpgradeResponse { entries }))
}

pub async fn list_auth_providers(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let configured_ids = configured_provider_ids(&state);
    let auth_map = state
        .runtime()
        .auth_manager()
        .all()
        .map_err(ServerError::Core)?;
    let provider_ids = public_auth_provider_ids(&configured_ids, &auth_map);
    let items = provider_ids
        .into_iter()
        .map(|provider_id| {
            let auth = auth_map.get(provider_id.as_str());
            auth_provider_resource(
                configured_ids.contains(provider_id.as_str()),
                provider_id,
                auth,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(items))
}

pub async fn get_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    ensure_public_auth_provider_id(provider_id.as_str())?;
    let configured = configured_provider_ids(&state);
    let auth = state
        .runtime()
        .auth_manager()
        .get(provider_id.as_str())
        .map_err(ServerError::Core)?;
    if !configured.contains(provider_id.as_str()) && auth.is_none() {
        return Err(ServerError::NotFound(format!(
            "auth provider not found: {provider_id}"
        )));
    }
    Ok(Json(auth_provider_resource(
        configured.contains(provider_id.as_str()),
        provider_id,
        auth.as_ref(),
    )?))
}

pub async fn set_auth_provider_api_key(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<AuthApiKeyWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    ensure_public_auth_provider_id(provider_id.as_str())?;
    let configured = configured_provider_ids(&state);
    let existing = state
        .runtime()
        .auth_manager()
        .get(provider_id.as_str())
        .map_err(ServerError::Core)?;
    if !configured.contains(provider_id.as_str()) && existing.is_none() {
        return Err(ServerError::NotFound(format!(
            "auth provider not found: {provider_id}"
        )));
    }

    state
        .runtime()
        .auth_manager()
        .set_api_key(provider_id.as_str(), request.api_key)
        .map_err(ServerError::Core)?;
    let auth = state
        .runtime()
        .auth_manager()
        .get(provider_id.as_str())
        .map_err(ServerError::Core)?;
    Ok(Json(auth_provider_resource(
        configured.contains(provider_id.as_str()),
        provider_id,
        auth.as_ref(),
    )?))
}

pub async fn delete_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    ensure_public_auth_provider_id(provider_id.as_str())?;
    let configured = configured_provider_ids(&state);
    let auth = state
        .runtime()
        .auth_manager()
        .get(provider_id.as_str())
        .map_err(ServerError::Core)?;
    if !configured.contains(provider_id.as_str()) && auth.is_none() {
        return Err(ServerError::NotFound(format!(
            "auth provider not found: {provider_id}"
        )));
    }

    state
        .runtime()
        .auth_manager()
        .remove(provider_id.as_str())
        .map_err(ServerError::Core)?;
    if provider_id == "gitlab" {
        state
            .runtime()
            .auth_manager()
            .remove("gitlab-instance")
            .map_err(ServerError::Core)?;
    }
    Ok(Json(auth_provider_resource(
        configured.contains(provider_id.as_str()),
        provider_id,
        None,
    )?))
}

pub async fn refresh_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    ensure_public_auth_provider_id(provider_id.as_str())?;
    let configured = configured_provider_ids(&state);
    let existing = state
        .runtime()
        .auth_manager()
        .get(provider_id.as_str())
        .map_err(ServerError::Core)?;
    if !configured.contains(provider_id.as_str()) && existing.is_none() {
        return Err(ServerError::NotFound(format!(
            "auth provider not found: {provider_id}"
        )));
    }

    let auth = match provider_id.as_str() {
        "openai" => state
            .runtime()
            .auth_manager()
            .refresh_openai_login()
            .await
            .map_err(ServerError::Core)?,
        "gitlab" => state
            .runtime()
            .auth_manager()
            .refresh_gitlab_login()
            .await
            .map_err(ServerError::Core)?,
        _ => {
            return Err(ServerError::BadRequest(format!(
                "credential refresh is not supported for provider '{provider_id}'"
            )));
        }
    };

    Ok(Json(auth_provider_resource(
        configured.contains(provider_id.as_str()),
        provider_id,
        Some(&auth),
    )?))
}

pub async fn list_providers(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_query(&state, Query::ListProviders).await? {
        QueryResult::Providers(providers) => Ok(Json(providers)),
        _ => unreachable!("providers query returned unexpected result"),
    }
}

pub async fn list_provider_models(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_query(
        &state,
        Query::ListProviderModels(agena_api::queries::ListProviderModelsParams { provider_id }),
    )
    .await?
    {
        QueryResult::ProviderModels(models) => Ok(Json(models)),
        _ => unreachable!("provider models query returned unexpected result"),
    }
}

pub async fn list_workspaces(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<WorkspaceListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .list_workspaces(query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let workspace = state
        .service()
        .get_workspace(workspace_id)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("workspace not found: {workspace_id}")))?;
    Ok(Json(workspace))
}

pub async fn create_workspace(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .create_workspace(request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn resolve_workspace(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceResolveRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .resolve_workspace(request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn replace_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
    Json(request): Json<WorkspaceWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .replace_workspace(workspace_id, request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn delete_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .delete_workspace(workspace_id)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn list_workspace_files(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
    AxumQuery(query): AxumQuery<WorkspaceFileTreeQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .list_workspace_files(workspace_id, query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SessionListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .list_sessions(query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let session = state
        .service()
        .get_session(session_id)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("session not found: {session_id}")))?;
    Ok(Json(session))
}

pub async fn get_session_state(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let session = manager
        .get_session(session_id)
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

#[tracing::instrument(skip_all)]
pub async fn create_session(
    State(state): State<AppState>,
    Json(request): Json<SessionCreateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .create_session(request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn replace_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionReplaceRequest>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }
    Ok(Json(
        state
            .service()
            .replace_session(session_id, request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn delete_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }
    Ok(Json(
        state
            .service()
            .delete_session(session_id)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn list_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionEventListCompatQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    if let Some(after_seq) = query.after_seq {
        let limit = query.limit.unwrap_or(100).clamp(1, 1000);
        let items = state
            .service()
            .list_session_events_after(manager.as_ref(), session_id, after_seq, Some(limit))
            .await
            .map_err(server_error_from_http)?;
        let returned = items.len() as u64;
        let next_cursor = items.last().map(|event| event.meta.seq_global.to_string());
        return Ok(Json(serde_json::json!({
            "items": items,
            "page": {
                "limit": limit,
                "returned": returned,
                "has_more": returned >= limit,
                "next_cursor": next_cursor,
                "order": "asc"
            }
        })));
    }

    let page = state
        .service()
        .list_session_events(
            manager.as_ref(),
            session_id,
            crate::local_api::SessionEventListQuery {
                cursor: query.cursor,
                limit: query.limit,
            },
        )
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(serde_json::to_value(page).map_err(|error| {
        ServerError::Internal(error.to_string())
    })?))
}

pub async fn stream_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionEventStreamQuery>,
) -> Result<impl IntoResponse, ServerError> {
    use agena::event::{EventFilter, Scope, bus::SubscriptionItem};

    let manager = state.session_manager()?;
    let service = state.service().clone();
    let backfill_after = query.after_seq.unwrap_or(0);
    let backfill_limit = query.limit.unwrap_or(100).clamp(1, 1000);
    let initial = service
        .list_session_events_after(
            manager.as_ref(),
            session_id,
            backfill_after,
            Some(backfill_limit),
        )
        .await
        .map_err(server_error_from_http)?;

    let bus = manager.event_bus();
    let mut subscription = bus.subscribe(EventFilter::new(Scope::Session { session_id }));

    let stream = stream! {
        for event in &initial {
            match Event::default()
                .event("session_event")
                .id(event.meta.seq_global.to_string())
                .json_data(event)
            {
                Ok(ev) => yield Ok::<Event, Infallible>(ev),
                Err(error) => {
                    yield Ok::<Event, Infallible>(sse_error_event(format!("failed to encode session event: {error}")));
                    return;
                }
            }
        }
        let mut last_seen = initial
            .last()
            .map(|event| event.meta.seq_global)
            .unwrap_or(backfill_after);

        loop {
            match subscription.recv().await {
                Some(SubscriptionItem::Event(arc_event)) => {
                    if arc_event.meta.seq_global <= last_seen {
                        continue;
                    }
                    last_seen = arc_event.meta.seq_global;
                    match Event::default()
                        .event("session_event")
                        .id(arc_event.meta.seq_global.to_string())
                        .json_data(arc_event.as_ref())
                    {
                        Ok(ev) => yield Ok::<Event, Infallible>(ev),
                        Err(error) => {
                            yield Ok::<Event, Infallible>(sse_error_event(format!("failed to encode session event: {error}")));
                            return;
                        }
                    }
                }
                Some(SubscriptionItem::Lagged(skipped)) => {
                    yield Ok::<Event, Infallible>(Event::default().event("lagged").data(skipped.to_string()));
                }
                None => return,
            }
        }
    };

    Ok(Sse::new(stream))
}

pub async fn submit_turn(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionTurnRequest>,
) -> Result<impl IntoResponse, ServerError> {
    if request.parts.is_empty() {
        return Err(ServerError::BadRequest(
            "session turn requires at least one part".into(),
        ));
    }
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let manager = state.session_manager()?;
    let session = manager
        .submit_user_turn(agena::session::SessionUserTurnRequest {
            session_id,
            options,
            parts: request.parts,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn continue_run(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionContinueRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let manager = state.session_manager()?;
    let session = manager
        .continue_session(agena::session::SessionContinueRequest {
            session_id,
            options,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn fork_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionForkRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if request.at_event_seq.is_some() && request.at_message_id.is_none() {
        return Err(ServerError::BadRequest(
            "fork expects at_message_id; at_event_seq is no longer supported".into(),
        ));
    }
    let manager = state.session_manager()?;
    let session = manager
        .fork_session(agena::session::SessionForkRequest {
            session_id,
            at_message_id: request.at_message_id,
            title: request.title,
            expected_version: if_match_version(&headers)?,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn cancel_turn(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_command(
        &state,
        agena_api::commands::Command::CancelTurn(agena_api::commands::CancelTurnParams {
            session_id,
        }),
    )
    .await?
    {
        agena_api::commands::CommandResult::Ack => Ok(Json(serde_json::json!({ "ok": true }))),
        _ => unreachable!("cancel turn returned unexpected result"),
    }
}

pub async fn reply_permission(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionPermissionReplyRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let manager = state.session_manager()?;
    let session = manager
        .reply_permission(agena::session::SessionPermissionReplyRequest {
            session_id,
            options,
            reply: request.reply,
            operator: Some("http_api".to_string()),
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn reply_user_input(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionUserInputReplyRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let manager = state.session_manager()?;
    let session = manager
        .reply_user_input(agena::session::SessionUserInputReplyRequest {
            session_id,
            options,
            reply: request.reply,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn rewind_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRewindRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let expected_version = if_match_version(&headers)?;
    let manager = state.session_manager()?;
    let session = manager
        .rewind_session(agena::session::SessionRewindRequest {
            session_id,
            message_id: request.message_id,
            expected_version,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn unrewind_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRewindRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let expected_version = if_match_version(&headers)?;
    let manager = state.session_manager()?;
    let session = manager
        .unrewind_session(agena::session::SessionUnrewindRequest {
            session_id,
            message_id: request.message_id,
            expected_version,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn list_session_tree(
    State(state): State<AppState>,
    Path(root_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let summaries = manager
        .list_session_tree(root_id)
        .await
        .map_err(ServerError::Core)?;
    let resources: Vec<crate::local_api::dto::SessionResource> =
        summaries.into_iter().map(Into::into).collect();
    Ok(Json(resources))
}

pub async fn list_rewind_checkpoints(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let checkpoints = manager
        .list_rewind_checkpoints(session_id)
        .await
        .map_err(ServerError::Core)?;
    let resources: Vec<agena_api::resource::RewindCheckpointResource> =
        checkpoints.into_iter().map(Into::into).collect();
    Ok(Json(resources))
}

pub async fn export_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let jsonl = manager
        .export_session_jsonl(session_id)
        .await
        .map_err(ServerError::Core)?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/x-ndjson")],
        jsonl,
    ))
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionImportRequestBody {
    pub jsonl: String,
}

pub async fn import_session(
    State(state): State<AppState>,
    Json(request): Json<SessionImportRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let session = manager
        .import_session_jsonl(&request.jsonl)
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn list_messages(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<MessageListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    Ok(Json(
        state
            .service()
            .list_messages(manager.as_ref(), session_id, query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_message(
    State(state): State<AppState>,
    Path(message_id): Path<i64>,
    AxumQuery(query): AxumQuery<MessageDetailQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let message = state
        .service()
        .get_message(manager.as_ref(), message_id, query.parts)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("message not found: {message_id}")))?;
    Ok(Json(message))
}

pub async fn list_message_parts(
    State(state): State<AppState>,
    Path(message_id): Path<i64>,
    AxumQuery(query): AxumQuery<MessagePartsQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    Ok(Json(
        state
            .service()
            .list_message_parts(manager.as_ref(), message_id, query.mode)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_message_part(
    State(state): State<AppState>,
    Path(part_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let part = state
        .service()
        .get_message_part(manager.as_ref(), part_id)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("message part not found: {part_id}")))?;
    Ok(Json(part))
}

pub async fn list_permission_rules(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<PermissionRuleListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .list_permission_rules(query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let rule = state
        .service()
        .get_permission_rule(rule_id)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("permission rule not found: {rule_id}")))?;
    Ok(Json(rule))
}

pub async fn create_permission_rule(
    State(state): State<AppState>,
    Json(request): Json<PermissionRuleWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .create_permission_rule(request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn replace_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
    Json(request): Json<PermissionRuleWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .replace_permission_rule(rule_id, request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn revoke_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
    Json(request): Json<PermissionRuleRevokeRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .revoke_permission_rule(rule_id, request.reason)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn delete_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .delete_permission_rule(rule_id)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn list_events(
    State(state): State<AppState>,
    AxumQuery(params): AxumQuery<ListEventsParams>,
) -> Result<impl IntoResponse, ServerError> {
    let publisher = state.event_publisher()?;
    let store: &Arc<dyn EventStore<agena::event::EventKind>> = publisher.store();
    let filter = agena::event::EventFilter {
        scope: params.scope,
        kinds: params.kinds,
        since_seq_global: params.since_seq_global,
    };
    let limit = params.limit.unwrap_or(100).clamp(1, 1000) as usize;
    let range = StoreRange {
        after_seq_global: params.since_seq_global.unwrap_or(0),
        limit,
    };
    let events = store
        .range(&filter, range)
        .await
        .map_err(|error| ServerError::Internal(error.to_string()))?;
    let returned = events.len() as u64;
    let next_cursor = events.last().map(|event| event.meta.seq_global.to_string());
    Ok(Json(serde_json::json!({
        "items": events,
        "page": {
            "next_cursor": next_cursor,
            "has_more": (returned as usize) >= limit,
            "returned": returned,
        }
    })))
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
    let manager = state.session_manager()?;
    state
        .service()
        .resolve_run_options(
            snapshot.provider_registry().as_ref(),
            manager.as_ref(),
            session_id,
            request,
        )
        .await
        .map_err(server_error_from_http)
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

fn configured_provider_ids(state: &AppState) -> BTreeSet<String> {
    state
        .runtime()
        .current_snapshot()
        .provider_registry()
        .provider_ids()
        .into_iter()
        .collect()
}

fn public_auth_provider_ids(
    configured_ids: &BTreeSet<String>,
    auth_map: &HashMap<String, agena::provider::auth::AuthData>,
) -> BTreeSet<String> {
    configured_ids
        .iter()
        .cloned()
        .chain(
            auth_map
                .keys()
                .filter(|provider_id| !is_internal_auth_provider_id(provider_id.as_str()))
                .cloned(),
        )
        .collect()
}

fn ensure_public_auth_provider_id(provider_id: &str) -> Result<(), ServerError> {
    if is_internal_auth_provider_id(provider_id) {
        return Err(ServerError::NotFound(format!(
            "auth provider not found: {provider_id}"
        )));
    }
    Ok(())
}

fn is_internal_auth_provider_id(provider_id: &str) -> bool {
    provider_id == "gitlab-instance"
}

fn auth_provider_resource(
    configured: bool,
    provider_id: String,
    auth: Option<&agena::provider::auth::AuthData>,
) -> Result<AuthProviderResource, ServerError> {
    let mut resource = AuthProviderResource {
        provider_id,
        configured,
        credential_present: auth.is_some(),
        credential_type: None,
        key_preview: None,
        expires_at: None,
        expired: None,
        account_id: None,
        enterprise_url: None,
    };

    match auth {
        Some(agena::provider::auth::AuthData::Api { key }) => {
            resource.credential_type = Some(AuthCredentialType::Api);
            resource.key_preview = secret_preview(key);
        }
        Some(agena::provider::auth::AuthData::OAuth {
            expires_at_ms,
            account_id,
            enterprise_url,
            ..
        }) => {
            resource.credential_type = Some(AuthCredentialType::Oauth);
            resource.expires_at = if *expires_at_ms > 0 {
                Some(
                    chrono::DateTime::from_timestamp_millis(*expires_at_ms).ok_or_else(|| {
                        ServerError::Internal(format!(
                            "invalid oauth expiry millis: {expires_at_ms}"
                        ))
                    })?,
                )
            } else {
                None
            };
            resource.expired = resource
                .expires_at
                .map(|expires_at| expires_at <= chrono::Utc::now());
            resource.account_id = account_id.clone();
            resource.enterprise_url = enterprise_url.clone();
        }
        Some(agena::provider::auth::AuthData::WellKnown { key, .. }) => {
            resource.credential_type = Some(AuthCredentialType::WellKnown);
            resource.key_preview = Some(key.clone());
        }
        None => {}
    }

    Ok(resource)
}

fn secret_preview(secret: &str) -> Option<String> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() <= 8 {
        return Some("*".repeat(trimmed.len()));
    }
    Some(format!(
        "{}...{}",
        &trimmed[..4],
        &trimmed[trimmed.len() - 4..]
    ))
}
