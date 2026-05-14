use std::{
    collections::HashSet,
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
    body::Body,
    extract::Query,
    http::{
        HeaderValue, Method, StatusCode,
        header::{self, HeaderName},
    },
    middleware,
    response::Response,
    routing::get,
};
use axum_extra::extract::cookie::SameSite;
use mime_guess::MimeGuess;
use path_clean::PathClean;
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

const MAX_COMPAT_FILE_BYTES: u64 = 50 * 1024 * 1024;
const MAX_COMPAT_LIST_LIMIT: usize = 2000;

type CompatResult<T> = Result<T, (StatusCode, String)>;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FsHomeCompatResponse {
    home: String,
    path: String,
}

#[derive(Debug, Deserialize, Default)]
struct FsListCompatQuery {
    path: Option<String>,
    #[serde(rename = "respectGitignore")]
    respect_gitignore: Option<bool>,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FsListCompatEntry {
    name: String,
    path: String,
    is_directory: bool,
    is_file: bool,
    is_symbolic_link: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FsListCompatResponse {
    path: String,
    entries: Vec<FsListCompatEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
    total: usize,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_offset: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
struct FsFileCompatQuery {
    directory: Option<String>,
    path: Option<String>,
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

fn compat_bad_request(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, message.into())
}

fn compat_forbidden(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::FORBIDDEN, message.into())
}

fn compat_not_found(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, message.into())
}

fn compat_internal(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, message.into())
}

fn compat_payload_too_large(message: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::PAYLOAD_TOO_LARGE, message.into())
}

fn compat_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            let drive = env::var_os("HOMEDRIVE")?;
            let path = env::var_os("HOMEPATH")?;
            if drive.is_empty() || path.is_empty() {
                return None;
            }
            let mut joined = PathBuf::from(drive);
            joined.push(path);
            Some(joined)
        })
}

fn compat_cwd() -> PathBuf {
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn compat_path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn compat_resolve_path(raw: &str) -> PathBuf {
    let candidate = PathBuf::from(raw);
    let absolute = if candidate.is_absolute() {
        candidate
    } else {
        compat_cwd().join(candidate)
    };
    absolute.clean()
}

async fn compat_validate_directory(raw: &str) -> CompatResult<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(compat_bad_request("Path is required"));
    }

    let absolute = compat_resolve_path(trimmed);
    let metadata = tokio::fs::metadata(&absolute)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => compat_not_found("Directory not found"),
            std::io::ErrorKind::PermissionDenied => compat_forbidden("Access to directory denied"),
            _ => compat_internal("Failed to validate directory"),
        })?;

    if !metadata.is_dir() {
        return Err(compat_bad_request("Specified path is not a directory"));
    }

    Ok(absolute)
}

fn compat_default_list_root() -> PathBuf {
    compat_home_dir().unwrap_or_else(compat_cwd).clean()
}

fn compat_git_check_ignore(directory: &Path, names: &[String]) -> HashSet<String> {
    if names.is_empty() || !command_available("git") {
        return HashSet::new();
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .arg("check-ignore")
        .arg("--")
        .args(names)
        .output();

    let Ok(output) = output else {
        return HashSet::new();
    };
    if !output.status.success() {
        return HashSet::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn compat_mime(path: &Path) -> String {
    MimeGuess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

fn compat_content_disposition(path: &Path, disposition_type: &str) -> String {
    let raw = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".to_string());

    let mut ascii = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let safe = ch.is_ascii() && !matches!(ch, '"' | '\\') && !ch.is_ascii_control();
        ascii.push(if safe { ch } else { '_' });
    }
    if ascii.trim().is_empty() {
        ascii = "download".to_string();
    }

    format!(
        "{}; filename=\"{}\"; filename*=UTF-8''{}",
        disposition_type,
        ascii,
        urlencoding::encode(&raw)
    )
}

fn compat_content_disposition_inline(path: &Path) -> String {
    compat_content_disposition(path, "inline")
}

fn compat_content_disposition_attachment(path: &Path) -> String {
    compat_content_disposition(path, "attachment")
}

async fn compat_resolve_scoped_file(
    directory: Option<&str>,
    path: Option<&str>,
) -> CompatResult<PathBuf> {
    let directory = directory
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("Directory parameter is required"))?;
    let base = compat_validate_directory(directory).await?;

    let target = path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("Path is required"))?;

    let raw_target = PathBuf::from(target);
    let absolute = if raw_target.is_absolute() {
        raw_target.clean()
    } else {
        base.join(raw_target).clean()
    };

    if !absolute.starts_with(&base) {
        return Err(compat_bad_request("Path is outside of active directory"));
    }

    let metadata = tokio::fs::metadata(&absolute)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => compat_not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => compat_forbidden("Access to file denied"),
            _ => compat_internal("Failed to read file"),
        })?;

    if !metadata.is_file() {
        return Err(compat_bad_request("Specified path is not a file"));
    }
    if metadata.len() > MAX_COMPAT_FILE_BYTES {
        return Err(compat_payload_too_large("File too large"));
    }

    Ok(absolute)
}

async fn compat_fs_home() -> CompatResult<Json<FsHomeCompatResponse>> {
    let home = compat_default_list_root();
    let path = compat_path_string(&home);
    Ok(Json(FsHomeCompatResponse {
        home: path.clone(),
        path,
    }))
}

async fn compat_fs_list(
    Query(query): Query<FsListCompatQuery>,
) -> CompatResult<Json<FsListCompatResponse>> {
    let requested = query
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let directory = match requested {
        Some(value) => compat_validate_directory(&value).await?,
        None => compat_default_list_root(),
    };
    let respect_gitignore = query.respect_gitignore.unwrap_or(false);

    let mut read_dir =
        tokio::fs::read_dir(&directory)
            .await
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => compat_not_found("Directory not found"),
                std::io::ErrorKind::PermissionDenied => {
                    compat_forbidden("Access to directory denied")
                }
                _ => compat_internal(error.to_string()),
            })?;

    let mut raw_entries = Vec::new();
    let mut names = Vec::new();
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|error| compat_internal(error.to_string()))?
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        names.push(name.clone());
        raw_entries.push((name, entry));
    }

    let ignored = if respect_gitignore {
        compat_git_check_ignore(&directory, &names)
    } else {
        HashSet::new()
    };

    let mut entries = Vec::new();
    for (name, entry) in raw_entries {
        if respect_gitignore && ignored.contains(&name) {
            continue;
        }

        let path = entry.path();
        let file_type = match entry.file_type().await {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        let is_symbolic_link = file_type.is_symlink();
        let mut is_directory = file_type.is_dir();
        if !is_directory && is_symbolic_link {
            if let Ok(target_metadata) = tokio::fs::metadata(&path).await {
                is_directory = target_metadata.is_dir();
            }
        }

        entries.push(FsListCompatEntry {
            name,
            path: compat_path_string(&path),
            is_directory,
            is_file: file_type.is_file(),
            is_symbolic_link,
        });
    }

    entries.sort_by(|left, right| left.name.cmp(&right.name));

    let total = entries.len();
    let offset = query.offset.unwrap_or(0).min(total);
    let limit = query
        .limit
        .map(|value| value.clamp(1, MAX_COMPAT_LIST_LIMIT))
        .filter(|value| *value > 0);

    let (entries, has_more, next_offset) = if let Some(limit) = limit {
        let end = offset.saturating_add(limit).min(total);
        let has_more = end < total;
        let next_offset = has_more.then_some(end);
        (entries[offset..end].to_vec(), has_more, next_offset)
    } else if offset > 0 {
        (entries[offset..].to_vec(), false, None)
    } else {
        (entries, false, None)
    };

    Ok(Json(FsListCompatResponse {
        path: compat_path_string(&directory),
        entries,
        offset: (query.limit.is_some() || offset > 0).then_some(offset),
        limit,
        total,
        has_more,
        next_offset,
    }))
}

async fn compat_fs_raw(Query(query): Query<FsFileCompatQuery>) -> CompatResult<Response> {
    let path =
        compat_resolve_scoped_file(query.directory.as_deref(), query.path.as_deref()).await?;
    let content = tokio::fs::read(&path)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => compat_not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => compat_forbidden("Access to file denied"),
            _ => compat_internal(error.to_string()),
        })?;

    Response::builder()
        .status(StatusCode::OK)
        .header("cache-control", "no-store")
        .header("content-type", compat_mime(&path))
        .header(
            "content-disposition",
            compat_content_disposition_inline(&path),
        )
        .body(Body::from(content))
        .map_err(|error| compat_internal(error.to_string()))
}

async fn compat_fs_download(Query(query): Query<FsFileCompatQuery>) -> CompatResult<Response> {
    let path =
        compat_resolve_scoped_file(query.directory.as_deref(), query.path.as_deref()).await?;
    let content = tokio::fs::read(&path)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => compat_not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => compat_forbidden("Access to file denied"),
            _ => compat_internal(error.to_string()),
        })?;

    Response::builder()
        .status(StatusCode::OK)
        .header("cache-control", "no-store")
        .header("content-type", compat_mime(&path))
        .header(
            "content-disposition",
            compat_content_disposition_attachment(&path),
        )
        .body(Body::from(content))
        .map_err(|error| compat_internal(error.to_string()))
}

fn compat_fs_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/fs/home", get(compat_fs_home))
        .route("/api/fs/list", get(compat_fs_list))
        .route("/api/fs/raw", get(compat_fs_raw))
        .route("/api/fs/download", get(compat_fs_download))
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
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
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
    let compat_routes = compat_fs_router::<Arc<AppState>>()
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
        .merge(compat_routes)
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
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tempfile::tempdir;
    use tower::ServiceExt;

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
    async fn compat_fs_home_route_returns_non_empty_home_path() {
        let response = compat_fs_router::<()>()
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
        let payload: FsHomeCompatResponse =
            serde_json::from_slice(&body).expect("response should be valid json");
        assert!(!payload.home.is_empty());
        assert_eq!(payload.home, payload.path);
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
        let response = compat_fs_router::<()>()
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
        let payload: FsListCompatResponse =
            serde_json::from_slice(&body).expect("response should be valid json");
        assert_eq!(payload.path, compat_path_string(temp.path()));
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

        let raw_uri = format!("/api/fs/raw?directory={directory}&path=notes.txt");
        let raw_response = compat_fs_router::<()>()
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
        let download_response = compat_fs_router::<()>()
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

        let traversal_uri = format!("/api/fs/raw?directory={directory}&path=../notes.txt");
        let traversal_response = compat_fs_router::<()>()
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
}
