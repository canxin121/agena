use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::Value;

use crate::server::settings;

mod merge;
mod response;
mod sanitize;

use merge::merge_persisted_settings;
pub(crate) use response::format_settings_response;
use sanitize::sanitize_settings_update;

async fn validate_project_entries(projects: &[settings::Project]) -> Vec<settings::Project> {
    let mut out = Vec::new();
    for p in projects {
        if p.path.trim().is_empty() {
            continue;
        }
        match tokio::fs::metadata(&p.path).await {
            Ok(meta) => {
                if meta.is_dir() {
                    out.push(p.clone());
                }
            }
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    continue;
                }
                out.push(p.clone());
            }
        }
    }
    out
}

pub async fn config_settings_get(State(state): State<Arc<crate::AppState>>) -> Response {
    let current = state.settings.read().await.clone();
    let value = serde_json::to_value(&current).unwrap_or(serde_json::json!({}));
    Json(format_settings_response(&value)).into_response()
}

pub async fn config_settings_put(
    State(state): State<Arc<crate::AppState>>,
    Json(body): Json<Value>,
) -> Response {
    let mut guard = state.settings.write().await;
    let current_value = serde_json::to_value(&*guard).unwrap_or(serde_json::json!({}));
    let sanitized = sanitize_settings_update(&body);
    let merged = merge_persisted_settings(&current_value, &sanitized);

    let mut next_settings =
        serde_json::from_value::<settings::Settings>(merged.clone()).unwrap_or_default();

    if !next_settings.projects.is_empty() {
        next_settings.projects = validate_project_entries(&next_settings.projects).await;
    }

    *guard = next_settings.clone();
    if let Err(err) =
        settings::persist_settings(state.server_state_db.as_ref(), &next_settings).await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": err.to_string()})),
        )
            .into_response();
    }

    let out = serde_json::to_value(&next_settings).unwrap_or(serde_json::json!({}));
    let formatted = format_settings_response(&out);
    crate::server::settings::events::publish_settings_replace(formatted.clone()).await;
    Json(formatted).into_response()
}
