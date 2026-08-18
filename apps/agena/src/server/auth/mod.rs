use std::sync::Arc;
use std::time::Duration;

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, Method, StatusCode, header},
    middleware,
    response::IntoResponse,
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

const UI_SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const UI_SESSION_CLEANUP_INTERVAL: Duration = Duration::from_secs(10 * 60);
const LOGIN_FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);
const LOGIN_FAILURE_LIMIT: u32 = 8;
const LOGIN_LOCKOUT_DURATION: Duration = Duration::from_secs(15 * 60);
const GLOBAL_LOGIN_FAILURE_LIMIT: u32 = 64;
const GLOBAL_LOGIN_LOCKOUT_DURATION: Duration = Duration::from_secs(5 * 60);
const GLOBAL_LOGIN_ATTEMPT_KEY: &str = "__global__";
const MAX_LOGIN_PASSWORD_BYTES: usize = 4096;

#[derive(Clone)]
pub(crate) enum UiAuth {
    Disabled,
    Enabled(Arc<UiAuthInner>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OAuthPasswordError {
    NotConfigured,
    Invalid,
    Locked(i64),
}

pub(crate) struct UiAuthInner {
    password_phc: String,
    sessions: DashMap<String, SessionRecord>,
    login_attempts: DashMap<String, LoginAttemptRecord>,
}

#[derive(Clone, Debug)]
struct SessionRecord {
    last_seen: OffsetDateTime,
}

#[derive(Clone, Debug)]
struct LoginAttemptRecord {
    window_started: OffsetDateTime,
    failures: u32,
    locked_until: Option<OffsetDateTime>,
}

#[derive(Debug, Serialize)]
struct AuthStatusOk {
    authenticated: bool,
    #[serde(skip_serializing_if = "is_false")]
    disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

#[derive(Debug, Serialize)]
struct AuthStatusLocked {
    authenticated: bool,
    locked: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateSessionBody {
    password: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthErrorBody {
    error: String,
    #[serde(skip_serializing_if = "is_false")]
    locked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_seconds: Option<i64>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn normalize_password(candidate: Option<&str>) -> String {
    candidate.unwrap_or("").trim().to_string()
}

fn normalize_client_key_value(raw: &str) -> Option<String> {
    let mut v = raw.trim().trim_matches('"').trim().to_string();
    if v.starts_with('[') && v.ends_with(']') && v.len() > 2 {
        v = v.trim_start_matches('[').trim_end_matches(']').to_string();
    }
    if v.is_empty() {
        return None;
    }
    if v.len() > 128 {
        v.truncate(128);
    }
    Some(v)
}

fn parse_forwarded_for(raw: &str) -> Option<String> {
    // A trusted reverse proxy appends its directly observed client address to
    // the right side. Reading the right-most element prevents a caller from
    // selecting the rate-limit bucket by prepending a spoofed value.
    for entry in raw.split(',').rev() {
        for kv in entry.split(';') {
            let part = kv.trim();
            let Some((name, value)) = part.split_once('=') else {
                continue;
            };
            if !name.trim().eq_ignore_ascii_case("for") {
                continue;
            }
            let mut value = value.trim().trim_matches('"');
            if value.starts_with('[')
                && let Some(end) = value.find(']')
            {
                value = &value[1..end];
            }
            if let Some(normalized) = normalize_client_key_value(value) {
                return Some(normalized);
            }
        }
    }
    None
}

fn login_attempt_key(headers: &HeaderMap) -> String {
    if let Some(v) = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .and_then(normalize_client_key_value)
    {
        return format!("xff:{v}");
    }

    if let Some(v) = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(normalize_client_key_value)
    {
        return format!("xri:{v}");
    }

    if let Some(v) = headers
        .get("forwarded")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_forwarded_for)
    {
        return format!("fwd:{v}");
    }

    if let Some(v) = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .and_then(normalize_client_key_value)
    {
        return format!("ua:{v}");
    }

    "anonymous".to_string()
}

fn verify_password(phc: &str, candidate: &str) -> bool {
    if candidate.len() > MAX_LOGIN_PASSWORD_BYTES {
        return false;
    }
    let Ok(hash) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default()
        .verify_password(candidate.as_bytes(), &hash)
        .is_ok()
}

fn get_token_from_authorization(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let trimmed = raw.trim();
    // Authorization: Bearer <token>
    let mut parts = trimmed.split_whitespace();
    let scheme = parts.next().unwrap_or("");
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = parts.next().unwrap_or("").trim();
    if token.is_empty() {
        return None;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(token.to_string())
}

fn is_session_valid(inner: &UiAuthInner, token: &str) -> bool {
    let now = OffsetDateTime::now_utc();
    let Some(mut entry) = inner.sessions.get_mut(token) else {
        return false;
    };

    if now - entry.last_seen > time::Duration::seconds(UI_SESSION_TTL.as_secs() as i64) {
        drop(entry);
        inner.sessions.remove(token);
        return false;
    }

    entry.last_seen = now;
    true
}

fn login_failure_window_duration() -> time::Duration {
    time::Duration::seconds(LOGIN_FAILURE_WINDOW.as_secs() as i64)
}

fn login_lockout_duration(duration: Duration) -> time::Duration {
    time::Duration::seconds(duration.as_secs() as i64)
}

fn login_lockout_remaining_seconds(
    inner: &UiAuthInner,
    attempt_key: &str,
    now: OffsetDateTime,
) -> Option<i64> {
    let mut entry = inner.login_attempts.get_mut(attempt_key)?;

    if let Some(locked_until) = entry.locked_until {
        if locked_until > now {
            return Some((locked_until - now).whole_seconds().max(1));
        }

        // Lockout elapsed; reset counters.
        entry.window_started = now;
        entry.failures = 0;
        entry.locked_until = None;
        return None;
    }

    if now - entry.window_started > login_failure_window_duration() {
        entry.window_started = now;
        entry.failures = 0;
    }

    None
}

fn record_failed_login_attempt(
    inner: &UiAuthInner,
    attempt_key: &str,
    now: OffsetDateTime,
    failure_limit: u32,
    lockout_duration: Duration,
) -> Option<i64> {
    let mut entry = inner
        .login_attempts
        .entry(attempt_key.to_string())
        .or_insert(LoginAttemptRecord {
            window_started: now,
            failures: 0,
            locked_until: None,
        });

    if now - entry.window_started > login_failure_window_duration() {
        entry.window_started = now;
        entry.failures = 0;
        entry.locked_until = None;
    }

    entry.failures = entry.failures.saturating_add(1);
    if entry.failures < failure_limit {
        return None;
    }

    let locked_until = now + login_lockout_duration(lockout_duration);
    entry.locked_until = Some(locked_until);
    Some((locked_until - now).whole_seconds().max(1))
}

fn login_lockout_remaining_for_request(
    inner: &UiAuthInner,
    attempt_key: &str,
    now: OffsetDateTime,
) -> Option<i64> {
    [
        login_lockout_remaining_seconds(inner, attempt_key, now),
        login_lockout_remaining_seconds(inner, GLOBAL_LOGIN_ATTEMPT_KEY, now),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn record_failed_login_attempts(
    inner: &UiAuthInner,
    attempt_key: &str,
    now: OffsetDateTime,
) -> Option<i64> {
    [
        record_failed_login_attempt(
            inner,
            attempt_key,
            now,
            LOGIN_FAILURE_LIMIT,
            LOGIN_LOCKOUT_DURATION,
        ),
        record_failed_login_attempt(
            inner,
            GLOBAL_LOGIN_ATTEMPT_KEY,
            now,
            GLOBAL_LOGIN_FAILURE_LIMIT,
            GLOBAL_LOGIN_LOCKOUT_DURATION,
        ),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn clear_failed_login_attempts(inner: &UiAuthInner, attempt_key: &str) {
    inner.login_attempts.remove(attempt_key);
    // A successful authentication proves possession of the credential and
    // safely releases the coarse global circuit breaker as well.
    inner.login_attempts.remove(GLOBAL_LOGIN_ATTEMPT_KEY);
}

async fn cleanup_sessions_task(inner: std::sync::Weak<UiAuthInner>) {
    let mut ticker = tokio::time::interval(UI_SESSION_CLEANUP_INTERVAL);
    loop {
        ticker.tick().await;
        let Some(inner) = inner.upgrade() else {
            break;
        };
        let now = OffsetDateTime::now_utc();
        let ttl = time::Duration::seconds(UI_SESSION_TTL.as_secs() as i64);
        let login_window = login_failure_window_duration();
        inner
            .sessions
            .retain(|_, record| now - record.last_seen <= ttl);
        inner.login_attempts.retain(|_, record| {
            if let Some(locked_until) = record.locked_until
                && locked_until > now
            {
                return true;
            }

            record.failures > 0 && now - record.window_started <= login_window
        });
    }
}

pub(crate) fn spawn_cleanup_sessions_task_if_enabled(ui_auth: &UiAuth) -> bool {
    match ui_auth {
        UiAuth::Disabled => false,
        UiAuth::Enabled(inner) => {
            tokio::spawn(cleanup_sessions_task(Arc::downgrade(inner)));
            true
        }
    }
}

pub(crate) fn init_ui_auth(ui_password: Option<String>) -> UiAuth {
    let password = normalize_password(ui_password.as_deref());
    if password.is_empty() {
        return UiAuth::Disabled;
    }

    let password_phc = hash_password(password.as_str()).expect("init_ui_auth: hash password");
    init_ui_auth_from_phc(password_phc).expect("init_ui_auth: invalid password hash")
}

/// Hash a password for a server-owned credential which is persisted as an
/// Argon2 PHC string. The plaintext never needs to leave the caller's stack.
pub(crate) fn hash_password(candidate: &str) -> Result<String, String> {
    let password = normalize_password(Some(candidate));
    if password.is_empty() {
        return Err("password must not be empty".to_owned());
    }

    let mut salt_bytes = [0u8; 16];
    getrandom::fill(&mut salt_bytes).map_err(|error| error.to_string())?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|error| error.to_string())?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| error.to_string())
}

/// Rehydrate a password verifier from a PHC value read from server state.
/// Keeping the login-attempt map on the verifier preserves the same OAuth
/// lockout behavior as the UI password without ever returning the PHC value.
pub(crate) fn init_ui_auth_from_phc(password_phc: String) -> Result<UiAuth, String> {
    PasswordHash::new(password_phc.as_str()).map_err(|error| error.to_string())?;
    Ok(UiAuth::Enabled(Arc::new(UiAuthInner {
        password_phc,
        sessions: DashMap::new(),
        login_attempts: DashMap::new(),
    })))
}

/// Verify the server UI password for the MCP OAuth authorization page.
///
/// MCP authorization is deliberately a separate browser flow from the UI
/// bearer session: a successful password check issues an OAuth authorization
/// code, never a UI session token. The same failure counters and lockout
/// policy are shared so the public OAuth endpoint does not become a weaker
/// password oracle.
pub(crate) fn verify_password_for_oauth(
    ui_auth: &UiAuth,
    candidate: &str,
    headers: &HeaderMap,
) -> Result<(), OAuthPasswordError> {
    let UiAuth::Enabled(inner) = ui_auth else {
        return Err(OAuthPasswordError::NotConfigured);
    };

    let attempt_key = login_attempt_key(headers);
    let now = OffsetDateTime::now_utc();
    if let Some(retry_after_seconds) = login_lockout_remaining_for_request(inner, &attempt_key, now)
    {
        return Err(OAuthPasswordError::Locked(retry_after_seconds));
    }

    let candidate = normalize_password(Some(candidate));
    if !verify_password(&inner.password_phc, &candidate) {
        if let Some(retry_after_seconds) = record_failed_login_attempts(inner, &attempt_key, now) {
            return Err(OAuthPasswordError::Locked(retry_after_seconds));
        }
        return Err(OAuthPasswordError::Invalid);
    }

    clear_failed_login_attempts(inner, &attempt_key);
    Ok(())
}

pub(crate) async fn auth_session_status(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    match &state.ui_auth {
        UiAuth::Disabled => Json(AuthStatusOk {
            authenticated: true,
            disabled: true,
            token: None,
        })
        .into_response(),
        UiAuth::Enabled(inner) => {
            if let Some(token) = get_token_from_authorization(&headers)
                && is_session_valid(inner, &token)
            {
                return Json(AuthStatusOk {
                    authenticated: true,
                    disabled: false,
                    token: None,
                })
                .into_response();
            }

            Json(AuthStatusLocked {
                authenticated: false,
                locked: true,
            })
            .into_response()
        }
    }
}

pub(crate) async fn auth_session_create(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateSessionBody>,
) -> impl IntoResponse {
    let candidate = normalize_password(body.password.as_deref());

    match &state.ui_auth {
        UiAuth::Disabled => (
            StatusCode::BAD_REQUEST,
            Json(AuthErrorBody {
                error: "UI password not configured".to_string(),
                locked: false,
                code: Some("auth_disabled".to_string()),
                retry_after_seconds: None,
            }),
        )
            .into_response(),
        UiAuth::Enabled(inner) => {
            let attempt_key = login_attempt_key(&headers);
            let now = OffsetDateTime::now_utc();

            if let Some(retry_after_seconds) =
                login_lockout_remaining_for_request(inner, &attempt_key, now)
            {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(AuthErrorBody {
                        error: format!(
                            "Too many failed login attempts. Try again in {} seconds",
                            retry_after_seconds
                        ),
                        locked: true,
                        code: Some("auth_rate_limited".to_string()),
                        retry_after_seconds: Some(retry_after_seconds),
                    }),
                )
                    .into_response();
            }

            if !verify_password(&inner.password_phc, &candidate) {
                if let Some(retry_after_seconds) =
                    record_failed_login_attempts(inner, &attempt_key, now)
                {
                    return (
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(AuthErrorBody {
                            error: format!(
                                "Too many failed login attempts. Try again in {} seconds",
                                retry_after_seconds
                            ),
                            locked: true,
                            code: Some("auth_rate_limited".to_string()),
                            retry_after_seconds: Some(retry_after_seconds),
                        }),
                    )
                        .into_response();
                }

                return (
                    StatusCode::UNAUTHORIZED,
                    Json(AuthErrorBody {
                        error: "Invalid password".to_string(),
                        locked: true,
                        code: Some("auth_invalid_password".to_string()),
                        retry_after_seconds: None,
                    }),
                )
                    .into_response();
            }

            clear_failed_login_attempts(inner, &attempt_key);

            let token = crate::server::issue_token();
            inner
                .sessions
                .insert(token.clone(), SessionRecord { last_seen: now });

            (
                StatusCode::OK,
                Json(AuthStatusOk {
                    authenticated: true,
                    disabled: false,
                    token: Some(token),
                }),
            )
                .into_response()
        }
    }
}

pub(crate) async fn require_ui_auth(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    req: axum::http::Request<axum::body::Body>,
    next: middleware::Next,
) -> impl IntoResponse {
    let req_method = req.method().clone();
    let req_path = req.uri().path().to_string();

    match &state.ui_auth {
        UiAuth::Disabled => next.run(req).await,
        UiAuth::Enabled(inner) => {
            // Server discovery and identity-safe lifecycle controls must work
            // before a UI login exists. This endpoint exposes only readiness
            // and process identity; all session/runtime APIs remain protected.
            if req_method == Method::GET && req_path == "/api/v1/health" {
                return next.run(req).await;
            }

            // Header token (preferred): avoids third-party cookie issues and doesn't
            // require CSRF origin enforcement because the token isn't sent automatically.
            if let Some(token) = get_token_from_authorization(&headers)
                && is_session_valid(inner, &token)
            {
                return next.run(req).await;
            }

            (
                StatusCode::UNAUTHORIZED,
                Json(AuthErrorBody {
                    error: "UI authentication required".to_string(),
                    locked: true,
                    code: Some("auth_required".to_string()),
                    retry_after_seconds: None,
                }),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_appended_address_wins_over_spoofed_forwarded_prefixes() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "198.51.100.10, 203.0.113.42".parse().expect("valid header"),
        );
        assert_eq!(login_attempt_key(&headers), "xff:203.0.113.42");

        headers.remove("x-forwarded-for");
        headers.insert(
            "forwarded",
            "for=198.51.100.10;proto=https, for=203.0.113.42;proto=https"
                .parse()
                .expect("valid header"),
        );
        assert_eq!(login_attempt_key(&headers), "fwd:203.0.113.42");
    }

    #[test]
    fn rotating_spoofed_client_keys_still_trip_the_global_login_circuit_breaker() {
        let UiAuth::Enabled(inner) =
            init_ui_auth_from_phc(hash_password("correct horse battery staple").unwrap()).unwrap()
        else {
            panic!("password auth should be enabled");
        };
        let now = OffsetDateTime::now_utc();
        let mut retry_after = None;
        for index in 0..GLOBAL_LOGIN_FAILURE_LIMIT {
            retry_after =
                record_failed_login_attempts(&inner, format!("spoofed:{index}").as_str(), now);
        }
        assert!(retry_after.is_some());
        assert!(login_lockout_remaining_for_request(&inner, "brand-new-spoof", now).is_some());
    }
}
