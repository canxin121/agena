//! Browser page APIs and public, immutable assets of installed plugins.

use agena_api::resource::{
    PluginWebCommandRequest, PluginWebSettingsQuery, PluginWebSettingsUpdateRequest,
};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::AppState;
use crate::error::ServerError;

/// These routes belong inside the host's normal authenticated API router.
pub(crate) fn operation_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/plugins/{plugin_id}/web/{page_id}/commands/{command_id}",
            post(invoke_command),
        )
        .route(
            "/api/v1/plugins/{plugin_id}/web/{page_id}/settings",
            get(read_settings).put(update_settings),
        )
        .layer(DefaultBodyLimit::max(1024 * 1024))
}

/// This router serves build artifacts only and is mounted outside API auth.
/// Every prefix shape is claimed so an absent asset never becomes SPA HTML.
pub(crate) fn asset_routes() -> Router<AppState> {
    Router::new()
        .route("/plugin-assets", get(missing_asset))
        .route("/plugin-assets/", get(missing_asset))
        .route("/plugin-assets/{plugin_id}", get(missing_asset))
        .route("/plugin-assets/{plugin_id}/", get(missing_asset))
        .route("/plugin-assets/{plugin_id}/{revision}", get(missing_asset))
        .route("/plugin-assets/{plugin_id}/{revision}/", get(missing_asset))
        .route("/plugin-assets/{plugin_id}/{revision}/{*path}", get(asset))
}

async fn missing_asset() -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn asset(
    State(state): State<AppState>,
    Path((plugin_id, revision, path)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Response {
    let Ok(asset) = state
        .plugin_runtime()
        .plugin_web_asset(&plugin_id, &revision, &path)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let etag = format!("\"{}\"", blake3::hash(&asset.bytes).to_hex());
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|tag| tag.trim() == etag || tag.trim() == "*")
        });
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, asset.content_type)], asset.bytes).into_response()
    };
    let response_headers = response.headers_mut();
    response_headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    response_headers.insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("hex content digest is a valid header"),
    );
    response_headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response_headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    // Direct navigation to an HTML/SVG build artifact does not turn it into
    // an authenticated host document. Imported modules use the parent CSP.
    response_headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("sandbox; default-src 'none'; frame-ancestors 'none'"),
    );
    response
}

async fn invoke_command(
    State(state): State<AppState>,
    Path((plugin_id, page_id, command_id)): Path<(String, String, String)>,
    Json(request): Json<PluginWebCommandRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let result = state
        .application()
        .command_plugin_management(agena_api::management::PluginCommand::InvokeWebCommand {
            plugin_id,
            page_id,
            generation: request.generation,
            command_id,
            input: request.input,
            session_id: request.session_id,
        })
        .await
        .map_err(ServerError::from)?;
    Ok(Json(result))
}

async fn read_settings(
    State(state): State<AppState>,
    Path((plugin_id, page_id)): Path<(String, String)>,
    Query(request): Query<PluginWebSettingsQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .application()
            .plugin_web_settings(&plugin_id, &page_id, &request.generation)
            .map_err(ServerError::from)?,
    ))
}

async fn update_settings(
    State(state): State<AppState>,
    Path((plugin_id, page_id)): Path<(String, String)>,
    Json(request): Json<PluginWebSettingsUpdateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .application()
            .update_plugin_web_settings(&plugin_id, &page_id, &request.generation, request.value)
            .await
            .map_err(ServerError::from)?,
    ))
}
