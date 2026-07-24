use std::path::PathBuf;

use axum::{Json, extract::Query};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::server::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerHealthResponse {
    status: &'static str,
    generation: u64,
    loaded_at: String,
    workspace_root: String,
    config_path: String,
    config_found: bool,
    provider_ids: Vec<String>,
    session_runtime_available: bool,
}

pub(crate) async fn health(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<ServerHealthResponse> {
    let snapshot = state.application.runtime_diagnostics().await;
    Json(ServerHealthResponse {
        status: "ok",
        generation: snapshot.generation,
        loaded_at: snapshot.loaded_at.to_rfc3339(),
        workspace_root: snapshot.workspace_root.display().to_string(),
        config_path: snapshot.config_path.display().to_string(),
        config_found: snapshot.config_found,
        provider_ids: snapshot.provider_ids,
        session_runtime_available: snapshot.session_runtime_available,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsQuery {
    directory: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticPathRecord {
    path: String,
    exists: bool,
}

fn diag_entry(path: PathBuf) -> DiagnosticPathRecord {
    let text = path.to_string_lossy().into_owned();
    let exists = std::fs::metadata(&path).is_ok();
    DiagnosticPathRecord { path: text, exists }
}

pub(crate) async fn agena_diagnostics(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Query(query): Query<DiagnosticsQuery>,
) -> Json<Value> {
    let snapshot = state.application.runtime_diagnostics().await;
    let normalized_directory = query
        .directory
        .as_deref()
        .map(crate::server::path_utils::normalize_directory_path)
        .and_then(|text| {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            }
        });

    Json(json!({
        "timestamp": time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        "runtime": {
            "generation": snapshot.generation,
            "loadedAt": snapshot.loaded_at.to_rfc3339(),
            "workspaceRoot": snapshot.workspace_root.display().to_string(),
            "configPath": snapshot.config_path.display().to_string(),
            "configFound": snapshot.config_found,
            "providerIds": snapshot.provider_ids,
            "sessionRuntimeAvailable": snapshot.session_runtime_available,
        },
        "server": {
            "apiSurface": "agena-native",
        },
        "paths": {
            "input": {
                "directory": query.directory,
                "normalizedDirectory": normalized_directory.as_ref().map(|path| path.to_string_lossy().into_owned())
            },
            "server": {
                "dataDirCandidates": crate::server::persistence::paths::server_data_dir_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "dbPath": diag_entry(crate::server::persistence::paths::server_state_db_path()),
                "dbCandidates": crate::server::persistence::paths::server_state_db_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "settingsPath": diag_entry(crate::server::persistence::paths::server_settings_path()),
                "settingsCandidates": crate::server::persistence::paths::server_settings_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "terminalUiStatePath": diag_entry(crate::server::persistence::paths::terminal_ui_state_path()),
                "terminalUiStateCandidates": crate::server::persistence::paths::terminal_ui_state_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>(),
                "terminalRegistryPath": diag_entry(crate::server::persistence::paths::terminal_session_registry_path()),
                "terminalRegistryCandidates": crate::server::persistence::paths::terminal_session_registry_path_candidates().into_iter().map(diag_entry).collect::<Vec<_>>()
            }
        },
        "environment": {
            "HOME": std::env::var("HOME").ok(),
            "USERPROFILE": std::env::var("USERPROFILE").ok(),
            "APPDATA": std::env::var("APPDATA").ok(),
            "LOCALAPPDATA": std::env::var("LOCALAPPDATA").ok(),
            "AGENA_SERVER_DATA_DIR": std::env::var("AGENA_SERVER_DATA_DIR").ok(),
            "AGENA_SERVER_HOST": std::env::var("AGENA_SERVER_HOST").ok(),
            "AGENA_SERVER_PORT": std::env::var("AGENA_SERVER_PORT").ok(),
        }
    }))
}
