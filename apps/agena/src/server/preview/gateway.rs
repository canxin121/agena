//! Root-path reverse proxy on capability-bound, isolated preview hostnames.
use futures_util::StreamExt;
use std::{
    future::Future,
    sync::{Arc, OnceLock},
    time::Duration,
};

use axum::{
    Json,
    body::Body,
    extract::{FromRequestParts, Path, State, ws::WebSocketUpgrade},
    http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};
use url::Url;

use super::{
    access::{PreviewAccessRequest, PreviewAccessResponse, PreviewGrant, run_generation},
    registry::build_proxy_target_url,
};
use crate::{ApiResult, AppError, AppState};

pub(crate) async fn create_access(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<PreviewAccessRequest>,
) -> ApiResult<Json<PreviewAccessResponse>> {
    let record = state
        .workspace_preview_registry
        .get_by_id(&id)
        .await
        .ok_or_else(|| AppError::not_found("preview session not found"))?;
    // An authenticated user may deliberately attach an external dev server.
    // Agent browser access separately requires a child owned by this runtime.
    if !state.workspace_preview_runtime.can_proxy_record(&record) {
        return Err(AppError::conflict(
            "preview process is not owned by this server; restart the preview",
        ));
    }
    let backend_origin = backend_origin_for_access(&input.backend_origin, &headers)?;
    let grant = state
        .workspace_preview_access
        .issue(&record, &backend_origin, &input.workbench_origin)
        .await?;
    Ok(Json(grant))
}

fn backend_origin_for_access(hint: &str, headers: &HeaderMap) -> ApiResult<String> {
    let hint = super::access::parse_origin(hint)?;
    let Some(authority) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(hint.origin().ascii_serialization());
    };
    // Vite changes Host to its backend target; SSH forwarding preserves the
    // client-facing authority. Both are more accurate than the SPA's origin.
    let mut actual = super::access::parse_origin(&format!("http://{authority}"))?;
    let forwarded = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .filter(|value| matches!(*value, "http" | "https"));
    let same_authority = actual.host_str() == hint.host_str() && actual.port() == hint.port();
    let scheme = forwarded.unwrap_or(if same_authority {
        hint.scheme()
    } else {
        "http"
    });
    actual
        .set_scheme(scheme)
        .map_err(|_| AppError::bad_request("invalid backend scheme"))?;
    Ok(actual.origin().ascii_serialization())
}

fn request_host(request: &Request<Body>) -> Option<String> {
    let authority = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            request
                .uri()
                .authority()
                .map(|authority| authority.as_str())
        })?;
    let url = Url::parse(&format!("http://{authority}")).ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    url.host_str()
        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
}

/// Run outside the main auth layer: only a valid preview host can reach an app;
/// preview hosts (including expired ones) can never reach the control router.
pub(crate) async fn dispatch(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if let Some(host) = request_host(&request)
        && state.workspace_preview_access.is_preview_host(&host)
    {
        return match proxy(state, &host, request).await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
    }
    // This also protects password-disabled local workbenches from scripts in a
    // preview. Main APIs remain header-authenticated; no cookie auth is added.
    if let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        && (state.workspace_preview_access.is_preview_origin(origin) || origin == "null")
    {
        return AppError::forbidden("preview documents cannot access the Agena control origin")
            .into_response();
    }
    next.run(request).await
}

async fn proxy(state: Arc<AppState>, host: &str, request: Request<Body>) -> ApiResult<Response> {
    let grant = state
        .workspace_preview_access
        .lookup(host)
        .await
        .ok_or_else(|| {
            AppError::forbidden("preview access expired or was revoked; reopen the preview")
        })?;
    let record = state
        .workspace_preview_registry
        .get_by_id(&grant.preview_id)
        .await
        .ok_or_else(|| AppError::not_found("preview session no longer exists"))?;
    if run_generation(&record) != grant.generation
        || !state.workspace_preview_runtime.can_proxy_record(&record)
    {
        state.workspace_preview_access.revoke_host(host).await;
        return Err(AppError::conflict(
            "preview run changed; reopen the preview",
        ));
    }
    let target = state.workspace_preview_access.validate_record(&record)?;
    if request.method() == Method::GET
        && request
            .headers()
            .get(header::UPGRADE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
    {
        return guard_http_response(
            proxy_websocket(state.clone(), request, target, grant.clone()),
            wait_for_revocation(state, grant),
        )
        .await;
    }
    guard_http_response(
        proxy_http(request, &target, &grant),
        wait_for_revocation(state, grant.clone()),
    )
    .await
}

/// The same lease covers waiting for response headers and reading the body.
/// Dropping a revoked response also drops its upstream stream; SSE and long
/// downloads must not outlive the capability that authorized the request.
pub(super) async fn guard_http_response(
    response: impl Future<Output = ApiResult<Response>>,
    revoked: impl Future<Output = ()> + Send + 'static,
) -> ApiResult<Response> {
    let mut revoked = Box::pin(revoked);
    let response = tokio::select! {
        biased;
        _ = &mut revoked => return Err(AppError::forbidden("preview access expired or was revoked; reopen the preview")),
        response = response => response?,
    };
    let (parts, body) = response.into_parts();
    Ok(Response::from_parts(
        parts,
        Body::from_stream(body.into_data_stream().take_until(revoked)),
    ))
}

async fn wait_for_revocation(state: Arc<AppState>, grant: PreviewGrant) {
    loop {
        let host = grant.origin.host_str().unwrap_or_default();
        if state.workspace_preview_access.lookup(host).await.is_none() {
            return;
        }
        let record = state
            .workspace_preview_registry
            .get_by_id(&grant.preview_id)
            .await;
        if record.is_none_or(|record| {
            run_generation(&record) != grant.generation
                || !state.workspace_preview_runtime.can_proxy_record(&record)
        }) {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn client() -> ApiResult<&'static reqwest::Client> {
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
        .map_err(|error| AppError::internal(format!("build preview client: {error}")))
}

pub(super) async fn send_proxy_request(
    request: reqwest::RequestBuilder,
    body: Body,
) -> ApiResult<reqwest::Response> {
    send_proxy_request_with_idle_timeout(request, body, Duration::from_secs(15)).await
}

async fn send_proxy_request_with_idle_timeout(
    request: reqwest::RequestBuilder,
    body: Body,
    timeout: Duration,
) -> ApiResult<reqwest::Response> {
    let (progress, mut progress_rx) = tokio::sync::watch::channel(tokio::time::Instant::now());
    let body = futures_util::stream::unfold(
        (body.into_data_stream(), progress),
        |(mut stream, progress)| async move {
            let chunk = stream.next().await?;
            progress.send_replace(tokio::time::Instant::now());
            Some((chunk, (stream, progress)))
        },
    );
    let response = request.body(reqwest::Body::wrap_stream(body)).send();
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(response, deadline);
    let mut uploading = true;
    loop {
        tokio::select! {
            biased;
            response = &mut response => return response.map_err(|error| {
                AppError::bad_gateway(format!("preview upstream unavailable: {error}"))
            }),
            progress = progress_rx.changed(), if uploading => {
                // Closing the sender means the request body finished (or the
                // transport stopped consuming it). Start the response-header
                // budget then, rather than charging a long upload against it.
                let activity = if progress.is_ok() {
                    *progress_rx.borrow_and_update()
                } else {
                    uploading = false;
                    tokio::time::Instant::now()
                };
                deadline.as_mut().reset(activity + timeout);
            },
            _ = &mut deadline => return Err(AppError::bad_gateway(
                "preview upstream upload or response headers made no progress before the timeout",
            )),
        }
    }
}

pub(super) fn hop_header(name: &HeaderName, source: &HeaderMap) -> bool {
    let key = name.as_str();
    matches!(
        key,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    ) || source
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|v| v.trim().eq_ignore_ascii_case(key))
}

fn request_headers(
    source: &HeaderMap,
    target: &Url,
    grant: &PreviewGrant,
    websocket: bool,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in source {
        if hop_header(name, source)
            || name == header::HOST
            || name == header::ACCEPT_ENCODING
            || (websocket
                && matches!(
                    name.as_str(),
                    "sec-websocket-key"
                        | "sec-websocket-version"
                        | "sec-websocket-accept"
                        | "sec-websocket-extensions"
                ))
        {
            continue;
        }
        // Origin checks inside dev servers should see their own origin. Do not
        // rewrite foreign origins, which may be part of the app's CSRF checks.
        if name == header::ORIGIN
            && value.to_str().ok() == Some(grant.origin.origin().ascii_serialization().as_str())
        {
            if let Ok(value) = HeaderValue::from_str(&target.origin().ascii_serialization()) {
                headers.append(name.clone(), value);
            }
        } else {
            headers.append(name.clone(), value.clone());
        }
    }
    headers
}

fn isolate_cookie(value: &str, target: &Url) -> String {
    let mut parts = value.split(';');
    // The first part is a cookie pair, not an attribute. A cookie named
    // `domain` or `path` is valid and must not disappear during isolation.
    let mut result = vec![parts.next().unwrap_or_default().trim().to_owned()];
    let base = target.path().trim_end_matches('/');
    for part in parts.map(str::trim) {
        let (name, value) = part.split_once('=').unwrap_or((part, ""));
        if name.trim().eq_ignore_ascii_case("domain") {
            continue;
        }
        if name.trim().eq_ignore_ascii_case("path") && !base.is_empty() {
            let path = value.trim();
            if path == base
                || path == format!("{base}/")
                || base.strip_prefix(path).is_some_and(|rest| {
                    path.starts_with('/') && (path.ends_with('/') || rest.starts_with('/'))
                })
            {
                result.push("Path=/".to_owned());
                continue;
            }
            if let Some(relative) = path.strip_prefix(&format!("{base}/")) {
                result.push(format!("Path=/{relative}"));
                continue;
            }
        }
        result.push(part.to_owned());
    }
    result.join("; ")
}

pub(crate) fn apply_frame_headers(headers: &mut HeaderMap, ancestors: &str, opaque_sandbox: bool) {
    let policies: Vec<String> = headers
        .get_all(header::CONTENT_SECURITY_POLICY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .map(str::trim)
                .filter(|directive| {
                    !directive.is_empty()
                        && !directive
                            .split_whitespace()
                            .next()
                            .is_some_and(|name| name.eq_ignore_ascii_case("frame-ancestors"))
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .collect();
    headers.remove(header::X_FRAME_OPTIONS);
    headers.remove(header::CONTENT_SECURITY_POLICY);
    for policy in policies {
        if !policy.is_empty()
            && let Ok(value) = HeaderValue::from_str(&policy)
        {
            headers.append(header::CONTENT_SECURITY_POLICY, value);
        }
    }
    let mut policy = format!("frame-ancestors {ancestors}");
    if opaque_sandbox {
        policy.push_str("; sandbox allow-scripts allow-forms allow-downloads");
    }
    if let Ok(value) = HeaderValue::from_str(&policy) {
        headers.append(header::CONTENT_SECURITY_POLICY, value);
    }
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
}

fn apply_browser_policy(headers: &mut HeaderMap, grant: &PreviewGrant) {
    if !grant.browser_only {
        return;
    }
    let mut websocket = grant.origin.clone();
    let _ = websocket.set_scheme(if grant.origin.scheme() == "https" {
        "wss"
    } else {
        "ws"
    });
    // CDP Fetch interception does not cover every WebSocket handshake. Enforce
    // the managed-preview boundary in the browser too, including child contexts
    // and popups that could otherwise create an unattached CDP target.
    let policy = format!(
        "connect-src 'self' {}; frame-src 'self'; worker-src 'self' blob:; sandbox allow-scripts allow-forms allow-same-origin allow-downloads",
        websocket.origin().ascii_serialization()
    );
    if let Ok(value) = HeaderValue::from_str(&policy) {
        headers.append(header::CONTENT_SECURITY_POLICY, value);
    }
}

fn rewrite_location(
    raw: &str,
    request_url: &Url,
    target: &Url,
    preview_origin: &Url,
) -> Option<String> {
    // Relative redirects are relative to the requested resource, not the app
    // root. Strip a configured upstream base path once when mapping it back.
    let resolved = request_url.join(raw).ok()?;
    if resolved.origin() != target.origin()
        && !super::routes::is_same_loopback_origin(&resolved, target)
    {
        return None;
    }
    let base = target.path().trim_end_matches('/');
    let path = if base.is_empty() {
        resolved.path()
    } else if resolved.path() == base {
        "/"
    } else {
        resolved.path().strip_prefix(&format!("{base}/"))?
    };
    let mut result = preview_origin.clone();
    result.set_path(path);
    result.set_query(resolved.query());
    result.set_fragment(resolved.fragment());
    Some(result.to_string())
}

async fn proxy_http(
    request: Request<Body>,
    target: &Url,
    grant: &PreviewGrant,
) -> ApiResult<Response> {
    let (parts, body) = request.into_parts();
    let upstream_url = build_proxy_target_url(
        target,
        Some(parts.uri.path().trim_start_matches('/')),
        parts.uri.query(),
    );
    let request = client()?
        .request(parts.method, upstream_url)
        .headers(request_headers(&parts.headers, target, grant, false));
    // Uploads reset an inactivity deadline; response bodies, including SSE,
    // remain streamed without a blanket response-duration limit.
    let upstream = send_proxy_request(request, body).await?;
    let status = StatusCode::from_u16(upstream.status().as_u16())
        .map_err(|error| AppError::internal(error.to_string()))?;
    let source = upstream.headers();
    let mut headers = HeaderMap::new();
    for (name, value) in source {
        if hop_header(name, source) {
            continue;
        }
        if name == header::SET_COOKIE {
            if let Ok(raw) = value.to_str()
                && let Ok(value) = HeaderValue::from_str(&isolate_cookie(raw, target))
            {
                headers.append(name.clone(), value);
            }
        } else if name == header::LOCATION {
            let rewritten = value
                .to_str()
                .ok()
                .and_then(|raw| rewrite_location(raw, upstream.url(), target, &grant.origin));
            if let Some(value) = rewritten.and_then(|value| HeaderValue::from_str(&value).ok()) {
                headers.append(name.clone(), value);
            } else {
                headers.append(name.clone(), value.clone());
            }
        } else {
            headers.append(name.clone(), value.clone());
        }
    }
    apply_frame_headers(
        &mut headers,
        &format!("'self' {}", grant.workbench_origin),
        false,
    );
    apply_browser_policy(&mut headers, grant);
    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}

async fn proxy_websocket(
    state: Arc<AppState>,
    request: Request<Body>,
    target: Url,
    grant: PreviewGrant,
) -> ApiResult<Response> {
    let (mut parts, _) = request.into_parts();
    let mut upstream = build_proxy_target_url(
        &target,
        Some(parts.uri.path().trim_start_matches('/')),
        parts.uri.query(),
    );
    upstream
        .set_scheme(if target.scheme() == "https" {
            "wss"
        } else {
            "ws"
        })
        .map_err(|_| AppError::bad_request("invalid preview websocket scheme"))?;
    let mut upstream_request = upstream
        .as_str()
        .into_client_request()
        .map_err(|error| AppError::bad_request(error.to_string()))?;
    for (name, value) in &request_headers(&parts.headers, &target, &grant, true) {
        upstream_request
            .headers_mut()
            .append(name.clone(), value.clone());
    }
    let websocket = WebSocketUpgrade::from_request_parts(&mut parts, &state)
        .await
        .map_err(|error| {
            AppError::bad_request(format!("invalid preview websocket upgrade: {error}"))
        })?;
    let (upstream_socket, upstream_response) =
        tokio::time::timeout(Duration::from_secs(10), connect_async(upstream_request))
            .await
            .map_err(|_| AppError::bad_gateway("preview websocket connection timed out"))?
            .map_err(|error| {
                AppError::bad_gateway(format!("preview websocket unavailable: {error}"))
            })?;
    let websocket = if let Some(protocol) = upstream_response
        .headers()
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
    {
        websocket.protocols([protocol.to_string()])
    } else {
        websocket
    };
    let ancestors = format!("'self' {}", grant.workbench_origin);
    let cookie_target = target.clone();
    let mut response = websocket
        .on_upgrade(move |socket| async move {
            let bridge = super::routes::bridge_websocket(socket, upstream_socket);
            let revoked = wait_for_revocation(state, grant);
            tokio::select! { _ = bridge => {}, _ = revoked => {} }
        })
        .into_response();
    for cookie in upstream_response.headers().get_all(header::SET_COOKIE) {
        if let Ok(raw) = cookie.to_str()
            && let Ok(value) = HeaderValue::from_str(&isolate_cookie(raw, &cookie_target))
        {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    apply_frame_headers(response.headers_mut(), &ancestors, false);
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> PreviewGrant {
        PreviewGrant {
            preview_id: "test".into(),
            generation: "run1".into(),
            origin: Url::parse("http://p-test.localhost:8080/").unwrap(),
            workbench_origin: "http://localhost:8080".into(),
            expires_at: i64::MAX,
            browser_only: false,
        }
    }

    #[test]
    fn access_uses_backend_authority_behind_vite_and_preserves_forwarded_ports() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:3210"));
        assert_eq!(
            backend_origin_for_access("http://localhost:5173", &headers).unwrap(),
            "http://127.0.0.1:3210"
        );
        headers.insert(header::HOST, HeaderValue::from_static("localhost:43210"));
        assert_eq!(
            backend_origin_for_access("http://localhost:43210", &headers).unwrap(),
            "http://localhost:43210"
        );
        headers.insert(header::HOST, HeaderValue::from_static("agena.example.com"));
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert_eq!(
            backend_origin_for_access("https://agena.example.com", &headers).unwrap(),
            "https://agena.example.com"
        );
    }

    #[test]
    fn application_credentials_are_preserved_on_isolated_origin() {
        let mut source = HeaderMap::new();
        source.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer app-token"),
        );
        source.insert(header::COOKIE, HeaderValue::from_static("session=app"));
        source.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://p-test.localhost:8080"),
        );
        source.insert(
            header::CONNECTION,
            HeaderValue::from_static("keep-alive, x-private-hop"),
        );
        source.insert("x-private-hop", HeaderValue::from_static("drop"));
        let result = request_headers(
            &source,
            &Url::parse("http://127.0.0.1:5173/").unwrap(),
            &grant(),
            false,
        );
        assert_eq!(result[header::AUTHORIZATION], "Bearer app-token");
        assert_eq!(result[header::COOKIE], "session=app");
        assert_eq!(result[header::ORIGIN], "http://127.0.0.1:5173");
        assert!(!result.contains_key("x-private-hop"));
    }

    #[test]
    fn frame_policy_preserves_upstream_restrictions_and_isolates_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; object-src 'none'; frame-ancestors 'none'",
            ),
        );
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        apply_frame_headers(&mut headers, "'self' https://ui.example.com", false);
        let policies = headers
            .get_all(header::CONTENT_SECURITY_POLICY)
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect::<Vec<_>>()
            .join("; ");
        assert!(policies.contains("default-src 'self'"));
        assert!(policies.contains("object-src 'none'"));
        assert!(policies.contains("frame-ancestors 'self' https://ui.example.com"));
        assert!(!policies.contains("frame-ancestors 'none'"));
        assert!(!headers.contains_key(header::X_FRAME_OPTIONS));
        assert_eq!(
            isolate_cookie(
                "sid=abc; Domain=127.0.0.1; Path=/; HttpOnly; SameSite=Lax",
                &Url::parse("http://127.0.0.1:5173/").unwrap(),
            ),
            "sid=abc; Path=/; HttpOnly; SameSite=Lax"
        );
    }

    #[test]
    fn cookie_names_and_upstream_mount_paths_survive_proxying() {
        let target = Url::parse("http://127.0.0.1:5173/app/").unwrap();
        for (input, expected) in [
            (
                "domain=app-value; Domain=localhost; Path=/app; HttpOnly",
                "domain=app-value; Path=/; HttpOnly",
            ),
            (
                "path=value; Path=/app/account; SameSite=Lax",
                "path=value; Path=/account; SameSite=Lax",
            ),
            ("sid=value; Path=/app/", "sid=value; Path=/"),
            (
                "sid=value; Path=/application",
                "sid=value; Path=/application",
            ),
        ] {
            assert_eq!(isolate_cookie(input, &target), expected);
        }
        let target = Url::parse("http://127.0.0.1:5173/app/admin/").unwrap();
        assert_eq!(
            isolate_cookie("sid=value; Path=/app", &target),
            "sid=value; Path=/"
        );
    }

    #[tokio::test]
    async fn revoked_http_requests_do_not_keep_waiting_for_upstream_headers() {
        let (revoke, revoked) = tokio::sync::oneshot::channel::<()>();
        revoke.send(()).unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            guard_http_response(std::future::pending(), async {
                let _ = revoked.await;
            }),
        )
        .await
        .expect("revocation must interrupt response-header waiting");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn proxy_upload_timeout_tracks_progress_then_bounds_response_header_waiting() {
        async fn upstream(request: Request<Body>) -> Response {
            let path = request.uri().path().to_owned();
            let bytes = axum::body::to_bytes(request.into_body(), 4096)
                .await
                .unwrap();
            if path == "/headers" {
                return std::future::pending().await;
            }
            Response::new(Body::from(bytes))
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, axum::Router::new().fallback(upstream))
                .await
                .unwrap();
        });
        let chunks = futures_util::stream::unfold(0, |chunk| async move {
            if chunk == 8 {
                None
            } else {
                tokio::time::sleep(Duration::from_millis(40)).await;
                Some((
                    Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"data")),
                    chunk + 1,
                ))
            }
        });
        // The upload takes over 320 ms in total but never goes idle for 150 ms.
        let response = send_proxy_request_with_idle_timeout(
            client().unwrap().post(target.clone()),
            Body::from_stream(chunks),
            Duration::from_millis(150),
        )
        .await
        .expect("a progressing upload must not hit a total response timeout");
        assert_eq!(response.bytes().await.unwrap().len(), 32);
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            send_proxy_request_with_idle_timeout(
                client().unwrap().post(target.join("headers").unwrap()),
                Body::from("complete request"),
                Duration::from_millis(100),
            ),
        )
        .await
        .expect("response headers still have a deadline after upload completion");
        assert!(result.is_err());
        server.abort();
    }

    #[test]
    fn legacy_document_policy_uses_opaque_sandbox() {
        let mut headers = HeaderMap::new();
        apply_frame_headers(&mut headers, "'self'", true);
        let policy = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(policy.contains("sandbox allow-scripts"));
        assert!(!policy.contains("allow-same-origin"));
    }

    #[test]
    fn redirects_resolve_against_request_and_strip_upstream_prefix_once() {
        let preview = grant().origin;
        let target = Url::parse("http://127.0.0.1:5173/").unwrap();
        let request = target.join("account/login").unwrap();
        assert_eq!(
            rewrite_location("verify?step=2#code", &request, &target, &preview).unwrap(),
            "http://p-test.localhost:8080/account/verify?step=2#code"
        );
        assert_eq!(
            rewrite_location("/dashboard", &request, &target, &preview).unwrap(),
            "http://p-test.localhost:8080/dashboard"
        );
        for location in [
            "http://localhost:5173/dashboard",
            "http://[::1]:5173/dashboard",
        ] {
            assert_eq!(
                rewrite_location(location, &request, &target, &preview).unwrap(),
                "http://p-test.localhost:8080/dashboard"
            );
        }
        for location in [
            "https://localhost:5173/dashboard",
            "http://localhost:5174/dashboard",
            "http://app.localhost:5173/dashboard",
            "http://127.0.0.2:5173/dashboard",
            "http://external.example:5173/dashboard",
        ] {
            assert!(
                rewrite_location(location, &request, &target, &preview).is_none(),
                "{location}"
            );
        }
        let target = Url::parse("http://127.0.0.1:5173/app/").unwrap();
        let request = target.join("account/login").unwrap();
        assert_eq!(
            rewrite_location("verify", &request, &target, &preview).unwrap(),
            "http://p-test.localhost:8080/account/verify"
        );
        assert_eq!(
            rewrite_location("/app/home", &request, &target, &preview).unwrap(),
            "http://p-test.localhost:8080/home"
        );
        assert_eq!(
            rewrite_location("/app", &request, &target, &preview).unwrap(),
            "http://p-test.localhost:8080/"
        );
        assert!(
            rewrite_location("https://login.example.com/", &request, &target, &preview).is_none()
        );
        assert!(rewrite_location("/another-app/", &request, &target, &preview).is_none());
    }

    #[test]
    fn browser_policy_restricts_connections_children_and_popups_only_for_agents() {
        let mut headers = HeaderMap::new();
        let mut grant = grant();
        apply_browser_policy(&mut headers, &grant);
        assert!(!headers.contains_key(header::CONTENT_SECURITY_POLICY));
        grant.browser_only = true;
        apply_browser_policy(&mut headers, &grant);
        let policy = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(policy.contains("connect-src 'self' ws://p-test.localhost:8080"));
        assert!(policy.contains("frame-src 'self'"));
        assert!(policy.contains("worker-src 'self' blob:"));
        assert!(
            policy.contains("sandbox allow-scripts allow-forms allow-same-origin allow-downloads")
        );
        assert!(!policy.contains("allow-popups"));
    }

    #[tokio::test]
    async fn real_http_proxy_preserves_app_credentials_paths_redirects_and_streaming() {
        use futures_util::StreamExt;
        async fn upstream(request: Request<Body>) -> Response {
            if request.uri().path() == "/account/login" {
                return Response::builder()
                    .status(302)
                    .header(header::LOCATION, "verify?step=2")
                    .body(Body::empty())
                    .unwrap();
            }
            if request.uri().path() == "/events" {
                let events = futures_util::stream::once(async {
                    Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"data: ready\n\n"))
                })
                .chain(futures_util::stream::pending());
                return Response::builder()
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from_stream(events))
                    .unwrap();
            }
            let (parts, body) = request.into_parts();
            let body = axum::body::to_bytes(body, 1024).await.unwrap();
            let observed = serde_json::json!({
                "path": parts.uri.path(), "query": parts.uri.query(), "method": parts.method.as_str(),
                "authorization": parts.headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()),
                "cookie": parts.headers.get(header::COOKIE).and_then(|v| v.to_str().ok()),
                "origin": parts.headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()),
                "body": String::from_utf8_lossy(&body),
            });
            Response::builder()
                .header(header::CONTENT_TYPE, "application/json")
                .header(
                    header::SET_COOKIE,
                    "sid=app; Domain=localhost; Path=/; HttpOnly",
                )
                .header(
                    header::CONTENT_SECURITY_POLICY,
                    "object-src 'none'; frame-ancestors 'none'",
                )
                .body(Body::from(observed.to_string()))
                .unwrap()
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, axum::Router::new().fallback(upstream))
                .await
                .unwrap();
        });
        let grant = grant();
        let response = proxy_http(
            Request::builder()
                .method(Method::POST)
                .uri("/api/items?filter=a%20b")
                .header(header::AUTHORIZATION, "Bearer app-only")
                .header(header::COOKIE, "sid=app")
                .header(header::ORIGIN, grant.origin.origin().ascii_serialization())
                .body(Body::from("item=hello world"))
                .unwrap(),
            &target,
            &grant,
        )
        .await
        .unwrap();
        assert_eq!(
            response.headers()[header::SET_COOKIE],
            "sid=app; Path=/; HttpOnly"
        );
        assert!(
            response
                .headers()
                .get_all(header::CONTENT_SECURITY_POLICY)
                .iter()
                .any(|value| value.to_str().unwrap().contains("object-src 'none'"))
        );
        let observed: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(observed["path"], "/api/items");
        assert_eq!(observed["query"], "filter=a%20b");
        assert_eq!(observed["authorization"], "Bearer app-only");
        assert_eq!(observed["cookie"], "sid=app");
        assert_eq!(observed["origin"], target.origin().ascii_serialization());
        assert_eq!(observed["body"], "item=hello world");
        let redirect = proxy_http(
            Request::builder()
                .uri("/account/login")
                .body(Body::empty())
                .unwrap(),
            &target,
            &grant,
        )
        .await
        .unwrap();
        assert_eq!(redirect.status(), StatusCode::FOUND);
        assert_eq!(
            redirect.headers()[header::LOCATION],
            "http://p-test.localhost:8080/account/verify?step=2"
        );
        let (revoke, revoked) = tokio::sync::oneshot::channel::<()>();
        let events = guard_http_response(
            proxy_http(
                Request::builder()
                    .uri("/events")
                    .body(Body::empty())
                    .unwrap(),
                &target,
                &grant,
            ),
            async {
                let _ = revoked.await;
            },
        )
        .await
        .unwrap();
        let mut stream = events.into_body().into_data_stream();
        let first = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(&first[..], b"data: ready\n\n");
        revoke.send(()).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), stream.next())
                .await
                .expect("revocation must terminate an already-streaming response")
                .is_none()
        );
        server.abort();
    }
}
