use std::{collections::HashSet, env, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use agena::config::ConfigLoader;
use agena::runtime::AgenaRuntime;
use agena::storage::StorageConfig;
use agena::tracing as tracing_config;
use agena_api_server::AppState as ApiV2State;
use anyhow::{Context, Result, anyhow};
#[cfg(test)]
use axum::body::Body;
#[cfg(test)]
use axum::http::StatusCode;
use axum::{
    Json, Router,
    body::to_bytes,
    extract::Query,
    http::{
        HeaderMap, HeaderValue, Method,
        header::{self, HeaderName},
    },
    middleware,
    response::Response,
    routing::{get, post},
};
use axum_extra::extract::cookie::SameSite;
use futures_util::stream::{self as futures_stream, StreamExt as _};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(test)]
use std::{path::Path, process::Command};
use tokio::sync::RwLock;
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use url::Url;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) ui_auth: crate::ui_auth::UiAuth,
    pub(crate) ui_cookie_same_site: SameSite,
    pub(crate) cors_allowed_origins: Vec<String>,
    pub(crate) cors_allow_all: bool,
    pub(crate) opencode: Arc<crate::opencode::OpenCodeManager>,
    pub(crate) plugin_runtime: Arc<crate::plugin_runtime::PluginRuntime>,
    pub(crate) terminal: Arc<crate::terminal::TerminalManager>,
    pub(crate) attachment_cache: Arc<crate::attachment_cache::AttachmentCacheManager>,
    pub(crate) session_activity: crate::session_activity::SessionActivityManager,
    pub(crate) directory_session_index:
        crate::directory_session_index::DirectorySessionIndexManager,
    pub(crate) workspace_preview_registry:
        Arc<crate::workspace_preview_registry::WorkspacePreviewRegistry>,
    pub(crate) workspace_preview_runtime:
        Arc<crate::workspace_preview_runtime::WorkspacePreviewRuntime>,
    pub(crate) studio_db: Arc<crate::studio_db::StudioDb>,
    pub(crate) settings: Arc<RwLock<crate::settings::Settings>>,
    pub(crate) runtime: AgenaRuntime,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StudioHealthResponse {
    status: &'static str,
    generation: u64,
    loaded_at: String,
    workspace_root: String,
    config_path: String,
    config_found: bool,
    provider_ids: Vec<String>,
    session_runtime_available: bool,
}

async fn health(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<StudioHealthResponse> {
    let snapshot = state.runtime.current_snapshot();
    let resolution = snapshot.config_resolution();

    Json(StudioHealthResponse {
        status: "ok",
        generation: snapshot.generation(),
        loaded_at: snapshot.loaded_at().to_rfc3339(),
        workspace_root: state.runtime.workspace_root().display().to_string(),
        config_path: resolution.meta.config_path.display().to_string(),
        config_found: resolution.meta.config_found,
        provider_ids: resolution.config.providers.keys().cloned().collect(),
        session_runtime_available: state.runtime.session_manager().is_some(),
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsQuery {
    directory: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticPathEntry {
    path: String,
    exists: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsResponse {
    timestamp: String,
    opencode: Value,
    paths: Value,
    environment: Value,
}

fn diag_entry(path: PathBuf) -> DiagnosticPathEntry {
    let text = path.to_string_lossy().into_owned();
    let exists = std::fs::metadata(&path).is_ok();
    DiagnosticPathEntry { path: text, exists }
}

fn parse_opencode_cli_version(raw: &str) -> Option<String> {
    for token in raw.split_whitespace().rev() {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        let starts_with_digit = trimmed
            .chars()
            .next()
            .map(|ch| ch.is_ascii_digit())
            .unwrap_or(false);
        if starts_with_digit {
            return Some(trimmed.to_string());
        }
    }
    None
}

async fn detect_opencode_cli_version() -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_millis(1600),
        tokio::process::Command::new("opencode")
            .arg("--version")
            .output(),
    )
    .await
    .ok()?
    .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    parse_opencode_cli_version(&stdout)
        .or_else(|| parse_opencode_cli_version(&stderr))
        .or_else(|| (!stdout.is_empty()).then_some(stdout))
}

async fn agena_studio_diagnostics(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Query(query): Query<DiagnosticsQuery>,
) -> Json<DiagnosticsResponse> {
    let oc = state.opencode.status().await;
    let bridge = state.opencode.bridge().await;
    let opencode_cli_version = detect_opencode_cli_version().await;

    let normalized_directory = query
        .directory
        .as_deref()
        .map(crate::path_utils::normalize_directory_path)
        .and_then(|text| {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            }
        });

    let config_store = crate::opencode_config::OpenCodeConfigStore::from_env();
    let config_paths = config_store.get_config_paths(normalized_directory.as_deref());

    Json(DiagnosticsResponse {
        timestamp: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        opencode: json!({
            "status": {
                "port": oc.port,
                "ready": oc.ready,
                "restarting": oc.restarting,
                "lastError": oc.last_error,
                "lastErrorInfo": oc.last_error_info,
                "baseUrl": bridge.as_ref().map(|item| item.base_url.clone()),
            },
            "version": {
                "cli": opencode_cli_version,
            }
        }),
        paths: json!({
            "input": {
                "directory": query.directory,
                "normalizedDirectory": normalized_directory.as_ref().map(|path| path.to_string_lossy().into_owned())
            },
            "studio": {
                "dbPath": diag_entry(crate::persistence_paths::studio_db_path()),
                "dbCandidates": crate::persistence_paths::studio_db_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "settingsPath": diag_entry(crate::persistence_paths::studio_settings_path()),
                "settingsCandidates": crate::persistence_paths::studio_settings_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "sidebarPreferencesPath": diag_entry(crate::persistence_paths::sidebar_preferences_path()),
                "sidebarPreferencesCandidates": crate::persistence_paths::sidebar_preferences_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "terminalUiStatePath": diag_entry(crate::persistence_paths::terminal_ui_state_path()),
                "terminalUiStateCandidates": crate::persistence_paths::terminal_ui_state_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "terminalRegistryPath": diag_entry(crate::persistence_paths::terminal_session_registry_path()),
                "terminalRegistryCandidates": crate::persistence_paths::terminal_session_registry_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>()
            },
            "opencodeStorage": {
                "dataDir": diag_entry(crate::path_utils::opencode_data_dir()),
                "dataDirCandidates": crate::persistence_paths::opencode_data_dir_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "dbPath": diag_entry(crate::persistence_paths::opencode_db_path()),
                "dbCandidates": crate::persistence_paths::opencode_db_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "sessionsDir": diag_entry(crate::persistence_paths::opencode_sessions_dir()),
                "sessionsDirCandidates": crate::persistence_paths::opencode_sessions_dir_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "messagesDir": diag_entry(crate::persistence_paths::opencode_messages_dir()),
                "messagesDirCandidates": crate::persistence_paths::opencode_messages_dir_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "messagePartsDir": diag_entry(crate::persistence_paths::opencode_message_parts_dir()),
                "messagePartsDirCandidates": crate::persistence_paths::opencode_message_parts_dir_candidates().into_iter().map(diag_entry).collect::<Vec<_>>()
            },
            "opencodeConfig": {
                "userPath": diag_entry(config_paths.user_path.clone()),
                "projectPath": config_paths.project_path.as_ref().cloned().map(diag_entry),
                "customPath": config_paths.custom_path.as_ref().cloned().map(diag_entry)
            }
        }),
        environment: json!({
            "HOME": std::env::var("HOME").ok(),
            "USERPROFILE": std::env::var("USERPROFILE").ok(),
            "APPDATA": std::env::var("APPDATA").ok(),
            "LOCALAPPDATA": std::env::var("LOCALAPPDATA").ok(),
            "AGENA_STUDIO_DATA_DIR": std::env::var("AGENA_STUDIO_DATA_DIR").ok(),
            "OPENCODE_CONFIG": std::env::var("OPENCODE_CONFIG").ok(),
        }),
    })
}

fn tracked_status_directories(settings: &crate::settings::Settings) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    for project in settings.projects.iter() {
        let raw = project.path.trim();
        if raw.is_empty() {
            continue;
        }
        let Some(normalized) = crate::path_utils::normalize_directory_for_match(raw) else {
            continue;
        };
        let key = normalized.trim();
        if key.is_empty() {
            continue;
        }
        if seen.insert(key.to_string()) {
            out.push(key.to_string());
        }
    }
    out
}

const SESSION_ACTIVITY_IDLE_RETENTION: Duration = Duration::from_secs(30 * 60);
const SESSION_RUNTIME_IDLE_RETENTION: Duration = Duration::from_secs(30 * 60);
const STATUS_RECONCILE_FETCH_CONCURRENCY: usize = 6;
const STATUS_HYDRATE_LOOKUP_MAX_IDS: usize = 200;
const STATUS_HYDRATE_RESPONSE_BODY_LIMIT: usize = 4 * 1024 * 1024;
const OPENCODE_BOOTSTRAP_READY_TIMEOUT: Duration = Duration::from_secs(20);
const OPENCODE_BOOTSTRAP_RETRY_DELAY: Duration = Duration::from_secs(3);

async fn fetch_session_status_map(
    bridge: &crate::opencode::OpenCodeBridge,
    directory: Option<&str>,
) -> Option<Value> {
    let mut target = format!("{}/session/status", bridge.base_url.trim_end_matches('/'));
    if let Some(directory) = directory {
        let normalized = crate::path_utils::normalize_directory_for_match(directory);
        let trimmed = normalized.as_deref().unwrap_or(directory).trim();
        if !trimmed.is_empty() {
            target.push_str("?directory=");
            target.push_str(&urlencoding::encode(trimmed));
        }
    }

    let resp = bridge.client.get(target).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<Value>().await.ok()
}

fn read_session_id_from_value(value: &Value) -> Option<String> {
    value
        .as_object()
        .and_then(|obj| {
            obj.get("sessionID")
                .or_else(|| obj.get("sessionId"))
                .or_else(|| obj.get("session_id"))
                .and_then(Value::as_str)
        })
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn collect_attention_session_ids_from_value(
    value: &Value,
    depth: usize,
    out: &mut HashSet<String>,
) {
    if depth > 8 {
        return;
    }

    if let Some(session_id) = read_session_id_from_value(value) {
        out.insert(session_id);
    }

    match value {
        Value::Array(arr) => {
            for item in arr {
                collect_attention_session_ids_from_value(item, depth + 1, out);
            }
        }
        Value::Object(obj) => {
            for key in [
                "items",
                "data",
                "value",
                "payload",
                "permissions",
                "questions",
                "results",
            ] {
                if let Some(nested) = obj.get(key) {
                    collect_attention_session_ids_from_value(nested, depth + 1, out);
                }
            }
        }
        _ => {}
    }
}

fn parse_attention_session_ids(payload: &Value) -> HashSet<String> {
    let mut out = HashSet::<String>::new();
    collect_attention_session_ids_from_value(payload, 0, &mut out);
    out
}

async fn fetch_attention_session_ids(
    bridge: &crate::opencode::OpenCodeBridge,
    endpoint: &str,
    directory: Option<&str>,
) -> Option<HashSet<String>> {
    let mut target = format!("{}{}", bridge.base_url.trim_end_matches('/'), endpoint);
    if let Some(directory) = directory {
        let normalized = crate::path_utils::normalize_directory_for_match(directory);
        let trimmed = normalized.as_deref().unwrap_or(directory).trim();
        if !trimmed.is_empty() {
            let separator = if target.contains('?') { '&' } else { '?' };
            target.push(separator);
            target.push_str("directory=");
            target.push_str(&urlencoding::encode(trimmed));
        }
    }
    let resp = bridge.client.get(target).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let payload = resp.json::<Value>().await.ok()?;
    Some(parse_attention_session_ids(&payload))
}

async fn decode_json_response_payload(response: Response) -> Option<Value> {
    if !response.status().is_success() {
        return None;
    }
    let body = to_bytes(response.into_body(), STATUS_HYDRATE_RESPONSE_BODY_LIMIT)
        .await
        .ok()?;
    serde_json::from_slice::<Value>(&body).ok()
}

fn extract_sessions_from_payload(payload: &Value) -> Vec<Value> {
    if let Some(arr) = payload.as_array() {
        return arr.to_vec();
    }
    payload
        .get("sessions")
        .and_then(Value::as_array)
        .map(|arr| arr.to_vec())
        .unwrap_or_default()
}

fn parse_session_id(value: &Value) -> Option<String> {
    value
        .get("id")
        .and_then(Value::as_str)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

async fn hydrate_runtime_session_directory_mappings(
    state: &Arc<AppState>,
    session_ids: &HashSet<String>,
) {
    let mut missing = session_ids
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| {
            state
                .directory_session_index
                .directory_for_session(value)
                .is_none()
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return;
    }
    missing.sort();
    missing.dedup();
    if missing.len() > STATUS_HYDRATE_LOOKUP_MAX_IDS {
        missing.truncate(STATUS_HYDRATE_LOOKUP_MAX_IDS);
    }

    let directories = {
        let settings = state.settings.read().await;
        settings
            .projects
            .iter()
            .map(|project| project.path.trim().to_string())
            .filter(|path| !path.is_empty())
            .collect::<Vec<_>>()
    };
    if directories.is_empty() {
        return;
    }

    let mut unresolved = missing.into_iter().collect::<HashSet<_>>();
    for directory in directories {
        if unresolved.is_empty() {
            break;
        }

        let ids_csv = unresolved.iter().cloned().collect::<Vec<_>>().join(",");
        if ids_csv.is_empty() {
            break;
        }

        let response = match crate::opencode_session::session_list(
            axum::extract::State(state.clone()),
            HeaderMap::new(),
            Query(crate::opencode_session::SessionListQuery {
                directory: Some(directory.clone()),
                scope: Some("directory".to_string()),
                roots: None,
                start: None,
                search: None,
                offset: None,
                limit: None,
                include_total: None,
                include_children: None,
                ids: Some(ids_csv),
                focus_session_id: None,
            }),
        )
        .await
        {
            Ok(response) => response,
            Err(_) => continue,
        };

        let Some(payload) = decode_json_response_payload(response).await else {
            continue;
        };

        for session in extract_sessions_from_payload(&payload) {
            state
                .directory_session_index
                .upsert_summary_from_value(&session);
            if let Some(session_id) = parse_session_id(&session) {
                unresolved.remove(&session_id);
            }
        }
    }
}

fn reconcile_runtime_attention_from_sets(
    index: &crate::directory_session_index::DirectorySessionIndexManager,
    permission_session_ids: &HashSet<String>,
    question_session_ids: &HashSet<String>,
) {
    let mut scope = HashSet::<String>::new();
    if let Some(runtime_map) = index.runtime_snapshot_json().as_object() {
        scope.extend(runtime_map.keys().cloned());
    }
    scope.extend(permission_session_ids.iter().cloned());
    scope.extend(question_session_ids.iter().cloned());

    reconcile_runtime_attention_from_sets_scoped(
        index,
        permission_session_ids,
        question_session_ids,
        &scope,
    );
}

fn reconcile_runtime_attention_from_sets_scoped(
    index: &crate::directory_session_index::DirectorySessionIndexManager,
    permission_session_ids: &HashSet<String>,
    question_session_ids: &HashSet<String>,
    scope_session_ids: &HashSet<String>,
) {
    for session_id in scope_session_ids {
        let sid = session_id.trim();
        if sid.is_empty() {
            continue;
        }

        if permission_session_ids.contains(sid) {
            index.upsert_runtime_attention(sid, Some("permission"));
        } else if question_session_ids.contains(sid) {
            index.upsert_runtime_attention(sid, Some("question"));
        } else {
            index.upsert_runtime_attention(sid, None);
        }
    }
}

async fn reconcile_runtime_attention_from_opencode(
    state: &Arc<AppState>,
    bridge: &crate::opencode::OpenCodeBridge,
    directories: &[String],
) -> HashSet<String> {
    if directories.is_empty() {
        let permissions = fetch_attention_session_ids(bridge, "/permission", None).await;
        let questions = fetch_attention_session_ids(bridge, "/question", None).await;
        let (Some(permission_session_ids), Some(question_session_ids)) = (permissions, questions)
        else {
            return HashSet::new();
        };

        reconcile_runtime_attention_from_sets(
            &state.directory_session_index,
            &permission_session_ids,
            &question_session_ids,
        );

        let mut all_attention = permission_session_ids;
        all_attention.extend(question_session_ids);
        return all_attention;
    }

    let mut all_attention = HashSet::<String>::new();
    let directory_list = directories.to_vec();
    let tasks = futures_stream::iter(directory_list.into_iter().map(|directory| {
        let bridge = bridge.clone();
        async move {
            let (permissions, questions) = tokio::join!(
                fetch_attention_session_ids(&bridge, "/permission", Some(&directory)),
                fetch_attention_session_ids(&bridge, "/question", Some(&directory)),
            );
            (directory, permissions, questions)
        }
    }))
    .buffer_unordered(STATUS_RECONCILE_FETCH_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;

    for (directory, permissions, questions) in tasks {
        let (Some(permission_session_ids), Some(question_session_ids)) = (permissions, questions)
        else {
            continue;
        };

        let mut scope = state
            .directory_session_index
            .session_ids_for_directory(&directory);
        scope.extend(permission_session_ids.iter().cloned());
        scope.extend(question_session_ids.iter().cloned());
        if !scope.is_empty() {
            reconcile_runtime_attention_from_sets_scoped(
                &state.directory_session_index,
                &permission_session_ids,
                &question_session_ids,
                &scope,
            );
        }

        all_attention.extend(permission_session_ids);
        all_attention.extend(question_session_ids);
    }

    all_attention
}

async fn reconcile_runtime_status_from_opencode(state: &Arc<AppState>) {
    let oc = state.opencode.status().await;
    if oc.restarting || !oc.ready {
        return;
    }

    let Some(bridge) = state.opencode.bridge().await else {
        return;
    };

    let directories = {
        let settings = state.settings.read().await;
        tracked_status_directories(&settings)
    };

    let mut status_reconciled = false;
    let mut sessions_requiring_directory_hydration = HashSet::<String>::new();

    if directories.is_empty() {
        if let Some(payload) = fetch_session_status_map(&bridge, None).await {
            let busy = state
                .directory_session_index
                .reconcile_runtime_status_map(&payload);
            state.session_activity.reconcile_busy_set(&busy);
            sessions_requiring_directory_hydration.extend(busy);
            status_reconciled = true;
        }
    } else {
        let mut busy = HashSet::<String>::new();
        let mut scope = HashSet::<String>::new();
        let mut failed_fetches = 0usize;

        let status_directories = directories.clone();
        let tasks = futures_stream::iter(status_directories.into_iter().map(|directory| {
            let bridge = bridge.clone();
            async move {
                let payload = fetch_session_status_map(&bridge, Some(&directory)).await;
                (directory, payload)
            }
        }))
        .buffer_unordered(STATUS_RECONCILE_FETCH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

        for (directory, payload) in tasks {
            let Some(payload) = payload else {
                failed_fetches += 1;
                continue;
            };
            let local_busy = state
                .directory_session_index
                .merge_runtime_status_map(&payload);
            scope.extend(
                state
                    .directory_session_index
                    .session_ids_for_directory(&directory),
            );
            scope.extend(local_busy.iter().cloned());
            busy.extend(local_busy);
        }

        if !scope.is_empty() {
            state
                .directory_session_index
                .reconcile_busy_set_scoped(&busy, &scope);
            state
                .session_activity
                .reconcile_busy_set_scoped(&busy, &scope);
            sessions_requiring_directory_hydration.extend(busy.iter().cloned());
            status_reconciled = true;
        }

        if (failed_fetches > 0 || scope.is_empty())
            && let Some(payload) = fetch_session_status_map(&bridge, None).await
        {
            let busy = state
                .directory_session_index
                .reconcile_runtime_status_map(&payload);
            state.session_activity.reconcile_busy_set(&busy);
            sessions_requiring_directory_hydration.extend(busy);
            status_reconciled = true;
        }
    }

    let attention_session_ids =
        reconcile_runtime_attention_from_opencode(state, &bridge, &directories).await;
    if !attention_session_ids.is_empty() {
        sessions_requiring_directory_hydration.extend(attention_session_ids);
    }

    if !sessions_requiring_directory_hydration.is_empty() {
        hydrate_runtime_session_directory_mappings(state, &sessions_requiring_directory_hydration)
            .await;
    }

    if !status_reconciled {
        tracing::debug!(
            target: "agena_studio.runtime.reconcile",
            "skipped runtime status reconciliation (no usable status payload)"
        );
    }
}

fn spawn_opencode_bootstrap_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            if let Err(err) = state.opencode.start_if_needed().await {
                tracing::warn!(
                    target: "agena_studio.opencode",
                    error = %err,
                    "failed to start OpenCode during startup bootstrap"
                );
            }

            match state
                .opencode
                .ensure_ready(OPENCODE_BOOTSTRAP_READY_TIMEOUT)
                .await
            {
                Ok(()) => {
                    if let Err(err) = state
                        .plugin_runtime
                        .refresh_from_opencode_config_layers(None)
                        .await
                    {
                        tracing::warn!(
                            target: "agena_studio.plugin_runtime",
                            error = %err,
                            "failed to refresh plugin runtime after OpenCode became ready"
                        );
                    }
                    break;
                }
                Err(err) => {
                    tracing::warn!(
                        target: "agena_studio.opencode",
                        error = %err,
                        retry_after_secs = OPENCODE_BOOTSTRAP_RETRY_DELAY.as_secs(),
                        "OpenCode not ready yet; will retry"
                    );
                    tokio::time::sleep(OPENCODE_BOOTSTRAP_RETRY_DELAY).await;
                }
            }
        }
    });
}

fn fs_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/fs/home", get(crate::fs::fs_home))
        .route("/api/fs/list", get(crate::fs::fs_list))
        .route("/api/fs/search", get(crate::fs::fs_search))
        .route("/api/fs/search-content", post(crate::fs::fs_content_search))
        .route(
            "/api/fs/replace-content",
            post(crate::fs::fs_content_replace),
        )
        .route("/api/fs/read", get(crate::fs::fs_read))
        .route("/api/fs/read-chunk", get(crate::fs::fs_read_chunk))
        .route("/api/fs/write", post(crate::fs::fs_write))
        .route("/api/fs/mkdir", post(crate::fs::fs_mkdir))
        .route("/api/fs/delete", post(crate::fs::fs_delete))
        .route("/api/fs/rename", post(crate::fs::fs_rename))
        .route("/api/fs/raw", get(crate::fs::fs_raw))
        .route("/api/fs/download", get(crate::fs::fs_download))
}

async fn session_activity_get(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<Value> {
    reconcile_runtime_status_from_opencode(&state).await;
    state
        .session_activity
        .prune_stale_idle_entries(SESSION_ACTIVITY_IDLE_RETENTION);
    state
        .directory_session_index
        .prune_stale_runtime_entries(SESSION_RUNTIME_IDLE_RETENTION);

    Json(state.session_activity.snapshot_json())
}

fn normalize_origin_str(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let Ok(url) = Url::parse(trimmed) else {
        return None;
    };
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

fn build_cors_layer(origins: &[String], allow_all: bool) -> Option<CorsLayer> {
    let allow_headers = [
        header::ACCEPT,
        header::CONTENT_TYPE,
        header::AUTHORIZATION,
        header::IF_MATCH,
        header::IF_NONE_MATCH,
        HeaderName::from_static("last-event-id"),
    ];
    let allow_methods = [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::PATCH,
        Method::OPTIONS,
    ];

    if allow_all {
        return Some(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_credentials(false)
                .allow_headers(allow_headers)
                .allow_methods(allow_methods)
                .max_age(std::time::Duration::from_secs(60 * 60)),
        );
    }

    if origins.is_empty() {
        return None;
    }

    let mut values: Vec<HeaderValue> = Vec::new();
    for origin in origins {
        let Ok(value) = HeaderValue::from_str(origin) else {
            tracing::warn!(origin = %origin, "ignoring invalid CORS origin");
            continue;
        };
        values.push(value);
    }

    if values.is_empty() {
        return None;
    }

    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(values))
            .allow_credentials(true)
            .allow_headers(allow_headers)
            .allow_methods(allow_methods)
            .max_age(std::time::Duration::from_secs(60 * 60)),
    )
}

fn resolve_same_site(mode: crate::UiCookieSameSite, has_cross_origin: bool) -> SameSite {
    match mode {
        crate::UiCookieSameSite::Strict => SameSite::Strict,
        crate::UiCookieSameSite::Lax => SameSite::Lax,
        crate::UiCookieSameSite::None => SameSite::None,
        crate::UiCookieSameSite::Auto => {
            if has_cross_origin {
                SameSite::None
            } else {
                SameSite::Strict
            }
        }
    }
}

pub(crate) async fn run(args: crate::Args) -> Result<()> {
    let mut normalized_cors_origins = Vec::<String>::new();
    for raw in &args.cors_origin {
        let Some(origin) = normalize_origin_str(raw) else {
            tracing::warn!(origin = %raw, "ignoring invalid CORS origin");
            continue;
        };
        normalized_cors_origins.push(origin);
    }
    normalized_cors_origins.sort();
    normalized_cors_origins.dedup();

    let database_url = StorageConfig {
        database_url: args.database_url.clone(),
        database_path: args.database_path.clone(),
    }
    .resolve_url()
    .map_err(|e| anyhow!("{e}"))?;
    StorageConfig::ensure_parent(database_url.as_str()).map_err(|e| anyhow!("{e}"))?;

    let workspace_root = args
        .workspace_root
        .clone()
        .unwrap_or(env::current_dir().context("failed to resolve current working directory")?);
    let tracing = ConfigLoader::default()
        .load(&args.load_request())
        .map(|resolution| resolution.config.tracing)
        .unwrap_or_default();
    let db = Arc::new(
        tracing_config::connect_database(database_url.as_str(), &tracing)
            .await
            .with_context(|| format!("failed to connect to database {database_url}"))?,
    );

    let runtime = AgenaRuntime::builder()
        .with_load_request(args.load_request())
        .with_workspace_root(workspace_root)
        .with_database_connection(db.as_ref().clone())
        .build()
        .await
        .context("failed to build agena runtime")?;

    let ui_auth = crate::ui_auth::init_ui_auth(args.ui_password.clone());
    let studio_db = Arc::new(
        crate::studio_db::StudioDb::open()
            .await
            .map_err(|error| anyhow!("failed to open agena studio database: {error}"))?,
    );
    let settings_value = crate::settings::init_settings(studio_db.as_ref()).await;
    let configured_opencode_port = args.opencode_port;
    let should_bootstrap_opencode = configured_opencode_port.is_some() || !args.skip_opencode_start;
    let opencode = Arc::new(crate::opencode::OpenCodeManager::new(
        args.opencode_host.clone(),
        configured_opencode_port,
        args.skip_opencode_start,
        args.opencode_log_level,
        Some(crate::opencode::format_http_base_url(&args.host, args.port)),
        ui_auth.clone(),
    ));
    let terminal = Arc::new(crate::terminal::TerminalManager::new(studio_db.clone()).await);
    terminal.clone().spawn_cleanup_task();
    let attachment_cache = Arc::new(crate::attachment_cache::AttachmentCacheManager::new(
        studio_db.clone(),
    ));
    let plugin_runtime = Arc::new(crate::plugin_runtime::PluginRuntime::new());
    let _ = plugin_runtime
        .refresh_from_opencode_config_layers(None)
        .await;
    let session_activity = crate::session_activity::SessionActivityManager::new();
    let directory_session_index =
        crate::directory_session_index::DirectorySessionIndexManager::new();
    let workspace_preview_registry = Arc::new(
        crate::workspace_preview_registry::WorkspacePreviewRegistry::new(studio_db.clone()),
    );
    let workspace_preview_runtime = Arc::new(
        crate::workspace_preview_runtime::WorkspacePreviewRuntime::new(
            workspace_preview_registry.clone(),
        ),
    );

    let shared_state = Arc::new(AppState {
        ui_auth: ui_auth.clone(),
        ui_cookie_same_site: resolve_same_site(
            args.ui_cookie_samesite.clone(),
            args.cors_allow_all || !normalized_cors_origins.is_empty(),
        ),
        cors_allowed_origins: normalized_cors_origins.clone(),
        cors_allow_all: args.cors_allow_all,
        opencode,
        plugin_runtime,
        terminal,
        attachment_cache,
        session_activity,
        directory_session_index,
        workspace_preview_registry,
        workspace_preview_runtime,
        studio_db,
        settings: Arc::new(RwLock::new(settings_value)),
        runtime: runtime.clone(),
    });
    let _ = crate::ui_auth::spawn_cleanup_sessions_task_if_enabled(&shared_state.ui_auth);

    if should_bootstrap_opencode {
        spawn_opencode_bootstrap_task(shared_state.clone());
    } else {
        tracing::info!(
            target: "agena_studio.opencode",
            "OpenCode bootstrap disabled (--skip-opencode-start without --opencode-port)"
        );
    }

    {
        let state = shared_state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(4)).await;
                reconcile_runtime_status_from_opencode(&state).await;
                state
                    .session_activity
                    .prune_stale_idle_entries(SESSION_ACTIVITY_IDLE_RETENTION);
                state
                    .directory_session_index
                    .prune_stale_runtime_entries(SESSION_RUNTIME_IDLE_RETENTION);
            }
        });
    }

    let public_router = Router::new()
        .route("/health", get(health))
        .route(
            "/auth/session",
            get(crate::ui_auth::auth_session_status).post(crate::ui_auth::auth_session_create),
        )
        .with_state(shared_state.clone());

    let agena_api = agena_api_server::router(ApiV2State::new(runtime.clone(), db.clone())).layer(
        middleware::from_fn_with_state(shared_state.clone(), crate::ui_auth::require_ui_auth),
    );
    let studio_api_routes = fs_router()
        .route("/api/fs/upload", post(crate::fs::fs_upload))
        .route(
            "/api/config/reload",
            post(crate::config::config_reload_post),
        )
        .route(
            "/api/config/settings",
            get(crate::config::config_settings_get).put(crate::config::config_settings_put),
        )
        .route(
            "/api/provider/{provider_id}/source",
            get(crate::providers::provider_source_get),
        )
        .route(
            "/api/provider/env/check",
            post(crate::providers::env_check_post),
        )
        .route(
            "/api/config/settings/events",
            get(crate::settings_events::config_settings_events),
        )
        .route(
            "/api/config/opencode",
            get(crate::config::config_opencode_get).put(crate::config::config_opencode_put),
        )
        .route("/api/plugins", get(crate::plugin_runtime::plugins_list_get))
        .route(
            "/api/plugins/{plugin_id}/manifest",
            get(crate::plugin_runtime::plugin_manifest_get),
        )
        .route(
            "/api/plugins/{plugin_id}/action",
            post(crate::plugin_runtime::plugin_action_post),
        )
        .route(
            "/api/plugins/{plugin_id}/events",
            get(crate::plugin_runtime::plugin_events_get),
        )
        .route(
            "/api/plugins/{plugin_id}/assets/{*asset_path}",
            get(crate::plugin_runtime::plugin_asset_get),
        )
        .route(
            "/api/event",
            get(crate::opencode_proxy::proxy_opencode_sse_event),
        )
        .route(
            "/api/global/event",
            get(crate::global_sse_hub::global_event_sse),
        )
        .route(
            "/api/global/ws",
            get(crate::global_sse_hub::global_event_ws),
        )
        .route(
            "/api/chat-sidebar/state",
            get(crate::chat_sidebar::chat_sidebar_state),
        )
        .route(
            "/api/chat-sidebar/commands",
            post(crate::chat_sidebar::chat_sidebar_commands_post),
        )
        .route(
            "/api/chat-sidebar/search",
            get(crate::chat_sidebar::chat_sidebar_session_search),
        )
        .route(
            "/api/chat-sidebar/footer",
            get(crate::chat_sidebar::chat_sidebar_footer_get),
        )
        .route(
            "/api/sessions/summaries",
            get(crate::chat_sidebar::sessions_summaries_get),
        )
        .route(
            "/api/directories",
            get(crate::chat_sidebar::directories_get),
        )
        .route(
            "/api/directories/{directory_id}/sessions",
            get(crate::chat_sidebar::directory_sessions_by_id_get),
        )
        .route(
            "/api/session",
            get(crate::opencode_session::session_list).post(crate::opencode_session::session_post),
        )
        .route("/api/session-activity", get(session_activity_get))
        .route(
            "/api/agena-studio/session-locate",
            get(crate::opencode_proxy::agena_studio_session_locate),
        )
        .route(
            "/api/session/status",
            get(crate::opencode_proxy::session_status_get),
        )
        .route(
            "/api/session/{session_id}/message",
            get(crate::opencode_session::session_message_get)
                .post(crate::opencode_proxy::session_message_post),
        )
        .route(
            "/api/session/{session_id}/message/{message_id}/part/{part_id}",
            get(crate::opencode_session::session_message_part_get),
        )
        .route("/api/lsp", get(crate::opencode_proxy::lsp_list))
        .route("/api/mcp", get(crate::opencode_proxy::mcp_status))
        .route(
            "/api/permission",
            get(crate::opencode_proxy::permission_list),
        )
        .route("/api/question", get(crate::opencode_proxy::question_list))
        .route(
            "/api/workspace/preview",
            get(crate::workspace_preview::workspace_preview_get),
        )
        .route(
            "/api/workspace/preview-url",
            get(crate::workspace_preview::workspace_preview_url_get),
        )
        .route(
            "/api/workspace/preview/proxy",
            get(crate::workspace_preview::workspace_preview_proxy_get),
        )
        .route(
            "/api/workspace/preview/sessions",
            get(crate::workspace_preview::workspace_preview_sessions_get)
                .post(crate::workspace_preview::workspace_preview_sessions_post),
        )
        .route(
            "/api/workspace/preview/sessions/{id}",
            get(crate::workspace_preview::workspace_preview_sessions_by_id_get)
                .delete(crate::workspace_preview::workspace_preview_sessions_delete)
                .put(crate::workspace_preview::workspace_preview_sessions_put),
        )
        .route(
            "/api/workspace/preview/sessions/{id}/rename",
            post(crate::workspace_preview::workspace_preview_sessions_rename_post),
        )
        .route(
            "/api/workspace/preview/sessions/{id}/start",
            post(crate::workspace_preview::workspace_preview_sessions_start_post),
        )
        .route(
            "/api/workspace/preview/sessions/{id}/stop",
            post(crate::workspace_preview::workspace_preview_sessions_stop_post),
        )
        .route(
            "/api/workspace/preview/sessions/discover",
            post(crate::workspace_preview::workspace_preview_sessions_discover_post),
        )
        .route(
            "/api/workspace/preview/s/{id}",
            axum::routing::any(crate::workspace_preview::workspace_preview_session_proxy_root),
        )
        .route(
            "/api/workspace/preview/s/{id}/",
            axum::routing::any(crate::workspace_preview::workspace_preview_session_proxy_root),
        )
        .route(
            "/api/workspace/preview/s/{id}/{*path}",
            axum::routing::any(crate::workspace_preview::workspace_preview_session_proxy_path),
        )
        .route(
            "/api/agena-studio/update-check",
            get(crate::updates::update_check),
        )
        .route(
            "/api/agena-studio/diagnostics",
            get(agena_studio_diagnostics),
        )
        .route("/api/git/status", get(crate::git::git_status))
        .route("/api/git/watch", get(crate::git::git_watch))
        .route("/api/git/blame", get(crate::git::git_blame))
        .route("/api/git/diff", get(crate::git::git_diff))
        .route("/api/git/file-diff", get(crate::git::git_file_diff))
        .route(
            "/api/git/commit-file-diff",
            get(crate::git::git_commit_file_diff),
        )
        .route(
            "/api/git/commit-file-content",
            get(crate::git::git_commit_file_content),
        )
        .route(
            "/api/git/conflicts/file",
            get(crate::git::git_conflict_file),
        )
        .route(
            "/api/git/conflicts/resolve",
            post(crate::git::git_conflict_resolve),
        )
        .route("/api/git/patch", post(crate::git::git_apply_patch))
        .route("/api/git/check", get(crate::git::git_check))
        .route("/api/git/repos", get(crate::git::git_repos))
        .route(
            "/api/git/safe-directory",
            post(crate::git::git_safe_directory),
        )
        .route("/api/git/init", post(crate::git::git_init))
        .route("/api/git/clone", post(crate::git::git_clone))
        .route(
            "/api/git/gpg/enable-preset-passphrase",
            post(crate::git::git_gpg_enable_preset_passphrase),
        )
        .route(
            "/api/git/gpg/disable-signing",
            post(crate::git::git_gpg_disable_signing),
        )
        .route(
            "/api/git/gpg/set-signing-key",
            post(crate::git::git_gpg_set_signing_key),
        )
        .route("/api/git/remote-info", get(crate::git::git_remote_info))
        .route(
            "/api/git/remotes",
            post(crate::git::git_remote_add)
                .put(crate::git::git_remote_rename)
                .delete(crate::git::git_remote_remove),
        )
        .route(
            "/api/git/remotes/set-url",
            post(crate::git::git_remote_set_url),
        )
        .route("/api/git/signing-info", get(crate::git::git_signing_info))
        .route("/api/git/state", get(crate::git::git_state))
        .route("/api/git/merge/abort", post(crate::git::git_merge_abort))
        .route("/api/git/rebase/abort", post(crate::git::git_rebase_abort))
        .route("/api/git/stash", get(crate::git::git_stash_list))
        .route("/api/git/stash/show", get(crate::git::git_stash_show))
        .route("/api/git/stash/push", post(crate::git::git_stash_push))
        .route("/api/git/stash/apply", post(crate::git::git_stash_apply))
        .route("/api/git/stash/pop", post(crate::git::git_stash_pop))
        .route("/api/git/stash/drop", post(crate::git::git_stash_drop))
        .route(
            "/api/git/stash/drop-all",
            post(crate::git::git_stash_drop_all),
        )
        .route("/api/git/stash/branch", post(crate::git::git_stash_branch))
        .route(
            "/api/git/rebase/continue",
            post(crate::git::git_rebase_continue),
        )
        .route("/api/git/rebase/skip", post(crate::git::git_rebase_skip))
        .route(
            "/api/git/cherry-pick/abort",
            post(crate::git::git_cherry_pick_abort),
        )
        .route(
            "/api/git/cherry-pick/continue",
            post(crate::git::git_cherry_pick_continue),
        )
        .route(
            "/api/git/cherry-pick/skip",
            post(crate::git::git_cherry_pick_skip),
        )
        .route("/api/git/cherry-pick", post(crate::git::git_cherry_pick))
        .route("/api/git/revert/abort", post(crate::git::git_revert_abort))
        .route(
            "/api/git/revert/continue",
            post(crate::git::git_revert_continue),
        )
        .route("/api/git/revert/skip", post(crate::git::git_revert_skip))
        .route(
            "/api/git/revert-commit",
            post(crate::git::git_revert_commit),
        )
        .route("/api/git/merge", post(crate::git::git_merge))
        .route("/api/git/rebase", post(crate::git::git_rebase))
        .route(
            "/api/git/remote-branches",
            get(crate::git::git_remote_branches_list),
        )
        .route("/api/git/compare", get(crate::git::git_compare))
        .route("/api/git/lfs", get(crate::git::git_lfs_status))
        .route("/api/git/lfs/install", post(crate::git::git_lfs_install))
        .route("/api/git/lfs/track", post(crate::git::git_lfs_track))
        .route("/api/git/lfs/locks", get(crate::git::git_lfs_locks))
        .route("/api/git/lfs/lock", post(crate::git::git_lfs_lock))
        .route("/api/git/lfs/unlock", post(crate::git::git_lfs_unlock))
        .route("/api/git/submodules", get(crate::git::git_submodules))
        .route(
            "/api/git/submodules/add",
            post(crate::git::git_submodule_add),
        )
        .route(
            "/api/git/submodules/init",
            post(crate::git::git_submodule_init),
        )
        .route(
            "/api/git/submodules/update",
            post(crate::git::git_submodule_update),
        )
        .route("/api/git/log", get(crate::git::git_log))
        .route("/api/git/commit-diff", get(crate::git::git_commit_diff))
        .route("/api/git/commit-files", get(crate::git::git_commit_files))
        .route("/api/git/stage", post(crate::git::git_stage))
        .route("/api/git/clean", post(crate::git::git_clean))
        .route("/api/git/ignore", post(crate::git::git_ignore))
        .route("/api/git/rename", post(crate::git::git_rename))
        .route("/api/git/delete", post(crate::git::git_delete))
        .route("/api/git/unstage", post(crate::git::git_unstage))
        .route("/api/git/revert", post(crate::git::git_revert))
        .route("/api/git/pull", post(crate::git::git_pull))
        .route("/api/git/push", post(crate::git::git_push))
        .route(
            "/api/git/create-github-repo-and-push",
            post(crate::git::git_create_github_repo_and_push),
        )
        .route("/api/git/fetch", post(crate::git::git_fetch))
        .route("/api/git/commit", post(crate::git::git_commit))
        .route("/api/git/undo-commit", post(crate::git::git_undo_commit))
        .route("/api/git/reset", post(crate::git::git_reset_commit))
        .route(
            "/api/git/commit-template",
            get(crate::git::git_commit_template),
        )
        .route("/api/git/conflicts", get(crate::git::git_conflicts_list))
        .route(
            "/api/git/branches",
            get(crate::git::git_branches)
                .post(crate::git::git_create_branch)
                .delete(crate::git::git_delete_branch),
        )
        .route(
            "/api/git/branches/rename",
            post(crate::git::git_rename_branch),
        )
        .route(
            "/api/git/branches/delete-remote",
            post(crate::git::git_delete_remote_branch),
        )
        .route("/api/git/tags", get(crate::git::git_tags_list))
        .route("/api/git/tags", post(crate::git::git_tags_create))
        .route(
            "/api/git/tags",
            axum::routing::delete(crate::git::git_tags_delete),
        )
        .route(
            "/api/git/tags/delete-remote",
            post(crate::git::git_tags_delete_remote),
        )
        .route("/api/git/checkout", post(crate::git::git_checkout))
        .route(
            "/api/git/checkout-detached",
            post(crate::git::git_checkout_detached),
        )
        .route(
            "/api/git/branches/create-from",
            post(crate::git::git_create_branch_from),
        )
        .route(
            "/api/git/worktrees",
            get(crate::git::git_worktrees)
                .post(crate::git::git_worktree_add)
                .delete(crate::git::git_worktree_remove),
        )
        .route(
            "/api/git/worktrees/prune",
            post(crate::git::git_worktree_prune),
        )
        .route(
            "/api/git/worktrees/migrate",
            post(crate::git::git_worktree_migrate),
        )
        .route(
            "/api/ui/terminal/state",
            get(crate::terminal_ui_state::terminal_ui_state_get)
                .put(crate::terminal_ui_state::terminal_ui_state_put),
        )
        .route(
            "/api/ui/terminal/state/events",
            get(crate::terminal_ui_state::terminal_ui_state_events),
        )
        .route(
            "/api/terminal/create",
            post(crate::terminal::terminal_create),
        )
        .route(
            "/api/terminal/{session_id}",
            get(crate::terminal::terminal_get).delete(crate::terminal::terminal_delete),
        )
        .route(
            "/api/terminal/{session_id}/stream",
            get(crate::terminal::terminal_stream),
        )
        .route(
            "/api/terminal/{session_id}/input",
            post(crate::terminal::terminal_input),
        )
        .route(
            "/api/terminal/{session_id}/resize",
            post(crate::terminal::terminal_resize),
        )
        .route(
            "/api/terminal/{session_id}/start",
            post(crate::terminal::terminal_start),
        )
        .route(
            "/api/terminal/{session_id}/stop",
            post(crate::terminal::terminal_stop),
        )
        .route(
            "/api/terminal/{session_id}/restart",
            post(crate::terminal::terminal_restart),
        )
        .route(
            "/api/{*path}",
            axum::routing::any(crate::opencode_proxy::proxy_opencode_rest),
        )
        .with_state(shared_state.clone())
        .layer(middleware::from_fn_with_state(
            shared_state.clone(),
            crate::ui_auth::require_ui_auth,
        ));

    let ui_dir_path = args.ui_dir.as_ref().map(PathBuf::from);
    let (has_ui, asset_files, static_files) = match &ui_dir_path {
        None => {
            tracing::info!("UI disabled (API-only mode)");
            (false, None, None)
        }
        Some(dir) => {
            let index_file = dir.join("index.html");
            let has_ui = index_file.is_file();
            tracing::info!(
                "UI dir resolved to {} (index.html exists: {})",
                dir.display(),
                has_ui
            );

            let asset_files = ServeDir::new(dir.join("assets"));
            let static_files = ServeDir::new(dir).fallback(ServeFile::new(index_file));
            (has_ui, Some(asset_files), Some(static_files))
        }
    };

    let mut app = public_router
        .merge(agena_api)
        .merge(studio_api_routes)
        .layer(TraceLayer::new_for_http());

    if let Some(cors) = build_cors_layer(&normalized_cors_origins, args.cors_allow_all) {
        if args.cors_allow_all {
            tracing::info!("CORS enabled (allow all)");
        } else {
            tracing::info!(origins = %normalized_cors_origins.len(), "CORS enabled");
        }
        app = app.layer(cors);
    }

    app = if has_ui {
        app.nest_service("/assets", asset_files.expect("assets service"))
            .fallback_service(static_files.expect("static service"))
    } else {
        app.fallback(|| async {
            Json(serde_json::json!({
                "service": "agena-studio",
                "ui": false,
                "message": "Agena Studio server is running in API-only mode. Pass --ui-dir <dist> to serve the bundled UI.",
            }))
        })
    };

    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .map_err(|error| anyhow!("invalid bind address {}:{}: {error}", args.host, args.port))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind listener on {addr}"))?;

    tracing::info!("Agena Studio listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            runtime.shutdown();
        })
        .await
        .context("server exited unexpectedly")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ENV_LOCK;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use serde_json::json;
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn assert_git_available() {
        assert!(
            Command::new("git").arg("--version").output().is_ok(),
            "git is required for this test"
        );
    }

    fn git_ok(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git -C {} {} should succeed\nstdout:\n{}\nstderr:\n{}",
            repo.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(repo: &Path, args: &[&str]) -> Option<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn test_api_path(path: &Path) -> String {
        path.display().to_string().replace('\\', "/")
    }

    fn init_git_repo(repo: &Path) {
        assert_git_available();
        let status = Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(repo)
            .status()
            .expect("git init should run");
        assert!(status.success(), "git init should succeed");
        git_ok(repo, &["config", "user.name", "Agena Test"]);
        git_ok(repo, &["config", "user.email", "test@example.com"]);
    }

    struct EnvVarGuard {
        key: String,
        old: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &str, value: String) -> Self {
            let old = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key: key.to_string(),
                old,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(ref old) = self.old {
                    std::env::set_var(&self.key, old);
                } else {
                    std::env::remove_var(&self.key);
                }
            }
        }
    }

    async fn compat_test_app_state_with_openai_base_url(
        openai_base_url: &str,
    ) -> (
        Arc<AppState>,
        Arc<sea_orm::DatabaseConnection>,
        tempfile::NamedTempFile,
        tempfile::TempDir,
    ) {
        let config = tempfile::NamedTempFile::new().expect("config file should be created");
        let workspace = tempdir().expect("workspace should be created");
        std::fs::write(
            config.path(),
            format!(
                r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "{openai_base_url}"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true

[plugins.list."agena.memory"]
kind = "static"

[plugins.list."agena.memory".options.project_instructions]
enabled = true
include_global = true
"#,
            ),
        )
        .expect("config file should be written");

        let db = Arc::new(
            sea_orm::Database::connect("sqlite::memory:")
                .await
                .expect("database should connect"),
        );
        let runtime = AgenaRuntime::builder()
            .with_load_request(agena::config::LoadConfigRequest {
                config_path: Some(config.path().to_path_buf()),
                ..agena::config::LoadConfigRequest::default()
            })
            .with_workspace_root(workspace.path())
            .with_database_connection(db.as_ref().clone())
            .build()
            .await
            .expect("runtime should build");

        let studio_db = Arc::new(
            crate::studio_db::StudioDb::open_at_path(
                workspace.path().join(".agena-studio-test.db"),
            )
            .await
            .expect("studio db should open"),
        );
        let workspace_preview_registry = Arc::new(
            crate::workspace_preview_registry::WorkspacePreviewRegistry::new(studio_db.clone()),
        );
        let workspace_preview_runtime = Arc::new(
            crate::workspace_preview_runtime::WorkspacePreviewRuntime::new(
                workspace_preview_registry.clone(),
            ),
        );
        let terminal = Arc::new(crate::terminal::TerminalManager::new(studio_db.clone()).await);

        (
            Arc::new(AppState {
                ui_auth: crate::ui_auth::init_ui_auth(None),
                ui_cookie_same_site: SameSite::Lax,
                cors_allowed_origins: Vec::new(),
                cors_allow_all: false,
                opencode: Arc::new(crate::opencode::OpenCodeManager::new(
                    "127.0.0.1".to_string(),
                    Some(1),
                    true,
                    None,
                    None,
                    crate::ui_auth::UiAuth::Disabled,
                )),
                plugin_runtime: Arc::new(crate::plugin_runtime::PluginRuntime::new()),
                terminal,
                attachment_cache: Arc::new(crate::attachment_cache::AttachmentCacheManager::new(
                    studio_db.clone(),
                )),
                session_activity: crate::session_activity::SessionActivityManager::new(),
                directory_session_index:
                    crate::directory_session_index::DirectorySessionIndexManager::new(),
                workspace_preview_registry,
                workspace_preview_runtime,
                studio_db,
                settings: Arc::new(RwLock::new(crate::settings::Settings::default())),
                runtime,
            }),
            db,
            config,
            workspace,
        )
    }

    async fn compat_test_app_state() -> (
        Arc<AppState>,
        Arc<sea_orm::DatabaseConnection>,
        tempfile::NamedTempFile,
        tempfile::TempDir,
    ) {
        compat_test_app_state_with_openai_base_url("http://127.0.0.1:9/v1").await
    }

    async fn test_app_state_with_opencode_port(
        opencode_port: u16,
        settings: crate::settings::Settings,
    ) -> (
        Arc<AppState>,
        Arc<sea_orm::DatabaseConnection>,
        tempfile::NamedTempFile,
        tempfile::TempDir,
    ) {
        let config = tempfile::NamedTempFile::new().expect("config file should be created");
        let workspace = tempdir().expect("workspace should be created");
        std::fs::write(
            config.path(),
            r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "http://127.0.0.1:9/v1"
api_key = "test"

[providers.openai.adapters.openai]
enabled = true
"#,
        )
        .expect("config file should be written");

        let db = Arc::new(
            sea_orm::Database::connect("sqlite::memory:")
                .await
                .expect("database should connect"),
        );
        let runtime = AgenaRuntime::builder()
            .with_load_request(agena::config::LoadConfigRequest {
                config_path: Some(config.path().to_path_buf()),
                ..agena::config::LoadConfigRequest::default()
            })
            .with_workspace_root(workspace.path())
            .with_database_connection(db.as_ref().clone())
            .build()
            .await
            .expect("runtime should build");

        let studio_db = Arc::new(
            crate::studio_db::StudioDb::open_at_path(
                workspace.path().join(".agena-studio-test.db"),
            )
            .await
            .expect("studio db should open"),
        );
        let workspace_preview_registry = Arc::new(
            crate::workspace_preview_registry::WorkspacePreviewRegistry::new(studio_db.clone()),
        );
        let workspace_preview_runtime = Arc::new(
            crate::workspace_preview_runtime::WorkspacePreviewRuntime::new(
                workspace_preview_registry.clone(),
            ),
        );
        let terminal = Arc::new(crate::terminal::TerminalManager::new(studio_db.clone()).await);

        (
            Arc::new(AppState {
                ui_auth: crate::ui_auth::init_ui_auth(None),
                ui_cookie_same_site: SameSite::Lax,
                cors_allowed_origins: Vec::new(),
                cors_allow_all: false,
                opencode: Arc::new(crate::opencode::OpenCodeManager::new(
                    "127.0.0.1".to_string(),
                    Some(opencode_port),
                    true,
                    None,
                    None,
                    crate::ui_auth::UiAuth::Disabled,
                )),
                plugin_runtime: Arc::new(crate::plugin_runtime::PluginRuntime::new()),
                terminal,
                attachment_cache: Arc::new(crate::attachment_cache::AttachmentCacheManager::new(
                    studio_db.clone(),
                )),
                session_activity: crate::session_activity::SessionActivityManager::new(),
                directory_session_index:
                    crate::directory_session_index::DirectorySessionIndexManager::new(),
                workspace_preview_registry,
                workspace_preview_runtime,
                studio_db,
                settings: Arc::new(RwLock::new(settings)),
                runtime,
            }),
            db,
            config,
            workspace,
        )
    }

    async fn fs_test_router() -> Router {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        fs_router().with_state(state)
    }

    async fn spawn_mock_opencode_server(extra: Router) -> (u16, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route("/config", get(|| async { Json(json!({ "ok": true })) }))
            .route("/agent", get(|| async { Json(json!({ "agents": [] })) }))
            .merge(extra);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener.local_addr().expect("listener should have address");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock opencode server should serve");
        });

        (address.port(), handle)
    }

    fn unique_tmp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("agena-studio-{label}-{nanos}"))
    }

    async fn write_json(path: &Path, value: &Value) {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .expect("json parent directory should exist");
        }
        tokio::fs::write(
            path,
            serde_json::to_vec_pretty(value).expect("json should encode"),
        )
        .await
        .expect("json file should write");
    }

    fn opencode_storage_root(home: &Path) -> PathBuf {
        home.join(".local")
            .join("share")
            .join("opencode")
            .join("storage")
    }

    async fn seed_opencode_session_summary(
        home: &Path,
        project_id: &str,
        session_id: &str,
        directory: &Path,
        title: &str,
        updated: f64,
    ) {
        write_json(
            &opencode_storage_root(home)
                .join("sessions")
                .join(project_id)
                .join(format!("{session_id}.json")),
            &json!({
                "id": session_id,
                "directory": directory.display().to_string(),
                "title": title,
                "slug": title.to_lowercase().replace(' ', "-"),
                "time": {
                    "created": updated,
                    "updated": updated
                }
            }),
        )
        .await;
    }

    async fn seed_opencode_message_with_parts(
        home: &Path,
        session_id: &str,
        message_id: &str,
        role: &str,
        created: f64,
        parts: Vec<Value>,
    ) {
        write_json(
            &opencode_storage_root(home)
                .join("messages")
                .join(session_id)
                .join(format!("{message_id}.json")),
            &json!({
                "id": message_id,
                "sessionId": session_id,
                "sessionID": session_id,
                "role": role,
                "time": {
                    "created": created,
                    "updated": created
                }
            }),
        )
        .await;

        for (idx, mut part) in parts.into_iter().enumerate() {
            let part_id = part
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("part_{idx}"));
            if let Some(obj) = part.as_object_mut() {
                obj.entry("id".to_string())
                    .or_insert_with(|| Value::String(part_id.clone()));
                obj.entry("sessionId".to_string())
                    .or_insert_with(|| Value::String(session_id.to_string()));
                obj.entry("sessionID".to_string())
                    .or_insert_with(|| Value::String(session_id.to_string()));
                obj.entry("messageId".to_string())
                    .or_insert_with(|| Value::String(message_id.to_string()));
                obj.entry("messageID".to_string())
                    .or_insert_with(|| Value::String(message_id.to_string()));
            }

            write_json(
                &opencode_storage_root(home)
                    .join("message-parts")
                    .join(message_id)
                    .join(format!("{part_id}.json")),
                &part,
            )
            .await;
        }
    }

    #[test]
    fn normalize_origin_str_accepts_http_and_https_origins() {
        assert_eq!(
            normalize_origin_str(" https://studio.example/path?q=1 ").as_deref(),
            Some("https://studio.example")
        );
        assert_eq!(
            normalize_origin_str("http://localhost:5173/").as_deref(),
            Some("http://localhost:5173")
        );
    }

    #[test]
    fn normalize_origin_str_rejects_invalid_and_non_http_schemes() {
        assert_eq!(normalize_origin_str(""), None);
        assert_eq!(normalize_origin_str("notaurl"), None);
        assert_eq!(normalize_origin_str("file:///tmp/demo"), None);
    }

    #[test]
    fn build_cors_layer_depends_on_allow_all_and_origin_list() {
        assert!(build_cors_layer(&[], false).is_none());
        assert!(build_cors_layer(&["https://studio.example".to_string()], false).is_some());
        assert!(build_cors_layer(&[], true).is_some());
    }

    #[test]
    fn resolve_same_site_auto_switches_for_cross_origin_usage() {
        assert!(matches!(
            resolve_same_site(crate::UiCookieSameSite::Auto, false),
            SameSite::Strict
        ));
        assert!(matches!(
            resolve_same_site(crate::UiCookieSameSite::Auto, true),
            SameSite::None
        ));
        assert!(matches!(
            resolve_same_site(crate::UiCookieSameSite::Lax, true),
            SameSite::Lax
        ));
    }

    #[tokio::test]
    async fn compat_provider_env_check_reports_present_and_missing_vars() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let router = Router::new()
            .route(
                "/api/provider/env/check",
                post(crate::providers::env_check_post),
            )
            .with_state(state.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/provider/env/check")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "vars": [
                                "PATH",
                                "AGENA_STUDIO_COMPAT_MISSING_ENV",
                                "bad-name",
                                "path"
                            ]
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert!(
            payload["present"]
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value == "PATH"))
        );
        assert!(payload["missing"].as_array().is_some_and(|values| {
            values
                .iter()
                .any(|value| value == "AGENA_STUDIO_COMPAT_MISSING_ENV")
        }));
        assert!(
            !payload["present"]
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value == "bad-name"))
        );
        assert!(
            !payload["missing"]
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value == "path"))
        );

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_config_reload_returns_success_shape() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let router = Router::new()
            .route(
                "/api/config/reload",
                post(crate::config::config_reload_post),
            )
            .with_state(state.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/config/reload")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload["success"], json!(true));
        assert_eq!(payload["requiresReload"], json!(true));
        assert!(payload["message"].is_string());
        assert!(payload["reloadDelayMs"].as_i64().is_some());

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn config_providers_route_proxies_to_opencode() {
        let extra = Router::new().route(
            "/config/providers",
            get(
                |Query(query): Query<std::collections::HashMap<String, String>>| async move {
                    Json(json!({
                        "providers": [{
                            "id": "openai",
                            "models": [{
                                "id": "gpt-4.1-mini",
                                "providerId": "openai"
                            }]
                        }],
                        "default": {
                            "openai": "gpt-4.1-mini"
                        },
                        "directory": query.get("directory").cloned().unwrap_or_default()
                    }))
                },
            ),
        );
        let (port, handle) = spawn_mock_opencode_server(extra).await;
        let (state, _db, _config, workspace) =
            test_app_state_with_opencode_port(port, crate::settings::Settings::default()).await;
        state
            .opencode
            .ensure_ready(Duration::from_secs(2))
            .await
            .expect("mock opencode should be ready");

        let directory = workspace.path().display().to_string();
        let router = Router::new()
            .route(
                "/api/{*path}",
                axum::routing::any(crate::opencode_proxy::proxy_opencode_rest),
            )
            .with_state(state.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/config/providers?directory={}",
                        urlencoding::encode(&directory)
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload["default"]["openai"], json!("gpt-4.1-mini"));
        assert_eq!(payload["directory"], json!(directory));
        assert_eq!(payload["providers"][0]["id"], json!("openai"));
        assert_eq!(
            payload["providers"][0]["models"][0],
            json!({
                "id": "gpt-4.1-mini",
                "providerId": "openai"
            })
        );

        state.runtime.shutdown();
        handle.abort();
    }

    #[tokio::test]
    async fn config_settings_route_round_trips_settings_payload() {
        let (state, _db, _config, workspace) = compat_test_app_state().await;
        let router = Router::new()
            .route(
                "/api/config/settings",
                get(crate::config::config_settings_get).put(crate::config::config_settings_put),
            )
            .with_state(state.clone());

        let update_body = json!({
            "directories": [
                {
                    "id": "dir_1",
                    "path": workspace.path().display().to_string()
                }
            ],
            "showReasoningTraces": true
        });

        let put_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/config/settings")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(update_body.to_string()))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(put_response.status(), StatusCode::OK);
        let put_payload: Value = serde_json::from_slice(
            &put_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(put_payload["showReasoningTraces"], json!(true));
        assert_eq!(put_payload["directories"], put_payload["projects"]);
        assert_eq!(put_payload["directories"][0]["id"], json!("dir_1"));
        assert_eq!(
            put_payload["directories"][0]["path"],
            json!(workspace.path().display().to_string())
        );

        let get_response = router
            .oneshot(
                Request::builder()
                    .uri("/api/config/settings")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(get_response.status(), StatusCode::OK);
        let get_payload: Value = serde_json::from_slice(
            &get_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(get_payload["showReasoningTraces"], json!(true));
        assert_eq!(get_payload["directories"], get_payload["projects"]);
        assert_eq!(get_payload["directories"][0]["id"], json!("dir_1"));

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_provider_source_returns_expected_shape_for_runtime_config() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let (state, _db, _config, workspace) = compat_test_app_state().await;
        let _home = EnvVarGuard::set("HOME", workspace.path().display().to_string());
        let custom_config = tempfile::NamedTempFile::new().expect("custom config should exist");
        std::fs::write(
            custom_config.path(),
            r#"{ provider: { openai: { apiKey: "custom-test" } } }"#,
        )
        .expect("custom config should be written");
        let _custom_config = EnvVarGuard::set(
            crate::opencode_config::OPENCODE_CONFIG_ENV,
            custom_config.path().display().to_string(),
        );
        let router = Router::new()
            .route(
                "/api/provider/{provider_id}/source",
                get(crate::providers::provider_source_get),
            )
            .with_state(state.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/provider/openai/source")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload["providerId"], json!("openai"));
        assert_eq!(payload["sources"]["auth"]["exists"], json!(false));
        assert_eq!(payload["sources"]["user"]["exists"], json!(false));
        assert_eq!(payload["sources"]["project"]["exists"], json!(false));
        assert_eq!(payload["sources"]["custom"]["exists"], json!(true));
        assert_eq!(
            payload["sources"]["custom"]["path"],
            json!(custom_config.path().display().to_string())
        );
        assert!(payload["sources"]["user"]["path"].is_string());

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_provider_source_uses_project_path_for_directory_scoped_lookup() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let (state, _db, _config, workspace) = compat_test_app_state().await;
        let _home = EnvVarGuard::set("HOME", workspace.path().display().to_string());
        let project_config = workspace.path().join("opencode.json");
        std::fs::write(
            &project_config,
            r#"{ provider: { openai: { apiKey: "project-test" } } }"#,
        )
        .expect("project config should be written");
        let router = Router::new()
            .route(
                "/api/provider/{provider_id}/source",
                get(crate::providers::provider_source_get),
            )
            .with_state(state.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/provider/openai/source?directory={}",
                        workspace.path().display()
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload["providerId"], json!("openai"));
        assert_eq!(payload["sources"]["project"]["exists"], json!(true));
        assert_eq!(
            payload["sources"]["project"]["path"],
            json!(project_config.display().to_string())
        );
        assert_eq!(payload["sources"]["custom"]["exists"], json!(false));

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_provider_source_returns_empty_sources_for_unknown_provider() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let router = Router::new()
            .route(
                "/api/provider/{provider_id}/source",
                get(crate::providers::provider_source_get),
            )
            .with_state(state.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/provider/missing/source")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload["providerId"], json!("missing"));
        assert_eq!(payload["sources"]["auth"]["exists"], json!(false));
        assert_eq!(payload["sources"]["user"]["exists"], json!(false));
        assert_eq!(payload["sources"]["project"]["exists"], json!(false));
        assert_eq!(payload["sources"]["custom"]["exists"], json!(false));

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn directories_route_pages_configured_entries() {
        let (state, _db, _config, workspace) = compat_test_app_state().await;
        let alpha = workspace.path().join("alpha");
        let beta = workspace.path().join("beta");
        let gamma = workspace.path().join("gamma");
        std::fs::create_dir_all(&alpha).expect("alpha should exist");
        std::fs::create_dir_all(&beta).expect("beta should exist");
        std::fs::create_dir_all(&gamma).expect("gamma should exist");
        *state.settings.write().await = crate::settings::Settings {
            projects: vec![
                crate::settings::Project {
                    id: "gamma".to_string(),
                    path: gamma.display().to_string(),
                    added_at: 0,
                    last_opened_at: 0,
                },
                crate::settings::Project {
                    id: "beta".to_string(),
                    path: beta.display().to_string(),
                    added_at: 0,
                    last_opened_at: 0,
                },
                crate::settings::Project {
                    id: "alpha".to_string(),
                    path: alpha.display().to_string(),
                    added_at: 0,
                    last_opened_at: 0,
                },
            ],
            ..Default::default()
        };

        let router = Router::new()
            .route(
                "/api/directories",
                get(crate::chat_sidebar::directories_get),
            )
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/directories?offset=1&limit=2")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload.get("total").and_then(Value::as_u64), Some(3));
        assert_eq!(payload.get("offset").and_then(Value::as_u64), Some(1));
        assert_eq!(payload.get("limit").and_then(Value::as_u64), Some(2));
        assert_eq!(payload.get("hasMore").and_then(Value::as_bool), Some(false));
        assert!(payload.get("nextOffset").is_some_and(Value::is_null));
        assert_eq!(
            payload
                .get("items")
                .and_then(Value::as_array)
                .expect("items should be present")
                .iter()
                .filter_map(|entry| entry.get("path").and_then(Value::as_str))
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>(),
            vec![beta.display().to_string(), alpha.display().to_string()]
        );

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn session_activity_route_returns_runtime_snapshot() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        state
            .session_activity
            .set_phase("ses_busy_1", crate::session_activity::SessionPhase::Busy);
        let router = Router::new()
            .route("/api/session-activity", get(session_activity_get))
            .with_state(state.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/session-activity")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload["ses_busy_1"]["type"], json!("busy"));

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_fs_home_route_returns_non_empty_home_path() {
        let response = fs_test_router()
            .await
            .oneshot(
                Request::builder()
                    .uri("/api/fs/home")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: Value = serde_json::from_slice(&body).expect("response should be valid json");
        let home = payload
            .get("home")
            .and_then(Value::as_str)
            .expect("response should include home");
        assert!(!home.is_empty());
    }

    #[tokio::test]
    async fn compat_fs_list_route_lists_directory_with_pagination() {
        let temp = tempdir().expect("tempdir should be created");
        std::fs::write(temp.path().join("alpha.txt"), "alpha").expect("alpha should be written");
        std::fs::write(temp.path().join("beta.txt"), "beta").expect("beta should be written");

        let uri = format!(
            "/api/fs/list?path={}&offset=1&limit=1",
            urlencoding::encode(&temp.path().display().to_string())
        );
        let response = fs_test_router()
            .await
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: crate::fs::ListResponse =
            serde_json::from_slice(&body).expect("response should be valid json");
        assert_eq!(payload.path, test_api_path(temp.path()));
        assert_eq!(payload.total, 2);
        assert_eq!(payload.offset, Some(1));
        assert_eq!(payload.limit, Some(1));
        assert!(!payload.has_more);
        assert_eq!(payload.next_offset, None);
        assert_eq!(payload.entries.len(), 1);
        assert_eq!(payload.entries[0].name, "beta.txt");
        assert!(payload.entries[0].is_file);
        assert!(!payload.entries[0].is_directory);
    }

    #[tokio::test]
    async fn compat_fs_raw_and_download_routes_serve_scoped_files() {
        let temp = tempdir().expect("tempdir should be created");
        let file = temp.path().join("notes.txt");
        std::fs::write(&file, "hello studio").expect("file should be written");

        let directory_path = temp.path().display().to_string();
        let directory = urlencoding::encode(&directory_path);
        let file_path_value = file.display().to_string();
        let file_path = urlencoding::encode(&file_path_value);

        let raw_uri = format!("/api/fs/raw?path={file_path}");
        let raw_response = fs_test_router()
            .await
            .clone()
            .oneshot(
                Request::builder()
                    .uri(raw_uri)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(raw_response.status(), StatusCode::OK);
        let raw_disposition = raw_response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .expect("content disposition should exist")
            .to_string();
        assert!(raw_disposition.starts_with("inline;"));
        let raw_body = raw_response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        assert_eq!(raw_body.as_ref(), b"hello studio");

        let download_uri = format!("/api/fs/download?directory={directory}&path=notes.txt");
        let download_response = fs_test_router()
            .await
            .clone()
            .oneshot(
                Request::builder()
                    .uri(download_uri)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(download_response.status(), StatusCode::OK);
        let download_disposition = download_response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .expect("content disposition should exist")
            .to_string();
        assert!(download_disposition.starts_with("attachment;"));

        let traversal_uri = "/api/fs/raw?path=../notes.txt";
        let traversal_response = fs_test_router()
            .await
            .oneshot(
                Request::builder()
                    .uri(traversal_uri)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(traversal_response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn compat_fs_read_route_returns_plain_text_for_scoped_file() {
        let temp = tempdir().expect("tempdir should be created");
        let file = temp.path().join("notes.txt");
        std::fs::write(&file, "hello studio").expect("file should be written");

        let read_uri = format!(
            "/api/fs/read?path={}",
            urlencoding::encode(&file.display().to_string())
        );

        let response = fs_test_router()
            .await
            .oneshot(
                Request::builder()
                    .uri(read_uri)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/plain")
        );
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        assert_eq!(body.as_ref(), b"hello studio");
    }

    #[tokio::test]
    async fn compat_fs_read_chunk_route_returns_metadata_and_chunk_content() {
        let temp = tempdir().expect("tempdir should be created");
        let file = temp.path().join("notes.txt");
        std::fs::write(&file, "hello studio").expect("file should be written");

        let read_chunk_uri = format!(
            "/api/fs/read-chunk?path={}&offset=0&limit=5",
            urlencoding::encode(&file.display().to_string())
        );

        let response = fs_test_router()
            .await
            .oneshot(
                Request::builder()
                    .uri(read_chunk_uri)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: crate::fs::ReadChunkResponse =
            serde_json::from_slice(&body).expect("response should be valid json");
        assert_eq!(payload.path, test_api_path(&file));
        assert_eq!(payload.content, "hello");
        assert_eq!(payload.offset, 0);
        assert_eq!(payload.limit, 5);
        assert_eq!(payload.loaded_bytes, 5);
        assert_eq!(payload.total_bytes, 12);
        assert!(payload.has_more);
        assert_eq!(payload.next_offset, Some(5));
    }

    #[tokio::test]
    async fn compat_fs_write_route_creates_scoped_file() {
        let temp = tempdir().expect("tempdir should be created");
        let directory_path = temp.path().display().to_string();
        let directory = urlencoding::encode(&directory_path);

        let response = fs_test_router()
            .await
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/fs/write?directory={directory}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"path":"nested/notes.txt","content":"hello studio"}).to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: crate::fs::SuccessPathResponse =
            serde_json::from_slice(&body).expect("response should be valid json");
        assert!(payload.success);
        assert_eq!(
            std::fs::read_to_string(temp.path().join("nested/notes.txt"))
                .expect("file should exist after write"),
            "hello studio"
        );
    }

    #[tokio::test]
    async fn compat_fs_mkdir_route_creates_scoped_directory() {
        let temp = tempdir().expect("tempdir should be created");
        let directory_path = temp.path().display().to_string();
        let directory = urlencoding::encode(&directory_path);

        let response = fs_test_router()
            .await
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/fs/mkdir?directory={directory}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({"path":"nested/deeper"}).to_string()))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: crate::fs::SuccessPathResponse =
            serde_json::from_slice(&body).expect("response should be valid json");
        assert!(payload.success);
        assert!(temp.path().join("nested/deeper").is_dir());
    }

    #[tokio::test]
    async fn compat_fs_rename_route_renames_scoped_path() {
        let temp = tempdir().expect("tempdir should be created");
        std::fs::create_dir_all(temp.path().join("nested")).expect("nested dir should exist");
        std::fs::write(temp.path().join("nested/notes.txt"), "hello studio")
            .expect("file should be written");
        let directory_path = temp.path().display().to_string();
        let directory = urlencoding::encode(&directory_path);

        let response = fs_test_router()
            .await
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/fs/rename?directory={directory}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"oldPath":"nested/notes.txt","newPath":"nested/archive.txt"})
                            .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: crate::fs::SuccessPathResponse =
            serde_json::from_slice(&body).expect("response should be valid json");
        assert!(payload.success);
        assert!(!temp.path().join("nested/notes.txt").exists());
        assert_eq!(
            std::fs::read_to_string(temp.path().join("nested/archive.txt"))
                .expect("renamed file should exist"),
            "hello studio"
        );
    }

    #[tokio::test]
    async fn compat_fs_delete_route_deletes_scoped_path_and_is_idempotent() {
        let temp = tempdir().expect("tempdir should be created");
        std::fs::create_dir_all(temp.path().join("nested/deeper"))
            .expect("nested dir should exist");
        std::fs::write(temp.path().join("nested/deeper/notes.txt"), "hello studio")
            .expect("file should be written");
        let directory_path = temp.path().display().to_string();
        let directory = urlencoding::encode(&directory_path);

        let request = || {
            Request::builder()
                .method("POST")
                .uri(format!("/api/fs/delete?directory={directory}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"path":"nested"}).to_string()))
                .expect("request should build")
        };

        let first_response = fs_test_router()
            .await
            .clone()
            .oneshot(request())
            .await
            .expect("request should succeed");
        assert_eq!(first_response.status(), StatusCode::OK);
        assert!(!temp.path().join("nested").exists());

        let second_response = fs_test_router()
            .await
            .oneshot(request())
            .await
            .expect("request should succeed");
        assert_eq!(second_response.status(), StatusCode::OK);
        let body = second_response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: crate::fs::SuccessPathResponse =
            serde_json::from_slice(&body).expect("response should be valid json");
        assert!(payload.success);
    }

    #[tokio::test]
    async fn compat_fs_search_route_returns_ranked_files() {
        let temp = tempdir().expect("tempdir should be created");
        std::fs::create_dir_all(temp.path().join("src")).expect("src dir should exist");
        std::fs::create_dir_all(temp.path().join("node_modules"))
            .expect("excluded dir should exist");
        std::fs::write(temp.path().join("src/app.ts"), "export {}")
            .expect("app.ts should be written");
        std::fs::write(temp.path().join("src/app.test.ts"), "export {}")
            .expect("app.test.ts should be written");
        std::fs::write(temp.path().join("node_modules/app.ts"), "ignored")
            .expect("ignored file should be written");

        let root_path = temp.path().display().to_string();
        let root = urlencoding::encode(&root_path);
        let uri = format!("/api/fs/search?root={root}&q=app&limit=5&respectGitignore=false");

        let response = fs_test_router()
            .await
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: Value = serde_json::from_slice(&body).expect("response should be valid json");
        assert_eq!(payload["root"], json!(test_api_path(temp.path())));
        assert_eq!(payload["count"], json!(2));
        let files = payload["files"]
            .as_array()
            .expect("response should include files");
        assert_eq!(files[0]["relativePath"], json!("src/app.ts"));
        assert_eq!(files[1]["relativePath"], json!("src/app.test.ts"));
        assert!(files.iter().all(|file| {
            file["path"]
                .as_str()
                .is_some_and(|path| !path.contains("node_modules"))
        }));
    }

    #[tokio::test]
    async fn compat_fs_content_search_and_replace_routes_work() {
        let temp = tempdir().expect("tempdir should be created");
        std::fs::create_dir_all(temp.path().join("src")).expect("src dir should exist");
        let file = temp.path().join("src/app.txt");
        std::fs::write(&file, "hello world\nhello studio\n").expect("file should be written");
        let directory_value = temp.path().display().to_string();
        let directory = urlencoding::encode(&directory_value);

        let search_response = fs_test_router()
            .await
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/fs/search-content?directory={directory}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "query":"hello",
                            "includeHidden": false,
                            "respectGitignore": false,
                            "isRegex": false,
                            "caseSensitive": false,
                            "wholeWord": false
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(search_response.status(), StatusCode::OK);
        let search_payload: crate::fs::ContentSearchResponse = serde_json::from_slice(
            &search_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(search_payload.file_count, 1);
        assert_eq!(search_payload.match_count, 2);
        assert_eq!(search_payload.files[0].relative_path, "src/app.txt");

        let replace_response = fs_test_router()
            .await
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/fs/replace-content?directory={directory}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "query":"hello",
                            "replace":"hi",
                            "includeHidden": false,
                            "respectGitignore": false,
                            "isRegex": false,
                            "caseSensitive": false,
                            "wholeWord": false
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(replace_response.status(), StatusCode::OK);
        let replace_payload: crate::fs::ContentReplaceResponse = serde_json::from_slice(
            &replace_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(replace_payload.file_count, 1);
        assert_eq!(replace_payload.replacement_count, 2);
        assert_eq!(
            std::fs::read_to_string(&file).expect("file should remain readable"),
            "hi world\nhi studio\n"
        );
    }

    #[tokio::test]
    async fn compat_git_status_route_returns_paginated_scoped_files() {
        let temp = tempdir().expect("tempdir should be created");
        let repo = temp.path();
        init_git_repo(repo);

        std::fs::write(repo.join("mixed.txt"), "base\n").expect("mixed file should be written");
        std::fs::write(repo.join("modified.txt"), "base\n")
            .expect("modified file should be written");
        git_ok(repo, &["add", "mixed.txt", "modified.txt"]);
        git_ok(repo, &["commit", "-m", "base"]);

        std::fs::write(repo.join("mixed.txt"), "staged change\n")
            .expect("mixed file should be updated");
        git_ok(repo, &["add", "mixed.txt"]);
        std::fs::write(repo.join("mixed.txt"), "staged change\nunstaged change\n")
            .expect("mixed file should be updated again");

        std::fs::write(repo.join("modified.txt"), "base\nlocal change\n")
            .expect("modified file should be updated");

        std::fs::write(repo.join("staged.txt"), "only staged\n")
            .expect("staged file should be written");
        git_ok(repo, &["add", "staged.txt"]);

        std::fs::write(repo.join("untracked.txt"), "new file\n")
            .expect("untracked file should be written");

        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let router = Router::new()
            .route("/api/git/status", get(crate::git::git_status))
            .with_state(state.clone());
        let directory_value = repo.display().to_string();
        let directory = urlencoding::encode(&directory_value);
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/git/status?directory={directory}&scope=staged&offset=0&limit=1"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload.get("scope").and_then(Value::as_str), Some("staged"));
        assert_eq!(payload.get("totalFiles").and_then(Value::as_u64), Some(4));
        assert_eq!(payload.get("stagedCount").and_then(Value::as_u64), Some(2));
        assert_eq!(
            payload.get("unstagedCount").and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            payload.get("untrackedCount").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(payload.get("mergeCount").and_then(Value::as_u64), Some(0));
        assert_eq!(payload.get("offset").and_then(Value::as_u64), Some(0));
        assert_eq!(payload.get("limit").and_then(Value::as_u64), Some(1));
        assert_eq!(payload.get("hasMore").and_then(Value::as_bool), Some(true));
        let files = payload
            .get("files")
            .and_then(Value::as_array)
            .expect("files should be present");
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].get("path").and_then(Value::as_str),
            Some("mixed.txt")
        );
        assert_eq!(files[0].get("index").and_then(Value::as_str), Some("M"));
        assert_eq!(
            files[0].get("workingDir").and_then(Value::as_str),
            Some("M")
        );
        assert!(payload.get("diffStats").is_none());

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn session_list_route_reads_paginated_opencode_sessions() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let home = unique_tmp_dir("app-session-list");
        tokio::fs::create_dir_all(&home)
            .await
            .expect("test home should exist");
        let _home = EnvVarGuard::set("HOME", home.display().to_string());

        let workspace_root = home.join("workspace");
        let other_root = home.join("other");
        tokio::fs::create_dir_all(&workspace_root)
            .await
            .expect("workspace should exist");
        tokio::fs::create_dir_all(&other_root)
            .await
            .expect("other workspace should exist");

        seed_opencode_session_summary(&home, "global", "ses_old", &workspace_root, "old", 10.0)
            .await;
        seed_opencode_session_summary(&home, "global", "ses_new", &workspace_root, "new", 20.0)
            .await;
        seed_opencode_session_summary(&home, "global", "ses_other", &other_root, "other", 30.0)
            .await;

        let state = crate::test_support::build_test_app_state(
            &workspace_root,
            crate::settings::Settings::default(),
        )
        .await;
        let workspace_directory = workspace_root.display().to_string();
        let directory = urlencoding::encode(&workspace_directory);
        let router = Router::new()
            .route("/api/session", get(crate::opencode_session::session_list))
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/session?directory={directory}&offset=0&limit=1&includeTotal=true"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        let sessions = payload
            .get("sessions")
            .and_then(Value::as_array)
            .expect("sessions should be present");
        assert_eq!(sessions.len(), 1);
        assert_eq!(payload.get("total").and_then(Value::as_u64), Some(2));
        assert_eq!(payload.get("offset").and_then(Value::as_u64), Some(0));
        assert_eq!(payload.get("limit").and_then(Value::as_u64), Some(1));
        assert_eq!(payload.get("hasMore").and_then(Value::as_bool), Some(true));
        assert_eq!(sessions[0]["id"], json!("ses_new"));
        assert_eq!(sessions[0]["directory"], json!(workspace_directory));

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn agena_studio_session_locate_route_uses_direct_handler() {
        let extra = Router::new().route(
            "/session/{session_id}",
            get(
                |axum::extract::Path(session_id): axum::extract::Path<String>,
                 Query(query): Query<std::collections::HashMap<String, String>>| async move {
                    let directory = query
                        .get("directory")
                        .cloned()
                        .unwrap_or_else(|| "/tmp/unknown".to_string());
                    Json(json!({
                        "id": session_id,
                        "directory": directory,
                    }))
                },
            ),
        );
        let (port, handle) = spawn_mock_opencode_server(extra).await;
        let settings = crate::settings::Settings {
            projects: vec![crate::settings::Project {
                id: "proj_1".to_string(),
                path: "/tmp/placeholder".to_string(),
                added_at: 0,
                last_opened_at: 0,
            }],
            ..Default::default()
        };
        let (state, _db, _config, workspace) =
            test_app_state_with_opencode_port(port, settings).await;
        let workspace_path = workspace.path().display().to_string();
        *state.settings.write().await = crate::settings::Settings {
            projects: vec![crate::settings::Project {
                id: "proj_1".to_string(),
                path: workspace_path.clone(),
                added_at: 0,
                last_opened_at: 0,
            }],
            ..Default::default()
        };
        state
            .directory_session_index
            .upsert_summary_from_value(&json!({
                "id": "ses_locate_1",
                "directory": workspace_path,
                "time": { "updated": 0.0 }
            }));
        state
            .opencode
            .ensure_ready(std::time::Duration::from_secs(2))
            .await
            .expect("mock opencode should be ready");

        let router = Router::new()
            .route(
                "/api/agena-studio/session-locate",
                get(crate::opencode_proxy::agena_studio_session_locate),
            )
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/agena-studio/session-locate?sessionId=ses_locate_1")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        let workspace_path = workspace.path().display().to_string();
        assert_eq!(
            payload.get("sessionId").and_then(Value::as_str),
            Some("ses_locate_1")
        );
        assert_eq!(
            payload.get("projectId").and_then(Value::as_str),
            Some("proj_1")
        );
        assert_eq!(
            payload.get("projectPath").and_then(Value::as_str),
            Some(workspace_path.as_str())
        );
        assert_eq!(
            payload.get("directory").and_then(Value::as_str),
            Some(workspace_path.as_str())
        );
        assert_eq!(
            payload
                .get("session")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str),
            Some("ses_locate_1")
        );

        state.runtime.shutdown();
        handle.abort();
    }

    #[tokio::test]
    async fn session_create_route_proxies_to_opencode() {
        let captured = Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
        let captured_for_server = captured.clone();
        let extra = Router::new().route(
            "/session",
            post(
                move |Query(query): Query<std::collections::HashMap<String, String>>,
                      body: String| {
                    let captured = captured_for_server.clone();
                    async move {
                        captured.lock().expect("capture mutex").push((
                            query.get("directory").cloned().unwrap_or_default(),
                            body.clone(),
                        ));
                        (
                            StatusCode::CREATED,
                            Json(json!({
                                "id": "ses_created_1",
                                "title": "created direct",
                                "directory": query.get("directory").cloned().unwrap_or_default(),
                                "time": {
                                    "created": 1,
                                    "updated": 1
                                }
                            })),
                        )
                    }
                },
            ),
        );
        let (port, handle) = spawn_mock_opencode_server(extra).await;
        let (state, _db, _config, workspace) =
            test_app_state_with_opencode_port(port, crate::settings::Settings::default()).await;
        state
            .opencode
            .ensure_ready(Duration::from_secs(2))
            .await
            .expect("mock opencode should be ready");

        let directory = workspace.path().display().to_string();
        let router = Router::new()
            .route("/api/session", post(crate::opencode_session::session_post))
            .with_state(state.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/session?directory={}",
                        urlencoding::encode(&directory)
                    ))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({ "title": "created direct" }).to_string()))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::CREATED);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload["id"], json!("ses_created_1"));
        assert_eq!(payload["title"], json!("created direct"));
        assert_eq!(payload["directory"], json!(directory.clone()));

        let captured = captured.lock().expect("capture mutex");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, directory);
        assert_eq!(
            captured[0].1,
            json!({ "title": "created direct" }).to_string()
        );

        state.runtime.shutdown();
        handle.abort();
    }

    #[tokio::test]
    async fn session_mutation_routes_proxy_and_sanitize_payloads_via_catch_all() {
        let captured = Arc::new(std::sync::Mutex::new(Vec::<(String, String, String)>::new()));

        let patch_captured = captured.clone();
        let delete_captured = captured.clone();
        let share_post_captured = captured.clone();
        let share_delete_captured = captured.clone();
        let fork_captured = captured.clone();
        let revert_captured = captured.clone();
        let unrevert_captured = captured.clone();

        let extra = Router::new()
            .route(
                "/session/{session_id}",
                axum::routing::patch(
                    move |axum::extract::Path(session_id): axum::extract::Path<String>,
                          body: String| {
                        let captured = patch_captured.clone();
                        async move {
                            captured.lock().expect("capture mutex").push((
                                "PATCH".to_string(),
                                session_id.clone(),
                                body.clone(),
                            ));
                            let title = serde_json::from_str::<Value>(&body)
                                .ok()
                                .and_then(|value| {
                                    value
                                        .get("title")
                                        .and_then(Value::as_str)
                                        .map(ToOwned::to_owned)
                                })
                                .unwrap_or_else(|| "patched".to_string());
                            Json(json!({
                                "id": session_id,
                                "title": title,
                                "directory": "/tmp/proj",
                                "time": { "created": 1, "updated": 2, "completed": 3 },
                                "projectID": "proj_drop",
                            }))
                        }
                    },
                )
                .delete(
                    move |axum::extract::Path(session_id): axum::extract::Path<String>| {
                        let captured = delete_captured.clone();
                        async move {
                            captured.lock().expect("capture mutex").push((
                                "DELETE".to_string(),
                                session_id.clone(),
                                String::new(),
                            ));
                            Json(json!({
                                "id": session_id,
                                "title": "delete proxied",
                                "directory": "/tmp/proj",
                                "time": { "created": 3, "updated": 4 },
                                "version": 99,
                            }))
                        }
                    },
                ),
            )
            .route(
                "/session/{session_id}/share",
                post(
                    move |axum::extract::Path(session_id): axum::extract::Path<String>| {
                        let captured = share_post_captured.clone();
                        async move {
                            captured.lock().expect("capture mutex").push((
                                "POST".to_string(),
                                format!("{session_id}/share"),
                                String::new(),
                            ));
                            Json(json!({
                                "id": session_id,
                                "directory": "/tmp/proj",
                                "share": {
                                    "url": "https://example.com/share",
                                    "other": true
                                }
                            }))
                        }
                    },
                )
                .delete(
                    move |axum::extract::Path(session_id): axum::extract::Path<String>| {
                        let captured = share_delete_captured.clone();
                        async move {
                            captured.lock().expect("capture mutex").push((
                                "DELETE".to_string(),
                                format!("{session_id}/share"),
                                String::new(),
                            ));
                            Json(json!({
                                "id": session_id,
                                "directory": "/tmp/proj",
                                "share": Value::Null
                            }))
                        }
                    },
                ),
            )
            .route(
                "/session/{session_id}/fork",
                post(
                    move |axum::extract::Path(session_id): axum::extract::Path<String>,
                          body: String| {
                        let captured = fork_captured.clone();
                        async move {
                            captured.lock().expect("capture mutex").push((
                                "POST".to_string(),
                                format!("{session_id}/fork"),
                                body,
                            ));
                            Json(json!({
                                "id": "ses_fork_1",
                                "parentID": session_id,
                                "directory": "/tmp/proj",
                                "extra": true,
                            }))
                        }
                    },
                ),
            )
            .route(
                "/session/{session_id}/revert",
                post(
                    move |axum::extract::Path(session_id): axum::extract::Path<String>,
                          body: String| {
                        let captured = revert_captured.clone();
                        async move {
                            captured.lock().expect("capture mutex").push((
                                "POST".to_string(),
                                format!("{session_id}/revert"),
                                body,
                            ));
                            Json(json!({
                                "id": session_id,
                                "directory": "/tmp/proj",
                                "revert": {
                                    "messageID": "msg_1",
                                    "diff": "@@ -1 +1 @@",
                                    "noise": true
                                }
                            }))
                        }
                    },
                ),
            )
            .route(
                "/session/{session_id}/unrevert",
                post(
                    move |axum::extract::Path(session_id): axum::extract::Path<String>| {
                        let captured = unrevert_captured.clone();
                        async move {
                            captured.lock().expect("capture mutex").push((
                                "POST".to_string(),
                                format!("{session_id}/unrevert"),
                                String::new(),
                            ));
                            Json(json!({
                                "id": session_id,
                                "directory": "/tmp/proj",
                                "revert": Value::Null
                            }))
                        }
                    },
                ),
            );

        let (port, handle) = spawn_mock_opencode_server(extra).await;
        let (state, _db, _config, workspace) =
            test_app_state_with_opencode_port(port, crate::settings::Settings::default()).await;
        state
            .opencode
            .ensure_ready(Duration::from_secs(2))
            .await
            .expect("mock opencode should be ready");

        state
            .directory_session_index
            .upsert_summary_from_value(&json!({
                "id": "ses_delete_1",
                "directory": workspace.path().display().to_string(),
                "time": { "updated": 1.0 }
            }));
        assert!(
            state
                .directory_session_index
                .summary("ses_delete_1")
                .is_some(),
            "delete target should be indexed before proxy delete"
        );

        let router = Router::new()
            .route(
                "/api/{*path}",
                axum::routing::any(crate::opencode_proxy::proxy_opencode_rest),
            )
            .with_state(state.clone());

        let patch_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/session/ses_patch_1")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({ "title": "after rename" }).to_string()))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(patch_response.status(), StatusCode::OK);
        let patch_payload: Value = serde_json::from_slice(
            &patch_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            patch_payload.get("id").and_then(Value::as_str),
            Some("ses_patch_1")
        );
        assert_eq!(
            patch_payload.get("title").and_then(Value::as_str),
            Some("after rename")
        );
        assert_eq!(
            patch_payload
                .get("time")
                .and_then(|value| value.get("updated"))
                .and_then(Value::as_i64),
            Some(2)
        );
        assert!(patch_payload.get("projectID").is_none());

        let delete_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/session/ses_delete_1")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(delete_response.status(), StatusCode::OK);
        let delete_payload: Value = serde_json::from_slice(
            &delete_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            delete_payload.get("id").and_then(Value::as_str),
            Some("ses_delete_1")
        );
        assert_eq!(
            delete_payload.get("title").and_then(Value::as_str),
            Some("delete proxied")
        );
        assert!(
            state
                .directory_session_index
                .summary("ses_delete_1")
                .is_none()
        );
        assert!(
            state
                .directory_session_index
                .is_recently_deleted("ses_delete_1")
        );

        let share_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/session/ses_share_1/share")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(share_response.status(), StatusCode::OK);
        let share_payload: Value = serde_json::from_slice(
            &share_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            share_payload.get("id").and_then(Value::as_str),
            Some("ses_share_1")
        );
        assert_eq!(
            share_payload
                .get("share")
                .and_then(|value| value.get("url"))
                .and_then(Value::as_str),
            Some("https://example.com/share")
        );
        assert!(
            share_payload
                .get("share")
                .and_then(|value| value.get("other"))
                .is_none()
        );

        let unshare_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/session/ses_share_1/share")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(unshare_response.status(), StatusCode::OK);
        let unshare_payload: Value = serde_json::from_slice(
            &unshare_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            unshare_payload.get("id").and_then(Value::as_str),
            Some("ses_share_1")
        );
        assert!(unshare_payload.get("share").is_none());

        let fork_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/session/ses_parent_1/fork")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({ "messageID": "msg_1" }).to_string()))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(fork_response.status(), StatusCode::OK);
        let fork_payload: Value = serde_json::from_slice(
            &fork_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            fork_payload.get("id").and_then(Value::as_str),
            Some("ses_fork_1")
        );
        assert_eq!(
            fork_payload.get("parentID").and_then(Value::as_str),
            Some("ses_parent_1")
        );
        assert!(fork_payload.get("extra").is_none());

        let revert_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/session/ses_revert_1/revert")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({ "messageID": "msg_1" }).to_string()))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(revert_response.status(), StatusCode::OK);
        let revert_payload: Value = serde_json::from_slice(
            &revert_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            revert_payload
                .get("revert")
                .and_then(|value| value.get("messageID"))
                .and_then(Value::as_str),
            Some("msg_1")
        );
        assert_eq!(
            revert_payload
                .get("revert")
                .and_then(|value| value.get("diff"))
                .and_then(Value::as_str),
            Some("@@ -1 +1 @@")
        );
        assert!(
            revert_payload
                .get("revert")
                .and_then(|value| value.get("noise"))
                .is_none()
        );

        let unrevert_response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/session/ses_revert_1/unrevert")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(unrevert_response.status(), StatusCode::OK);
        let unrevert_payload: Value = serde_json::from_slice(
            &unrevert_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            unrevert_payload.get("id").and_then(Value::as_str),
            Some("ses_revert_1")
        );
        assert!(unrevert_payload.get("revert").is_none());

        let captured = captured.lock().expect("capture mutex");
        assert!(captured.iter().any(|entry| {
            entry.0 == "PATCH"
                && entry.1 == "ses_patch_1"
                && entry.2 == json!({ "title": "after rename" }).to_string()
        }));
        assert!(captured.iter().any(|entry| {
            entry.0 == "POST"
                && entry.1 == "ses_parent_1/fork"
                && entry.2 == json!({ "messageID": "msg_1" }).to_string()
        }));
        assert!(captured.iter().any(|entry| {
            entry.0 == "POST"
                && entry.1 == "ses_revert_1/revert"
                && entry.2 == json!({ "messageID": "msg_1" }).to_string()
        }));

        state.runtime.shutdown();
        handle.abort();
    }

    #[tokio::test]
    async fn session_summarize_route_proxies_ack_via_catch_all() {
        let captured = Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
        let captured_for_server = captured.clone();
        let extra = Router::new().route(
            "/session/{session_id}/summarize",
            post(
                move |axum::extract::Path(session_id): axum::extract::Path<String>,
                      body: String| {
                    let captured = captured_for_server.clone();
                    async move {
                        captured
                            .lock()
                            .expect("capture mutex")
                            .push((session_id, body));
                        Json(json!({
                            "ok": true,
                            "queued": true,
                            "auto": false
                        }))
                    }
                },
            ),
        );
        let (port, handle) = spawn_mock_opencode_server(extra).await;
        let (state, _db, _config, _workspace) =
            test_app_state_with_opencode_port(port, crate::settings::Settings::default()).await;
        state
            .opencode
            .ensure_ready(Duration::from_secs(2))
            .await
            .expect("mock opencode should be ready");

        let request_body = json!({
            "providerID": "openai",
            "modelID": "gpt-4.1-mini",
            "auto": false
        });
        let router = Router::new()
            .route(
                "/api/{*path}",
                axum::routing::any(crate::opencode_proxy::proxy_opencode_rest),
            )
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/session/ses_sum_1/summarize")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body.to_string()))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        assert_eq!(payload.get("queued").and_then(Value::as_bool), Some(true));
        assert_eq!(payload.get("auto").and_then(Value::as_bool), Some(false));

        let captured = captured.lock().expect("capture mutex");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, "ses_sum_1");
        assert_eq!(captured[0].1, request_body.to_string());

        state.runtime.shutdown();
        handle.abort();
    }

    #[tokio::test]
    async fn session_diff_route_reads_authoritative_opencode_history_via_catch_all() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let home = unique_tmp_dir("app-session-diff");
        tokio::fs::create_dir_all(&home)
            .await
            .expect("test home should exist");
        let _home = EnvVarGuard::set("HOME", home.display().to_string());

        let workspace_root = home.join("workspace");
        tokio::fs::create_dir_all(workspace_root.join("src"))
            .await
            .expect("workspace should exist");
        let directory = workspace_root.display().to_string();

        seed_opencode_message_with_parts(
            &home,
            "ses_diff_1",
            "msg_old",
            "assistant",
            10.0,
            vec![json!({
                "id": "part_old",
                "type": "tool",
                "state": {
                    "metadata": {
                        "diff": "diff --git a/src/old.ts b/src/old.ts\n--- a/src/old.ts\n+++ b/src/old.ts\n@@ -1 +1,2 @@\n-a\n+b\n+c"
                    }
                }
            })],
        )
        .await;
        seed_opencode_message_with_parts(
            &home,
            "ses_diff_1",
            "msg_new",
            "assistant",
            20.0,
            vec![json!({
                "id": "part_new",
                "type": "tool",
                "state": {
                    "metadata": {
                        "files": [
                            {
                                "path": format!("{directory}/src/new.ts"),
                                "before": "old\n",
                                "after": "new\n",
                                "additions": 1,
                                "deletions": 1
                            }
                        ]
                    }
                }
            })],
        )
        .await;

        let state = crate::test_support::build_test_app_state(
            &workspace_root,
            crate::settings::Settings::default(),
        )
        .await;
        let router = Router::new()
            .route(
                "/api/{*path}",
                axum::routing::any(crate::opencode_proxy::proxy_opencode_rest),
            )
            .with_state(state.clone());

        let page_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/session/ses_diff_1/diff?directory={}&offset=0&limit=1",
                        urlencoding::encode(&directory)
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(page_response.status(), StatusCode::OK);
        let page_payload: Value = serde_json::from_slice(
            &page_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(page_payload.get("total").and_then(Value::as_u64), Some(2));
        assert_eq!(page_payload.get("offset").and_then(Value::as_u64), Some(0));
        assert_eq!(page_payload.get("limit").and_then(Value::as_u64), Some(1));
        assert_eq!(
            page_payload.get("hasMore").and_then(Value::as_bool),
            Some(true)
        );
        let items = page_payload
            .get("items")
            .and_then(Value::as_array)
            .expect("items should be present");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("file").and_then(Value::as_str),
            Some("src/new.ts")
        );
        assert_eq!(items[0].get("additions").and_then(Value::as_u64), Some(1));
        assert_eq!(items[0].get("deletions").and_then(Value::as_u64), Some(1));

        let second_page_response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/session/ses_diff_1/diff?directory={}&offset=1&limit=1",
                        urlencoding::encode(&directory)
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(second_page_response.status(), StatusCode::OK);
        let second_page_payload: Value = serde_json::from_slice(
            &second_page_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        let second_page_items = second_page_payload
            .get("items")
            .and_then(Value::as_array)
            .expect("items should be present");
        assert_eq!(second_page_items.len(), 1);
        assert_eq!(
            second_page_items[0].get("file").and_then(Value::as_str),
            Some("src/old.ts")
        );
        assert!(
            second_page_items[0]
                .get("diff")
                .and_then(Value::as_str)
                .is_some_and(|diff| diff.contains("diff --git a/src/old.ts b/src/old.ts"))
        );

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn session_status_route_uses_direct_local_snapshot() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        state
            .directory_session_index
            .upsert_runtime_status("ses_local_1", "idle");

        let router = Router::new()
            .route(
                "/api/session/status",
                get(crate::opencode_proxy::session_status_get),
            )
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/session/status?sessionId=ses_local_1&local=true")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            payload
                .get("ses_local_1")
                .and_then(|entry| entry.get("type"))
                .and_then(Value::as_str),
            Some("idle")
        );

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn api_proxy_catch_all_preserves_api_v1_and_proxies_unhandled_paths() {
        let extra = Router::new().route(
            "/session/{session_id}/abort",
            post(
                |axum::extract::Path(session_id): axum::extract::Path<String>| async move {
                    Json(json!({
                        "proxied": true,
                        "sessionId": session_id,
                    }))
                },
            ),
        );
        let (port, handle) = spawn_mock_opencode_server(extra).await;
        let (state, _db, _config, _workspace) =
            test_app_state_with_opencode_port(port, crate::settings::Settings::default()).await;
        state
            .opencode
            .ensure_ready(std::time::Duration::from_secs(2))
            .await
            .expect("mock opencode should be ready");

        let router = Router::new()
            .route(
                "/api/v1/health",
                get(|| async { Json(json!({ "source": "agena-v1" })) }),
            )
            .route(
                "/api/{*path}",
                axum::routing::any(crate::opencode_proxy::proxy_opencode_rest),
            )
            .with_state(state.clone());

        let v1_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(v1_response.status(), StatusCode::OK);
        let v1_payload: Value = serde_json::from_slice(
            &v1_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(v1_payload["source"], json!("agena-v1"));

        let proxy_response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/session/ses_proxy_1/abort")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(proxy_response.status(), StatusCode::OK);
        let proxy_payload: Value = serde_json::from_slice(
            &proxy_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(proxy_payload["proxied"], json!(true));
        assert_eq!(proxy_payload["sessionId"], json!("ses_proxy_1"));

        state.runtime.shutdown();
        handle.abort();
    }

    #[tokio::test]
    async fn session_message_routes_read_direct_opencode_storage() {
        let _env_lock = ENV_LOCK.lock().unwrap();
        let home = unique_tmp_dir("app-session-message");
        tokio::fs::create_dir_all(&home)
            .await
            .expect("test home should exist");
        let _home = EnvVarGuard::set("HOME", home.display().to_string());

        let workspace_root = home.join("workspace");
        tokio::fs::create_dir_all(&workspace_root)
            .await
            .expect("workspace should exist");

        seed_opencode_message_with_parts(
            &home,
            "ses_msg_1",
            "msg_1",
            "user",
            1.0,
            vec![json!({
                "id": "part_1",
                "type": "text",
                "text": "hello direct session"
            })],
        )
        .await;

        let state = crate::test_support::build_test_app_state(
            &workspace_root,
            crate::settings::Settings::default(),
        )
        .await;

        let router = Router::new()
            .route(
                "/api/session/{session_id}/message",
                get(crate::opencode_session::session_message_get),
            )
            .route(
                "/api/session/{session_id}/message/{message_id}/part/{part_id}",
                get(crate::opencode_session::session_message_part_get),
            )
            .with_state(state.clone());

        let list_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/session/ses_msg_1/message?offset=0&limit=10&includeTotal=true")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(list_response.status(), StatusCode::OK);
        let list_payload: Value = serde_json::from_slice(
            &list_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        let entries = list_payload
            .get("entries")
            .and_then(Value::as_array)
            .expect("entries should be present");
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0]
                .get("info")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str),
            Some("msg_1")
        );
        assert_eq!(
            entries[0]
                .get("parts")
                .and_then(Value::as_array)
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("partId"))
                .and_then(Value::as_str),
            Some("part_1")
        );

        let part_response = router
            .oneshot(
                Request::builder()
                    .uri("/api/session/ses_msg_1/message/msg_1/part/part_1")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(part_response.status(), StatusCode::OK);
        let part_payload: Value = serde_json::from_slice(
            &part_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            part_payload.get("id").and_then(Value::as_str),
            Some("part_1")
        );
        assert_eq!(
            part_payload.get("text").and_then(Value::as_str),
            Some("hello direct session")
        );

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn session_message_post_route_proxies_to_prompt_async() {
        let captured = Arc::new(std::sync::Mutex::new(Vec::<(String, String, String)>::new()));
        let captured_for_server = captured.clone();
        let extra = Router::new().route(
            "/session/{session_id}/prompt_async",
            post(
                move |axum::extract::Path(session_id): axum::extract::Path<String>,
                      Query(query): Query<std::collections::HashMap<String, String>>,
                      body: String| {
                    let captured = captured_for_server.clone();
                    async move {
                        captured.lock().expect("capture mutex").push((
                            session_id,
                            query.get("directory").cloned().unwrap_or_default(),
                            body,
                        ));
                        Json(json!({ "ok": true }))
                    }
                },
            ),
        );
        let (port, handle) = spawn_mock_opencode_server(extra).await;
        let (state, _db, _config, workspace) =
            test_app_state_with_opencode_port(port, crate::settings::Settings::default()).await;
        state
            .opencode
            .ensure_ready(Duration::from_secs(2))
            .await
            .expect("mock opencode should be ready");

        let directory = workspace.path().display().to_string();
        let request_body = json!({
            "parts": [
                { "type": "text", "text": "hello direct" }
            ]
        });
        let router = Router::new()
            .route(
                "/api/session/{session_id}/message",
                post(crate::opencode_proxy::session_message_post),
            )
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/session/ses_msg_post_1/message?directory={}",
                        urlencoding::encode(&directory)
                    ))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request_body.to_string()))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        assert_eq!(status, StatusCode::ACCEPTED);
        let payload: Value =
            serde_json::from_slice(body.as_ref()).expect("response should be valid json");
        assert_eq!(payload.get("queued").and_then(Value::as_bool), Some(true));

        let captured = captured.lock().expect("capture mutex");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, "ses_msg_post_1");
        assert_eq!(captured[0].1, directory);
        assert_eq!(captured[0].2, request_body.to_string());

        state.runtime.shutdown();
        handle.abort();
    }

    #[tokio::test]
    async fn terminal_routes_create_get_and_delete_session() {
        let (state, _db, _config, workspace) = compat_test_app_state().await;
        let cwd = workspace.path().display().to_string();

        let router = Router::new()
            .route(
                "/api/terminal/create",
                post(crate::terminal::terminal_create),
            )
            .route(
                "/api/terminal/{session_id}",
                get(crate::terminal::terminal_get).delete(crate::terminal::terminal_delete),
            )
            .with_state(state.clone());

        let create_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/terminal/create")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "cwd": cwd,
                            "cols": 90,
                            "rows": 30
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(create_response.status(), StatusCode::OK);
        let create_payload: Value = serde_json::from_slice(
            &create_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        let session_id = create_payload
            .get("sessionId")
            .and_then(Value::as_str)
            .expect("session id should be present")
            .to_string();
        assert_eq!(create_payload.get("cols").and_then(Value::as_u64), Some(90));
        assert_eq!(create_payload.get("rows").and_then(Value::as_u64), Some(30));

        let get_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/terminal/{session_id}"))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(get_response.status(), StatusCode::OK);
        let get_payload: Value = serde_json::from_slice(
            &get_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            get_payload.get("sessionId").and_then(Value::as_str),
            Some(session_id.as_str())
        );
        assert_eq!(
            get_payload.get("cwd").and_then(Value::as_str),
            Some(cwd.as_str())
        );

        let delete_response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/terminal/{session_id}"))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(delete_response.status(), StatusCode::OK);
        let delete_payload: Value = serde_json::from_slice(
            &delete_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            delete_payload.get("success").and_then(Value::as_bool),
            Some(true)
        );

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_git_status_route_includes_diff_stats_for_page_files() {
        let temp = tempdir().expect("tempdir should be created");
        let repo = temp.path();
        init_git_repo(repo);

        std::fs::write(repo.join("tracked.txt"), "alpha\n")
            .expect("tracked file should be written");
        git_ok(repo, &["add", "tracked.txt"]);
        git_ok(repo, &["commit", "-m", "base"]);

        std::fs::write(repo.join("tracked.txt"), "alpha\nbeta\n")
            .expect("tracked file should be updated");
        std::fs::write(repo.join("untracked.txt"), "one\ntwo\n")
            .expect("untracked file should be written");

        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let router = Router::new()
            .route("/api/git/status", get(crate::git::git_status))
            .with_state(state.clone());
        let directory_value = repo.display().to_string();
        let directory = urlencoding::encode(&directory_value);
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/git/status?directory={directory}&includeDiffStats=true&limit=10"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload.get("scope").and_then(Value::as_str), Some("all"));
        assert_eq!(payload.get("totalFiles").and_then(Value::as_u64), Some(2));
        assert_eq!(payload.get("stagedCount").and_then(Value::as_u64), Some(0));
        assert_eq!(
            payload.get("unstagedCount").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            payload.get("untrackedCount").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(payload.get("mergeCount").and_then(Value::as_u64), Some(0));
        let files = payload
            .get("files")
            .and_then(Value::as_array)
            .expect("files should be present");
        assert_eq!(files.len(), 2);

        let diff_stats = payload
            .get("diffStats")
            .and_then(Value::as_object)
            .expect("diff stats should be present");
        assert_eq!(
            diff_stats
                .get("tracked.txt")
                .and_then(|value| value.get("insertions"))
                .and_then(Value::as_i64),
            Some(1)
        );
        assert_eq!(
            diff_stats
                .get("tracked.txt")
                .and_then(|value| value.get("deletions"))
                .and_then(Value::as_i64),
            Some(0)
        );
        assert_eq!(
            diff_stats
                .get("untracked.txt")
                .and_then(|value| value.get("insertions"))
                .and_then(Value::as_i64),
            Some(2)
        );
        assert_eq!(
            diff_stats
                .get("untracked.txt")
                .and_then(|value| value.get("deletions"))
                .and_then(Value::as_i64),
            Some(0)
        );
        assert_eq!(diff_stats.len(), files.len());

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_git_commit_file_routes_return_history_diff_and_content() {
        let temp = tempdir().expect("tempdir should be created");
        let repo = temp.path();
        init_git_repo(repo);

        let file = repo.join("notes.txt");
        std::fs::write(&file, "alpha\nbeta\n").expect("file should be written");
        git_ok(repo, &["add", "notes.txt"]);
        git_ok(repo, &["commit", "-m", "init"]);

        std::fs::write(&file, "alpha\nbeta changed\ncharlie\n").expect("file should be rewritten");
        git_ok(repo, &["add", "notes.txt"]);
        git_ok(repo, &["commit", "-m", "update notes"]);

        let commit = git_output(repo, &["rev-parse", "HEAD"]).expect("commit hash should exist");
        let directory_value = repo.display().to_string();
        let directory = urlencoding::encode(&directory_value);
        let router = Router::new()
            .route(
                "/api/git/commit-file-diff",
                get(crate::git::git_commit_file_diff),
            )
            .route(
                "/api/git/commit-file-content",
                get(crate::git::git_commit_file_content),
            );

        let diff_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/git/commit-file-diff?directory={directory}&commit={commit}&path=notes.txt&contextLines=1"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(diff_response.status(), StatusCode::OK);
        let diff_payload: Value = serde_json::from_slice(
            &diff_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert!(
            diff_payload["diff"]
                .as_str()
                .expect("diff payload should include text diff")
                .contains("beta changed")
        );
        assert!(
            diff_payload["diff"]
                .as_str()
                .expect("diff payload should include text diff")
                .contains("charlie")
        );

        let content_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/git/commit-file-content?directory={directory}&commit={commit}&path=notes.txt"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(content_response.status(), StatusCode::OK);
        let content_payload: Value = serde_json::from_slice(
            &content_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(content_payload["exists"], json!(true));
        assert_eq!(content_payload["binary"], json!(false));
        assert_eq!(content_payload["truncated"], json!(false));
        assert_eq!(
            content_payload["content"],
            json!("alpha\nbeta changed\ncharlie\n")
        );

        let missing_response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/git/commit-file-content?directory={directory}&commit={commit}&path=missing.txt"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(missing_response.status(), StatusCode::OK);
        let missing_payload: Value = serde_json::from_slice(
            &missing_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(missing_payload["exists"], json!(false));
        assert_eq!(missing_payload["binary"], json!(false));
        assert_eq!(missing_payload["truncated"], json!(false));
        assert_eq!(missing_payload["content"], json!(""));
    }

    #[tokio::test]
    async fn compat_git_conflict_routes_read_and_resolve_markers() {
        let temp = tempdir().expect("tempdir should be created");
        let repo = temp.path();
        init_git_repo(repo);

        let file = repo.join("conflict.txt");
        std::fs::write(&file, "shared\nbase line\n").expect("file should be written");
        git_ok(repo, &["add", "conflict.txt"]);
        git_ok(repo, &["commit", "-m", "base"]);

        let base_branch =
            git_output(repo, &["branch", "--show-current"]).expect("branch name should exist");
        git_ok(repo, &["checkout", "-b", "feature"]);
        std::fs::write(&file, "shared\nfeature line\n").expect("feature change should be written");
        git_ok(repo, &["add", "conflict.txt"]);
        git_ok(repo, &["commit", "-m", "feature change"]);

        git_ok(repo, &["checkout", base_branch.as_str()]);
        std::fs::write(&file, "shared\nmain line\n").expect("main change should be written");
        git_ok(repo, &["add", "conflict.txt"]);
        git_ok(repo, &["commit", "-m", "main change"]);

        let merge_output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["merge", "feature"])
            .output()
            .expect("git merge should run");
        assert!(
            !merge_output.status.success(),
            "merge should create a conflict"
        );

        let directory_value = repo.display().to_string();
        let directory = urlencoding::encode(&directory_value);
        let router = Router::new()
            .route(
                "/api/git/conflicts/file",
                get(crate::git::git_conflict_file),
            )
            .route(
                "/api/git/conflicts/resolve",
                post(crate::git::git_conflict_resolve),
            );

        let conflict_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/git/conflicts/file?directory={directory}&path=conflict.txt"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(conflict_response.status(), StatusCode::OK);
        let conflict_payload: Value = serde_json::from_slice(
            &conflict_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(conflict_payload["hasMarkers"], json!(true));
        assert_eq!(conflict_payload["isUnmerged"], json!(true));
        let blocks = conflict_payload["blocks"]
            .as_array()
            .expect("conflict response should contain block array");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["ours"], json!("main line"));
        assert_eq!(blocks[0]["theirs"], json!("feature line"));

        let resolve_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/git/conflicts/resolve?directory={directory}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "path": "conflict.txt",
                            "strategy": "manual",
                            "stage": true,
                            "choices": [{"id": 0, "choice": "theirs"}]
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(resolve_response.status(), StatusCode::OK);
        let resolve_payload: Value = serde_json::from_slice(
            &resolve_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(resolve_payload["success"], json!(true));
        assert_eq!(
            std::fs::read_to_string(&file).expect("resolved file should remain readable"),
            "shared\nfeature line\n"
        );

        let after_response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/git/conflicts/file?directory={directory}&path=conflict.txt"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(after_response.status(), StatusCode::OK);
        let after_payload: Value = serde_json::from_slice(
            &after_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(after_payload["hasMarkers"], json!(false));
        assert_eq!(after_payload["isUnmerged"], json!(false));
        assert!(
            after_payload["blocks"]
                .as_array()
                .expect("conflict response should contain block array")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn compat_git_blame_diff_patch_and_watch_routes_work() {
        let temp = tempdir().expect("tempdir should be created");
        let repo = temp.path();
        init_git_repo(repo);

        let file = repo.join("notes.txt");
        std::fs::write(&file, "alpha\nbeta\n").expect("file should be written");
        git_ok(repo, &["add", "notes.txt"]);
        git_ok(repo, &["commit", "-m", "init"]);

        std::fs::write(&file, "alpha\nbeta changed\n").expect("file should be rewritten");
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let directory_value = repo.display().to_string();
        let directory = urlencoding::encode(&directory_value);
        let router = Router::new()
            .route("/api/git/blame", get(crate::git::git_blame))
            .route("/api/git/diff", get(crate::git::git_diff))
            .route("/api/git/file-diff", get(crate::git::git_file_diff))
            .route("/api/git/patch", post(crate::git::git_apply_patch))
            .route("/api/git/watch", get(crate::git::git_watch))
            .with_state(state.clone());

        let blame_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/git/blame?directory={directory}&path=notes.txt"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(blame_response.status(), StatusCode::OK);
        let blame_payload: Value = serde_json::from_slice(
            &blame_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            blame_payload["lines"]
                .as_array()
                .expect("blame response should contain lines")
                .len(),
            2
        );

        let diff_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/git/diff?directory={directory}&path=notes.txt&staged=false&contextLines=3&includeMeta=true"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(diff_response.status(), StatusCode::OK);
        let diff_payload: Value = serde_json::from_slice(
            &diff_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        let diff_text = diff_payload["diff"]
            .as_str()
            .expect("diff response should contain unified diff");
        assert!(diff_text.contains("beta changed"));
        assert!(
            diff_payload["meta"]["hunks"]
                .as_array()
                .is_some_and(|hunks| !hunks.is_empty())
        );

        let file_diff_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/git/file-diff?directory={directory}&path=notes.txt&staged=false"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(file_diff_response.status(), StatusCode::OK);
        let file_diff_payload: Value = serde_json::from_slice(
            &file_diff_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert!(
            file_diff_payload["original"]
                .as_str()
                .expect("file diff response should include original")
                .contains("beta")
        );
        assert!(
            file_diff_payload["modified"]
                .as_str()
                .expect("file diff response should include modified")
                .contains("beta changed")
        );

        let patch_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/git/patch?directory={directory}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "patch": diff_text,
                            "mode": "discard"
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(patch_response.status(), StatusCode::OK);
        let patch_payload: Value = serde_json::from_slice(
            &patch_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(patch_payload["success"], json!(true));
        assert_eq!(
            std::fs::read_to_string(&file).expect("file should stay readable"),
            "alpha\nbeta\n"
        );

        let watch_response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/git/watch?directory={directory}&intervalMs=500"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(watch_response.status(), StatusCode::OK);
        assert_eq!(
            watch_response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );

        state.runtime.shutdown();
    }
}
