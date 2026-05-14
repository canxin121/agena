use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use agena::config::ConfigLoader;
use agena::runtime::AgenaRuntime;
use agena::storage::StorageConfig;
use agena::tracing as tracing_config;
use agena_api_server::AppState as ApiV2State;
use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    extract::Query,
    http::{
        HeaderValue, Method,
        header::{self, HeaderName},
    },
    middleware,
    routing::get,
};
use axum_extra::extract::cookie::SameSite;
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Deserialize, Default)]
struct GitStatusCompatQuery {
    directory: Option<String>,
    summary: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GitStatusCompatResponse {
    current: String,
    tracking: Option<String>,
    ahead: u64,
    behind: u64,
    files: Vec<GitStatusCompatFile>,
    total_files: u64,
    staged_count: u64,
    unstaged_count: u64,
    untracked_count: u64,
    merge_count: u64,
    offset: u64,
    limit: u64,
    has_more: bool,
    scope: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GitStatusCompatFile {
    path: String,
    index: String,
    working_dir: String,
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

fn command_available(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn parse_ahead_behind(raw: Option<&str>) -> (u64, u64) {
    let Some(raw) = raw else {
        return (0, 0);
    };
    let mut parts = raw.split_whitespace();
    let ahead = parts
        .next()
        .and_then(|part| part.parse::<u64>().ok())
        .unwrap_or(0);
    let behind = parts
        .next()
        .and_then(|part| part.parse::<u64>().ok())
        .unwrap_or(0);
    (ahead, behind)
}

fn summarize_git_status(status: &str) -> (u64, u64, u64, u64) {
    let mut staged = 0_u64;
    let mut unstaged = 0_u64;
    let mut untracked = 0_u64;
    let mut changed = 0_u64;

    for line in status.lines().filter(|line| !line.is_empty()) {
        changed += 1;
        let bytes = line.as_bytes();
        let x = bytes.first().copied().unwrap_or(b' ');
        let y = bytes.get(1).copied().unwrap_or(b' ');
        if x == b'?' && y == b'?' {
            untracked += 1;
            continue;
        }
        if x != b' ' {
            staged += 1;
        }
        if y != b' ' {
            unstaged += 1;
        }
    }

    (staged, unstaged, untracked, changed)
}

async fn compat_git_status(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Query(query): Query<GitStatusCompatQuery>,
) -> Json<GitStatusCompatResponse> {
    let workspace_root = query
        .directory
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| state.runtime.workspace_root().to_path_buf());
    let summary_only = query.summary.unwrap_or(false);
    let scope = if summary_only { "summary" } else { "full" }.to_string();

    if !command_available("git") {
        return Json(GitStatusCompatResponse {
            current: String::new(),
            tracking: None,
            ahead: 0,
            behind: 0,
            files: Vec::new(),
            total_files: 0,
            staged_count: 0,
            unstaged_count: 0,
            untracked_count: 0,
            merge_count: 0,
            offset: 0,
            limit: 0,
            has_more: false,
            scope,
        });
    }

    let repo = git_output(&workspace_root, &["rev-parse", "--is-inside-work-tree"])
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| value == "true");
    if !repo {
        return Json(GitStatusCompatResponse {
            current: String::new(),
            tracking: None,
            ahead: 0,
            behind: 0,
            files: Vec::new(),
            total_files: 0,
            staged_count: 0,
            unstaged_count: 0,
            untracked_count: 0,
            merge_count: 0,
            offset: 0,
            limit: 0,
            has_more: false,
            scope,
        });
    }

    let current = git_output(&workspace_root, &["branch", "--show-current"]).unwrap_or_default();
    let tracking = git_output(
        &workspace_root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
    )
    .and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed.to_string())
    });
    let ahead_behind = tracking.as_ref().and_then(|_| {
        git_output(
            &workspace_root,
            &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        )
    });
    let (ahead, behind) = parse_ahead_behind(ahead_behind.as_deref());
    let status = git_output(&workspace_root, &["status", "--porcelain"]).unwrap_or_default();
    let (staged_count, unstaged_count, untracked_count, total_files) =
        summarize_git_status(status.as_str());

    Json(GitStatusCompatResponse {
        current,
        tracking,
        ahead,
        behind,
        files: Vec::new(),
        total_files,
        staged_count,
        unstaged_count,
        untracked_count,
        merge_count: 0,
        offset: 0,
        limit: 0,
        has_more: false,
        scope,
    })
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

    let shared_state = Arc::new(AppState {
        ui_auth: crate::ui_auth::init_ui_auth(args.ui_password.clone()),
        ui_cookie_same_site: resolve_same_site(
            args.ui_cookie_samesite.clone(),
            args.cors_allow_all || !normalized_cors_origins.is_empty(),
        ),
        cors_allowed_origins: normalized_cors_origins.clone(),
        cors_allow_all: args.cors_allow_all,
        runtime: runtime.clone(),
    });
    let _ = crate::ui_auth::spawn_cleanup_sessions_task_if_enabled(&shared_state.ui_auth);

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
    let git_compat = Router::new()
        .route("/api/git/status", get(compat_git_status))
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
        .merge(git_compat)
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
}
