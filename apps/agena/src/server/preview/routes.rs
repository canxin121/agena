use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, OriginalUri, Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Request, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::{
    CloseFrame as TungsteniteCloseFrame, Message as TungsteniteMessage,
};
use url::{Host, Url};

use crate::server::preview::registry::{
    PreviewSessionCreateRequest, PreviewSessionRecord, PreviewSessionUpdatePatch,
    PreviewSessionsResponse, build_proxy_target_url, validate_preview_target_url,
    websocket_target_url,
};
use crate::{ApiResult, AppError, AppState};

const PREVIEW_DISCOVERY_HOSTS: [&str; 2] = ["127.0.0.1", "localhost"];
const PREVIEW_DISCOVERY_PORTS: [u16; 8] = [5173, 3000, 4173, 8080, 8000, 4200, 4321, 5174];
const PREVIEW_PROXY_BASE_PREFIX: &str = "/api/v1/workbench/preview/s/";

#[derive(Debug, Deserialize)]
pub(crate) struct WorkspacePreviewQuery {
    pub(crate) directory: Option<String>,
    pub(crate) target: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkspacePreviewProxyQuery {
    pub(crate) target: String,
    pub(crate) refresh: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspacePreviewResponse {
    pub(crate) url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspacePreviewSessionCreateBody {
    pub(crate) id: String,
    pub(crate) directory: String,
    pub(crate) run_directory: String,
    pub(crate) agena_session_id: Option<String>,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) logs_path: String,
    pub(crate) target_url: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspacePreviewSessionDiscoverBody {
    pub(crate) id: String,
    pub(crate) directory: String,
    pub(crate) run_directory: String,
    pub(crate) agena_session_id: Option<String>,
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) logs_path: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspacePreviewSessionUpdateBody {
    pub(crate) directory: Option<String>,
    pub(crate) run_directory: Option<String>,
    pub(crate) agena_session_id: Option<String>,
    pub(crate) command: Option<String>,
    pub(crate) args: Option<Vec<String>>,
    pub(crate) logs_path: Option<String>,
    pub(crate) target_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspacePreviewSessionDeleteResponse {
    pub(crate) ok: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspacePreviewSessionRenameBody {
    pub(crate) id: String,
}

pub(crate) async fn workspace_preview_get(
    Query(query): Query<WorkspacePreviewQuery>,
) -> ApiResult<Json<WorkspacePreviewResponse>> {
    if let Some(raw_target) = query.target.as_deref() {
        let target = validate_preview_target_url(raw_target)?;
        return Ok(Json(WorkspacePreviewResponse {
            url: target.to_string(),
        }));
    }

    let _ = query.directory;
    if let Some(target) = discover_preview_target().await? {
        return Ok(Json(WorkspacePreviewResponse {
            url: target.to_string(),
        }));
    }

    Err(AppError::not_found(
        "No reachable local preview target found",
    ))
}

pub(crate) async fn workspace_preview_url_get(
    query: Query<WorkspacePreviewQuery>,
) -> ApiResult<Json<WorkspacePreviewResponse>> {
    workspace_preview_get(query).await
}

pub(crate) async fn workspace_preview_proxy_get(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WorkspacePreviewProxyQuery>,
) -> ApiResult<Response> {
    let _ = query.refresh;
    let target = validate_preview_target_url(&query.target)?;
    state.workspace_preview_access.validate_upstream(&target)?;
    let client = preview_proxy_client()?;

    let upstream = tokio::time::timeout(Duration::from_secs(15), client.get(target.clone()).send())
        .await
        .map_err(|_| AppError::bad_gateway("preview upstream response timed out"))?
        .map_err(|err| {
            AppError::bad_gateway(format!("failed to fetch preview target {target}: {err}"))
        })?;

    response_from_upstream(upstream).await
}

pub(crate) async fn workspace_preview_sessions_get(
    State(state): State<std::sync::Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let etag = format!("W/\"{}\"", state.workspace_preview_registry.revision());
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|h| h.to_str().ok())
        == Some(etag.as_str());
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Json(state.workspace_preview_registry.list_all().await?).into_response()
    };
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("preview revision tag"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    Ok(response)
}

pub(crate) async fn workspace_preview_sessions_events(
    State(state): State<Arc<AppState>>,
) -> Response {
    use axum::response::sse::{Event, KeepAlive, Sse};
    let registry = state.workspace_preview_registry.clone();
    let mut changes = registry.subscribe();
    let stream = async_stream::stream! {
        let mut previous: Option<String> = None;
        let mut failures = 0_u32;
        loop {
            let (revision, payload) = match registry.event_payload(previous.as_deref()).await {
                Ok(frame) => { failures = 0; frame }
                Err(error) => {
                    tracing::warn!(target: "agena.preview_registry", error = %error, "Failed to prepare preview event");
                    failures = (failures + 1).min(5);
                    tokio::time::sleep(Duration::from_secs((5_u64 << (failures - 1)).min(60))).await;
                    continue;
                }
            };
            if let Some(payload) = payload {
                yield Ok::<_, std::convert::Infallible>(Event::default().data(payload.as_ref()));
            }
            previous = Some(revision);
            if changes.changed().await.is_err() { break; }
        }
    };
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(25)))
        .into_response()
}

pub(crate) async fn workspace_preview_sessions_by_id_get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<Json<PreviewSessionRecord>> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(AppError::bad_request("id is required"));
    }

    let session = state
        .workspace_preview_registry
        .get_by_id(trimmed)
        .await
        .ok_or_else(|| AppError::not_found(format!("preview session not found: {trimmed}")))?;
    Ok(Json(session))
}

pub(crate) async fn workspace_preview_sessions_post(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspacePreviewSessionCreateBody>,
) -> ApiResult<Json<PreviewSessionRecord>> {
    let session = create_server_preview_session(&state, body).await?;
    Ok(Json(session))
}

pub(crate) async fn workspace_preview_sessions_discover_post(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WorkspacePreviewSessionDiscoverBody>,
) -> ApiResult<Json<PreviewSessionRecord>> {
    let Some(target) = discover_preview_target().await? else {
        return Err(AppError::not_found(
            "No reachable local preview target found",
        ));
    };
    let WorkspacePreviewSessionDiscoverBody {
        id,
        directory,
        run_directory,
        agena_session_id,
        command,
        args,
        logs_path,
    } = body;
    let session = state
        .workspace_preview_registry
        .create_server_session(PreviewSessionCreateRequest {
            id,
            directory,
            run_directory,
            agena_session_id,
            command,
            args,
            logs_path,
            target_url: target,
        })
        .await?;
    Ok(Json(session))
}

pub(crate) async fn workspace_preview_sessions_delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkspacePreviewSessionDeleteResponse>> {
    state.workspace_preview_access.revoke(&id).await;
    state.workspace_preview_runtime.delete_by_id(&id).await?;
    Ok(Json(WorkspacePreviewSessionDeleteResponse { ok: true }))
}

pub(crate) async fn workspace_preview_sessions_put(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<WorkspacePreviewSessionUpdateBody>,
) -> ApiResult<Json<PreviewSessionRecord>> {
    let previous_hosts = state
        .workspace_preview_access
        .hosts_for_preview(id.trim())
        .await;
    let updated = state
        .workspace_preview_runtime
        .update_by_id(
            id.trim(),
            PreviewSessionUpdatePatch {
                directory: body.directory,
                run_directory: body.run_directory,
                agena_session_id: body.agena_session_id,
                command: body.command,
                args: body.args,
                logs_path: body.logs_path,
                target_url: body.target_url,
            },
        )
        .await?;
    state
        .workspace_preview_access
        .revoke_hosts(&previous_hosts)
        .await;
    Ok(Json(updated))
}

pub(crate) async fn workspace_preview_sessions_rename_post(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<WorkspacePreviewSessionRenameBody>,
) -> ApiResult<Json<PreviewSessionRecord>> {
    let previous_hosts = state
        .workspace_preview_access
        .hosts_for_preview(id.trim())
        .await;
    let updated = state
        .workspace_preview_runtime
        .rename_by_id(id.trim(), body.id.trim())
        .await?;
    state
        .workspace_preview_access
        .revoke_hosts(&previous_hosts)
        .await;
    Ok(Json(updated))
}

pub(crate) async fn workspace_preview_sessions_start_post(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<Json<PreviewSessionRecord>> {
    let previous_hosts = state
        .workspace_preview_access
        .hosts_for_preview(id.trim())
        .await;
    let updated = state.workspace_preview_runtime.start_by_id(&id).await?;
    state
        .workspace_preview_access
        .revoke_hosts_except_generation(&previous_hosts, &super::access::run_generation(&updated))
        .await;
    Ok(Json(updated))
}

pub(crate) async fn workspace_preview_sessions_stop_post(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<Json<PreviewSessionRecord>> {
    state.workspace_preview_access.revoke(&id).await;
    let updated = state.workspace_preview_runtime.stop_by_id(&id).await?;
    Ok(Json(updated))
}

pub(crate) async fn workspace_preview_session_proxy_root(
    state: State<std::sync::Arc<AppState>>,
    Path(id): Path<String>,
    request: Request<Body>,
) -> ApiResult<Response> {
    proxy_preview_session(state, id, request).await
}

pub(crate) async fn workspace_preview_session_proxy_path(
    state: State<std::sync::Arc<AppState>>,
    Path((id, _path)): Path<(String, String)>,
    request: Request<Body>,
) -> ApiResult<Response> {
    proxy_preview_session(state, id, request).await
}

async fn proxy_preview_session(
    State(state): State<std::sync::Arc<AppState>>,
    id: String,
    request: Request<Body>,
) -> ApiResult<Response> {
    let record = state
        .workspace_preview_registry
        .get_by_id(&id)
        .await
        .ok_or_else(|| AppError::not_found(format!("preview session not found: {id}")))?;
    let proxy_base_path = record.proxy_base_path.clone();
    let target = state.workspace_preview_access.validate_record(&record)?;
    if !state.workspace_preview_runtime.can_proxy_record(&record) {
        return Err(AppError::conflict(
            "preview process stopped or changed; restart the preview",
        ));
    }

    let (parts, body) = request.into_parts();
    let original_uri = parts
        .extensions
        .get::<OriginalUri>()
        .map(|uri| uri.0.clone())
        .unwrap_or_else(|| parts.uri.clone());
    // Axum's Path extractor decodes escapes. Proxy paths must preserve the
    // original representation: a%2Fb and a/b can name different app routes.
    let path = original_uri
        .path()
        .strip_prefix(&proxy_base_path)
        .map(str::to_owned);
    let changed = wait_for_preview_change(state.clone(), record.clone());

    if is_websocket_upgrade_request(&parts.headers, &parts.method) {
        return super::gateway::guard_http_response(
            proxy_preview_session_websocket(state, parts, path, target, original_uri, record),
            changed,
        )
        .await;
    }

    super::gateway::guard_http_response(
        proxy_preview_session_http(parts, body, path, target, original_uri, &proxy_base_path),
        changed,
    )
    .await
}

async fn wait_for_preview_change(state: Arc<AppState>, original: PreviewSessionRecord) {
    let generation = super::access::run_generation(&original);
    loop {
        let record = state
            .workspace_preview_registry
            .get_by_id(&original.id)
            .await;
        if record.is_none_or(|record| {
            super::access::run_generation(&record) != generation
                || !state.workspace_preview_runtime.can_proxy_record(&record)
        }) {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn proxy_preview_session_http(
    parts: axum::http::request::Parts,
    body: Body,
    path: Option<String>,
    target: Url,
    original_uri: Uri,
    proxy_base_path: &str,
) -> ApiResult<Response> {
    let client = preview_proxy_client()?;
    let upstream_url = build_proxy_target_url(&target, path.as_deref(), original_uri.query());
    let mut request = client
        .request(parts.method, upstream_url.clone())
        .headers(filtered_request_headers(&parts.headers));
    // The body is forwarded byte-for-byte. Preserve its known length so
    // servers without chunked-request support (including Python's stdlib)
    // receive JSON/form bodies correctly. Hop-by-hop framing stays filtered.
    if let Some(length) = parts.headers.get(header::CONTENT_LENGTH)
        && !super::gateway::hop_header(&header::CONTENT_LENGTH, &parts.headers)
    {
        request = request.header(header::CONTENT_LENGTH, length);
    }
    let upstream = super::gateway::send_proxy_request(request, body).await?;

    response_from_upstream_for_session(upstream, proxy_base_path, &target).await
}

fn normalize_proxy_base_path(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() || !trimmed.starts_with('/') {
        return String::new();
    }
    if trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    }
}

fn should_rewrite_root_absolute(value: &str, proxy_base_path: &str) -> bool {
    if value.starts_with(proxy_base_path) {
        return false;
    }
    if value.starts_with(PREVIEW_PROXY_BASE_PREFIX) {
        return false;
    }
    true
}

fn rewrite_root_absolute(value: &str, proxy_base_path: &str) -> String {
    format!("{proxy_base_path}{}", value.trim_start_matches('/'))
}

fn rewrite_html_proxy_root_urls(html: &str, proxy_base_path: &str) -> String {
    let base = normalize_proxy_base_path(proxy_base_path);
    if base.is_empty() {
        return html.to_string();
    }

    static DOUBLE_QUOTED: OnceLock<Regex> = OnceLock::new();
    static SINGLE_QUOTED: OnceLock<Regex> = OnceLock::new();
    let double_quoted = DOUBLE_QUOTED.get_or_init(|| {
        Regex::new(r#"(?i)\b(src|href)\s*=\s*"(/[^/][^"]*|/)""#)
            .expect("preview proxy html rewrite regex (double-quoted)")
    });
    let single_quoted = SINGLE_QUOTED.get_or_init(|| {
        Regex::new(r#"(?i)\b(src|href)\s*=\s*'(/[^/][^']*|/)'"#)
            .expect("preview proxy html rewrite regex (single-quoted)")
    });

    let out = double_quoted
        .replace_all(html, |caps: &regex::Captures<'_>| {
            let attr = caps.get(1).map(|m| m.as_str()).unwrap_or("src");
            let value = caps.get(2).map(|m| m.as_str()).unwrap_or("/");
            if should_rewrite_root_absolute(value, &base) {
                format!("{attr}=\"{}\"", rewrite_root_absolute(value, &base))
            } else {
                caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string()
            }
        })
        .into_owned();

    single_quoted
        .replace_all(&out, |caps: &regex::Captures<'_>| {
            let attr = caps.get(1).map(|m| m.as_str()).unwrap_or("src");
            let value = caps.get(2).map(|m| m.as_str()).unwrap_or("/");
            if should_rewrite_root_absolute(value, &base) {
                format!("{attr}='{}'", rewrite_root_absolute(value, &base))
            } else {
                caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string()
            }
        })
        .into_owned()
}

fn is_html_content_type(value: &HeaderValue) -> bool {
    let Ok(raw) = value.to_str() else {
        return false;
    };
    let lower = raw.trim().to_ascii_lowercase();
    lower.starts_with("text/html") || lower.starts_with("application/xhtml+xml")
}

fn is_css_content_type(value: &HeaderValue) -> bool {
    let Ok(raw) = value.to_str() else {
        return false;
    };
    raw.trim().to_ascii_lowercase().starts_with("text/css")
}

fn rewrite_css_proxy_root_urls(css: &str, proxy_base_path: &str) -> String {
    let base = normalize_proxy_base_path(proxy_base_path);
    if base.is_empty() {
        return css.to_string();
    }

    static URL_DQ: OnceLock<Regex> = OnceLock::new();
    static URL_SQ: OnceLock<Regex> = OnceLock::new();
    static URL_BARE: OnceLock<Regex> = OnceLock::new();

    let url_dq = URL_DQ.get_or_init(|| {
        Regex::new(r#"(?i)url\(\s*\"(/[^/][^\"]*|/)\"\s*\)"#)
            .expect("preview proxy css rewrite regex (double-quoted)")
    });
    let url_sq = URL_SQ.get_or_init(|| {
        Regex::new(r#"(?i)url\(\s*'(/[^/][^']*|/)'\s*\)"#)
            .expect("preview proxy css rewrite regex (single-quoted)")
    });
    let url_bare = URL_BARE.get_or_init(|| {
        Regex::new(r#"(?i)url\(\s*(/[^/\s][^)\s]*|/)\s*\)"#)
            .expect("preview proxy css rewrite regex (bare)")
    });

    let out = url_dq
        .replace_all(css, |caps: &regex::Captures<'_>| {
            let value = caps.get(1).map(|m| m.as_str()).unwrap_or("/");
            if should_rewrite_root_absolute(value, &base) {
                format!("url(\"{}\")", rewrite_root_absolute(value, &base))
            } else {
                caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string()
            }
        })
        .into_owned();

    let out = url_sq
        .replace_all(&out, |caps: &regex::Captures<'_>| {
            let value = caps.get(1).map(|m| m.as_str()).unwrap_or("/");
            if should_rewrite_root_absolute(value, &base) {
                format!("url('{}')", rewrite_root_absolute(value, &base))
            } else {
                caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string()
            }
        })
        .into_owned();

    url_bare
        .replace_all(&out, |caps: &regex::Captures<'_>| {
            let value = caps.get(1).map(|m| m.as_str()).unwrap_or("/");
            if should_rewrite_root_absolute(value, &base) {
                format!("url({})", rewrite_root_absolute(value, &base))
            } else {
                caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string()
            }
        })
        .into_owned()
}

fn is_loopback_host(host: Host<&str>) -> bool {
    match host {
        Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ipv4) => ipv4.octets() == [127, 0, 0, 1],
        Host::Ipv6(ipv6) => ipv6.is_loopback(),
    }
}

pub(super) fn is_same_loopback_origin(left: &Url, right: &Url) -> bool {
    if left.scheme() != right.scheme() {
        return false;
    }
    let left_port = left.port_or_known_default();
    let right_port = right.port_or_known_default();
    if left_port.is_none() || right_port.is_none() {
        return false;
    }
    if left_port != right_port {
        return false;
    }
    let Some(left_host) = left.host() else {
        return false;
    };
    let Some(right_host) = right.host() else {
        return false;
    };
    is_loopback_host(left_host) && is_loopback_host(right_host)
}

fn normalize_path_prefix(input: &str) -> String {
    if input.trim().is_empty() {
        return "/".to_string();
    }
    if input.ends_with('/') {
        input.to_string()
    } else {
        format!("{input}/")
    }
}

fn try_rewrite_location_header(raw: &str, proxy_base_path: &str, target: &Url) -> Option<String> {
    let base = normalize_proxy_base_path(proxy_base_path);
    if base.is_empty() {
        return None;
    }

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.starts_with(&base) || trimmed.starts_with(PREVIEW_PROXY_BASE_PREFIX) {
        return None;
    }

    if trimmed.starts_with("//") {
        return None;
    }

    if trimmed.starts_with('/') {
        return Some(rewrite_root_absolute(trimmed, &base));
    }

    let Ok(url) = Url::parse(trimmed) else {
        return None;
    };

    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }

    if !is_same_loopback_origin(&url, target) {
        return None;
    }

    let target_prefix = normalize_path_prefix(target.path());
    let location_path = url.path();
    let relative = location_path
        .strip_prefix(&target_prefix)
        .unwrap_or_else(|| location_path.trim_start_matches('/'))
        .trim_start_matches('/');
    let mut rewritten = format!("{base}{relative}");
    if let Some(query) = url.query() {
        rewritten.push('?');
        rewritten.push_str(query);
    }
    if let Some(fragment) = url.fragment() {
        rewritten.push('#');
        rewritten.push_str(fragment);
    }
    Some(rewritten)
}

async fn response_from_upstream_for_session(
    upstream: reqwest::Response,
    proxy_base_path: &str,
    target: &Url,
) -> ApiResult<Response> {
    let status = StatusCode::from_u16(upstream.status().as_u16())
        .map_err(|err| AppError::internal(format!("invalid upstream status code: {err}")))?;

    let upstream_headers = upstream.headers().clone();
    let can_rewrite = status != StatusCode::PARTIAL_CONTENT
        && !upstream_headers.contains_key(header::CONTENT_RANGE)
        && upstream_headers
            .get_all(header::CONTENT_ENCODING)
            .iter()
            .all(|value| {
                value
                    .to_str()
                    .is_ok_and(|value| value.eq_ignore_ascii_case("identity"))
            });
    let should_rewrite_html = can_rewrite
        && upstream_headers
            .get(header::CONTENT_TYPE)
            .is_some_and(is_html_content_type);
    let should_rewrite_css = can_rewrite
        && upstream_headers
            .get(header::CONTENT_TYPE)
            .is_some_and(is_css_content_type);

    let mut response_headers = HeaderMap::new();
    copy_safe_response_headers(&upstream_headers, &mut response_headers);

    if let Some(value) = upstream_headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| try_rewrite_location_header(value, proxy_base_path, target))
        .and_then(|value| HeaderValue::from_str(&value).ok())
    {
        response_headers.insert(header::LOCATION, value);
    }

    if should_rewrite_html || should_rewrite_css {
        let bytes = read_legacy_document(upstream, Duration::from_secs(15)).await?;
        let text = String::from_utf8_lossy(&bytes);
        let rewritten = if should_rewrite_html {
            rewrite_html_proxy_root_urls(&text, proxy_base_path)
        } else {
            rewrite_css_proxy_root_urls(&text, proxy_base_path)
        };

        response_headers.remove(header::ETAG);
        response_headers.remove(header::LAST_MODIFIED);

        let mut response = Response::new(Body::from(rewritten));
        *response.status_mut() = status;
        *response.headers_mut() = response_headers;
        return Ok(add_preview_frame_headers(response));
    }

    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    Ok(add_preview_frame_headers(response))
}

async fn read_legacy_document(
    mut upstream: reqwest::Response,
    timeout: Duration,
) -> ApiResult<Vec<u8>> {
    tokio::time::timeout(timeout, async move {
        const MAX_REWRITE_BYTES: usize = 8 * 1024 * 1024;
        let mut bytes = Vec::new();
        while let Some(chunk) = upstream.chunk().await.map_err(|err| {
            AppError::bad_gateway(format!("failed to read preview upstream response: {err}"))
        })? {
            if bytes.len().saturating_add(chunk.len()) > MAX_REWRITE_BYTES {
                return Err(AppError::bad_gateway(
                    "legacy preview document exceeds the 8 MiB rewrite limit; use isolated preview access",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| AppError::bad_gateway("legacy preview document body timed out; use isolated preview access"))?
}

async fn proxy_preview_session_websocket(
    state: std::sync::Arc<AppState>,
    mut parts: axum::http::request::Parts,
    path: Option<String>,
    target: Url,
    original_uri: Uri,
    record: PreviewSessionRecord,
) -> ApiResult<Response> {
    let websocket = WebSocketUpgrade::from_request_parts(&mut parts, &state)
        .await
        .map_err(|err| AppError::bad_request(format!("invalid websocket upgrade: {err}")))?;

    let upstream_url = websocket_target_url(&target, path.as_deref(), original_uri.query());
    let mut upstream_request = upstream_url
        .as_str()
        .into_client_request()
        .map_err(|err| AppError::bad_gateway(format!("invalid upstream websocket URL: {err}")))?;
    apply_filtered_websocket_headers(upstream_request.headers_mut(), &parts.headers)?;

    let (upstream_socket, upstream_response) =
        tokio::time::timeout(Duration::from_secs(10), connect_async(upstream_request))
            .await
            .map_err(|_| AppError::bad_gateway("preview websocket connection timed out"))?
            .map_err(|err| {
                AppError::bad_gateway(format!(
                    "failed to connect websocket preview target {upstream_url}: {err}"
                ))
            })?;

    let websocket = if let Some(protocol) = upstream_response
        .headers()
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        websocket.protocols([protocol.to_string()])
    } else {
        websocket
    };

    Ok(add_preview_frame_headers(websocket.on_upgrade(
        move |socket| async move {
            tokio::select! {
                _ = bridge_websocket(socket, upstream_socket) => {},
                _ = wait_for_preview_change(state, record) => {},
            }
        },
    )))
}

pub(super) async fn bridge_websocket(
    downstream: WebSocket,
    upstream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    let (mut downstream_sender, mut downstream_receiver) = downstream.split();
    let (mut upstream_sender, mut upstream_receiver) = upstream.split();

    let downstream_to_upstream = async {
        while let Some(message) = downstream_receiver.next().await {
            let message = match message {
                Ok(message) => message,
                Err(error) => {
                    tracing::debug!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "preview WebSocket downstream receive failed",
                            &error,
                        ),
                        "preview WebSocket downstream disconnected"
                    );
                    break;
                }
            };
            let message = downstream_message_to_upstream(message);
            if let Err(error) = upstream_sender.send(message).await {
                tracing::debug!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "preview WebSocket upstream send failed",
                        &error,
                    ),
                    "preview WebSocket upstream disconnected"
                );
                break;
            }
        }
        if let Err(error) = upstream_sender.close().await {
            tracing::debug!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "preview WebSocket upstream close failed",
                    &error,
                ),
                "preview WebSocket upstream did not close cleanly"
            );
        }
    };

    let upstream_to_downstream = async {
        while let Some(message) = upstream_receiver.next().await {
            let message = match message {
                Ok(message) => message,
                Err(error) => {
                    tracing::debug!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "preview WebSocket upstream receive failed",
                            &error,
                        ),
                        "preview WebSocket upstream disconnected"
                    );
                    break;
                }
            };
            let Some(message) = upstream_message_to_downstream(message) else {
                continue;
            };
            if let Err(error) = downstream_sender.send(message).await {
                tracing::debug!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "preview WebSocket downstream send failed",
                        &error,
                    ),
                    "preview WebSocket downstream disconnected"
                );
                break;
            }
        }
        if let Err(error) = downstream_sender.close().await {
            tracing::debug!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "preview WebSocket downstream close failed",
                    &error,
                ),
                "preview WebSocket downstream did not close cleanly"
            );
        }
    };

    tokio::select! {
        _ = downstream_to_upstream => {}
        _ = upstream_to_downstream => {}
    }
}

fn downstream_message_to_upstream(message: Message) -> TungsteniteMessage {
    match message {
        Message::Text(text) => TungsteniteMessage::Text(text.to_string().into()),
        Message::Binary(bytes) => TungsteniteMessage::Binary(bytes),
        Message::Ping(bytes) => TungsteniteMessage::Ping(bytes),
        Message::Pong(bytes) => TungsteniteMessage::Pong(bytes),
        Message::Close(frame) => {
            TungsteniteMessage::Close(frame.map(|frame| TungsteniteCloseFrame {
                code: frame.code.into(),
                reason: frame.reason.to_string().into(),
            }))
        }
    }
}

fn upstream_message_to_downstream(message: TungsteniteMessage) -> Option<Message> {
    match message {
        TungsteniteMessage::Text(text) => Some(Message::Text(text.to_string().into())),
        TungsteniteMessage::Binary(bytes) => Some(Message::Binary(bytes)),
        TungsteniteMessage::Ping(bytes) => Some(Message::Ping(bytes)),
        TungsteniteMessage::Pong(bytes) => Some(Message::Pong(bytes)),
        TungsteniteMessage::Close(frame) => Some(Message::Close(frame.map(|frame| {
            axum::extract::ws::CloseFrame {
                code: frame.code.into(),
                reason: frame.reason.to_string().into(),
            }
        }))),
        TungsteniteMessage::Frame(_) => None,
    }
}

fn apply_filtered_websocket_headers(to: &mut HeaderMap, from: &HeaderMap) -> ApiResult<()> {
    for (name, value) in from {
        if should_strip_request_header(name) || super::gateway::hop_header(name, from) {
            continue;
        }
        if is_websocket_handshake_header(name) {
            if name == header::SEC_WEBSOCKET_PROTOCOL {
                to.append(name, value.clone());
            }
            continue;
        }
        to.append(name, value.clone());
    }
    Ok(())
}

fn filtered_request_headers(from: &HeaderMap) -> HeaderMap {
    let mut to = HeaderMap::new();
    for (name, value) in from {
        if should_strip_request_header(name) || super::gateway::hop_header(name, from) {
            continue;
        }
        to.append(name, value.clone());
    }
    to
}

async fn create_server_preview_session(
    state: &Arc<AppState>,
    body: WorkspacePreviewSessionCreateBody,
) -> ApiResult<PreviewSessionRecord> {
    let WorkspacePreviewSessionCreateBody {
        id,
        directory,
        run_directory,
        agena_session_id,
        command,
        args,
        logs_path,
        target_url,
    } = body;
    let target = validate_preview_target_url(&target_url)?;
    state
        .workspace_preview_registry
        .create_server_session(PreviewSessionCreateRequest {
            id,
            directory,
            run_directory,
            agena_session_id,
            command,
            args,
            logs_path,
            target_url: target,
        })
        .await
}

async fn discover_preview_target() -> ApiResult<Option<Url>> {
    let client = preview_probe_client()?;
    for candidate in preview_probe_candidates() {
        let Ok(response) = client.get(candidate.clone()).send().await else {
            continue;
        };

        if response.status().is_success() || response.status().is_redirection() {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn preview_probe_candidates() -> Vec<Url> {
    PREVIEW_DISCOVERY_PORTS
        .iter()
        .flat_map(|port| {
            PREVIEW_DISCOVERY_HOSTS
                .iter()
                .map(move |host| format!("http://{host}:{port}/"))
        })
        .filter_map(|candidate| Url::parse(&candidate).ok())
        .collect()
}

fn preview_probe_client() -> ApiResult<reqwest::Client> {
    reqwest::Client::builder()
        // Discovery probes only target this machine. A shell proxy must not
        // intercept them when NO_PROXY is absent.
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_millis(350))
        .timeout(Duration::from_millis(700))
        .build()
        .map_err(|err| AppError::internal_error_with_context("build preview probe client", &err))
}

async fn response_from_upstream(upstream: reqwest::Response) -> ApiResult<Response> {
    let status = StatusCode::from_u16(upstream.status().as_u16())
        .map_err(|err| AppError::internal(format!("invalid upstream status code: {err}")))?;

    let mut response_headers = HeaderMap::new();
    copy_safe_response_headers(upstream.headers(), &mut response_headers);
    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    Ok(add_preview_frame_headers(response))
}

fn preview_proxy_client() -> ApiResult<reqwest::Client> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(3))
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .cloned()
        .map_err(|error| AppError::internal(format!("build preview proxy client: {error}")))
}

fn add_preview_frame_headers(mut response: Response) -> Response {
    // Legacy same-origin endpoints are data bridges. If opened as documents,
    // enforce an opaque sandbox even outside the workbench iframe.
    super::gateway::apply_frame_headers(response.headers_mut(), "'self'", true);
    // A reachable app may correctly require its own cookies or return an
    // application error. Diagnostics must distinguish that from control API
    // authentication failures and a proxy that never reached the upstream.
    response
        .headers_mut()
        .insert("x-agena-preview-upstream", HeaderValue::from_static("1"));
    response
}

fn copy_safe_response_headers(from: &HeaderMap, to: &mut HeaderMap) {
    for (name, value) in from {
        if should_strip_response_header(name) || super::gateway::hop_header(name, from) {
            continue;
        }
        to.append(name, value.clone());
    }
}

fn should_strip_request_header(header_name: &HeaderName) -> bool {
    let key = header_name.as_str().to_ascii_lowercase();
    matches!(
        key.as_str(),
        "accept-encoding"
            | "authorization"
            | "connection"
            | "content-length"
            | "cookie"
            | "host"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn should_strip_response_header(header_name: &HeaderName) -> bool {
    let key = header_name.as_str().to_ascii_lowercase();
    matches!(
        key.as_str(),
        "connection"
            | "content-length"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "set-cookie"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn is_websocket_handshake_header(header_name: &HeaderName) -> bool {
    let key = header_name.as_str().to_ascii_lowercase();
    key.starts_with("sec-websocket-")
}

fn is_websocket_upgrade_request(headers: &HeaderMap, method: &axum::http::Method) -> bool {
    if method != axum::http::Method::GET {
        return false;
    }

    let has_upgrade = headers
        .get(header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    let has_connection_upgrade = headers
        .get(header::CONNECTION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        });

    has_upgrade && has_connection_upgrade
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_proxy_filters_connection_nominated_headers_both_directions() {
        let mut input = HeaderMap::new();
        input.append(
            header::CONNECTION,
            HeaderValue::from_static("keep-alive, x-request-hop"),
        );
        input.append(
            header::CONNECTION,
            HeaderValue::from_static("X-Another-Hop"),
        );
        input.insert("x-request-hop", HeaderValue::from_static("remove"));
        input.insert("x-another-hop", HeaderValue::from_static("remove"));
        input.insert("x-app-header", HeaderValue::from_static("preserve"));
        input.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer workbench"),
        );
        input.insert(header::COOKIE, HeaderValue::from_static("workbench=value"));
        input.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("vite-hmr"),
        );
        let request = filtered_request_headers(&input);
        assert!(!request.contains_key("x-request-hop"));
        assert!(!request.contains_key("x-another-hop"));
        assert!(!request.contains_key(header::AUTHORIZATION));
        assert!(!request.contains_key(header::COOKIE));
        assert_eq!(request["x-app-header"], "preserve");
        let mut websocket = HeaderMap::new();
        apply_filtered_websocket_headers(&mut websocket, &input).unwrap();
        assert!(!websocket.contains_key("x-request-hop"));
        assert!(!websocket.contains_key("x-another-hop"));
        assert_eq!(websocket[header::SEC_WEBSOCKET_PROTOCOL], "vite-hmr");
        let mut response = HeaderMap::new();
        copy_safe_response_headers(&input, &mut response);
        assert!(!response.contains_key("x-request-hop"));
        assert!(!response.contains_key("x-another-hop"));
        assert_eq!(response["x-app-header"], "preserve");
    }

    #[tokio::test]
    async fn legacy_proxy_marks_reachable_errors_and_preserves_encoded_or_partial_bodies() {
        async fn upstream(request: Request<Body>) -> Response {
            match request.uri().path() {
                "/encoded" => Response::builder()
                    .header(header::CONTENT_TYPE, "text/html")
                    .header(header::CONTENT_ENCODING, "x-test-encoding")
                    .body(Body::from(vec![0, 255, 128, 65]))
                    .unwrap(),
                "/partial" => Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(header::CONTENT_TYPE, "text/html")
                    .header(header::CONTENT_RANGE, "bytes 0-12/100")
                    .body(Body::from("<a href=\"/x\">"))
                    .unwrap(),
                _ => Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header("x-agena-preview-upstream", "untrusted-upstream-value")
                    .body(Body::from("app login required"))
                    .unwrap(),
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, axum::Router::new().fallback(upstream))
                .await
                .unwrap();
        });
        for (path, status, expected) in [
            (
                "error",
                StatusCode::UNAUTHORIZED,
                b"app login required".as_slice(),
            ),
            ("encoded", StatusCode::OK, &[0, 255, 128, 65][..]),
            (
                "partial",
                StatusCode::PARTIAL_CONTENT,
                b"<a href=\"/x\">".as_slice(),
            ),
        ] {
            let response = preview_proxy_client()
                .unwrap()
                .get(target.join(path).unwrap())
                .send()
                .await
                .unwrap();
            let response = response_from_upstream_for_session(
                response,
                "/api/v1/workbench/preview/s/demo/",
                &target,
            )
            .await
            .unwrap();
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()["x-agena-preview-upstream"], "1");
            if path == "encoded" {
                assert_eq!(
                    response.headers()[header::CONTENT_ENCODING],
                    "x-test-encoding"
                );
            }
            if path == "partial" {
                assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 0-12/100");
            }
            assert_eq!(
                &axum::body::to_bytes(response.into_body(), 4096)
                    .await
                    .unwrap()[..],
                expected
            );
        }
        server.abort();
    }

    #[tokio::test]
    async fn legacy_html_rewrite_has_a_body_deadline_even_after_headers_arrive() {
        async fn upstream() -> Response {
            let body = futures_util::stream::once(async {
                Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"<html>"))
            })
            .chain(futures_util::stream::pending());
            Response::builder()
                .header(header::CONTENT_TYPE, "text/html")
                .body(Body::from_stream(body))
                .unwrap()
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, axum::Router::new().fallback(upstream))
                .await
                .unwrap();
        });
        let upstream = preview_proxy_client()
            .unwrap()
            .get(target)
            .send()
            .await
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            read_legacy_document(upstream, Duration::from_millis(25)),
        )
        .await
        .expect("the legacy rewrite must not wait indefinitely for EOF");
        assert!(result.is_err());
        server.abort();
    }
}
