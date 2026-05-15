use std::{
    collections::HashSet,
    convert::Infallible,
    env,
    io::Write as _,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use agena::config::ConfigLoader;
use agena::runtime::AgenaRuntime;
use agena::storage::StorageConfig;
use agena::tracing as tracing_config;
use agena_api_server::AppState as ApiV2State;
use anyhow::{Context, Result, anyhow};
use async_stream::stream;
use axum::{
    Json, Router,
    body::Body,
    extract::Query,
    http::{
        HeaderValue, Method, StatusCode,
        header::{self, HeaderName},
    },
    middleware,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use axum_extra::extract::cookie::SameSite;
use ignore::WalkBuilder;
use mime_guess::MimeGuess;
use path_clean::PathClean;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
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
const DEFAULT_COMPAT_READ_CHUNK_LIMIT: usize = 256 * 1024;
const MAX_COMPAT_READ_CHUNK_LIMIT: usize = 2 * 1024 * 1024;
const DEFAULT_COMPAT_SEARCH_LIMIT: usize = 60;
const MAX_COMPAT_SEARCH_LIMIT: usize = 400;
const DEFAULT_CONTENT_SEARCH_MAX_RESULTS: usize = 200;
const MAX_CONTENT_SEARCH_MAX_RESULTS: usize = 1000;
const DEFAULT_CONTENT_SEARCH_MAX_MATCHES_PER_FILE: usize = 20;
const MAX_CONTENT_SEARCH_MAX_MATCHES_PER_FILE: usize = 200;
const DEFAULT_CONTENT_SEARCH_CONTEXT_CHARS: usize = 80;
const MAX_CONTENT_SEARCH_CONTEXT_CHARS: usize = 240;
const MAX_CONTENT_SCOPE_PATHS: usize = 10_000;
const COMPAT_FILE_SEARCH_EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    "coverage",
    "tmp",
    "logs",
];

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

#[derive(Debug, Deserialize, Default)]
struct FsReadChunkCompatQuery {
    directory: Option<String>,
    path: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FsReadChunkCompatResponse {
    path: String,
    content: String,
    offset: usize,
    limit: usize,
    loaded_bytes: usize,
    total_bytes: usize,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_offset: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
struct FsWriteCompatQuery {
    directory: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FsWriteCompatBody {
    path: Option<String>,
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FsMkdirCompatBody {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FsDeleteCompatBody {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FsRenameCompatBody {
    old_path: Option<String>,
    new_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FsWriteCompatResponse {
    success: bool,
}

#[derive(Debug, Deserialize, Default)]
struct FsSearchCompatQuery {
    root: Option<String>,
    directory: Option<String>,
    q: Option<String>,
    #[serde(rename = "includeHidden")]
    include_hidden: Option<bool>,
    #[serde(rename = "respectGitignore")]
    respect_gitignore: Option<bool>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FsSearchCompatFile {
    name: String,
    path: String,
    relative_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct FsSearchCompatResponse {
    root: String,
    count: usize,
    files: Vec<FsSearchCompatFile>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FsContentSearchCompatBody {
    query: Option<String>,
    paths: Option<Vec<String>>,
    include_hidden: Option<bool>,
    respect_gitignore: Option<bool>,
    is_regex: Option<bool>,
    case_sensitive: Option<bool>,
    whole_word: Option<bool>,
    max_results: Option<usize>,
    max_matches_per_file: Option<usize>,
    context_chars: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FsContentSearchMatchCompat {
    line: usize,
    start_column: usize,
    end_column: usize,
    start_offset: usize,
    end_offset: usize,
    before: String,
    matched: String,
    after: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FsContentSearchFileCompat {
    path: String,
    relative_path: String,
    match_count: usize,
    matches: Vec<FsContentSearchMatchCompat>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FsContentSearchResponseCompat {
    root: String,
    query: String,
    file_count: usize,
    match_count: usize,
    files: Vec<FsContentSearchFileCompat>,
    truncated: bool,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FsContentReplaceTargetCompat {
    path: Option<String>,
    start_offset: Option<usize>,
    end_offset: Option<usize>,
    expected: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FsContentReplaceCompatBody {
    query: Option<String>,
    replace: Option<String>,
    include_hidden: Option<bool>,
    respect_gitignore: Option<bool>,
    is_regex: Option<bool>,
    case_sensitive: Option<bool>,
    whole_word: Option<bool>,
    paths: Option<Vec<String>>,
    r#match: Option<FsContentReplaceTargetCompat>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FsContentReplaceFileCompat {
    path: String,
    relative_path: String,
    replacements: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct FsContentReplaceResponseCompat {
    root: String,
    file_count: usize,
    replacement_count: usize,
    skipped: usize,
    files: Vec<FsContentReplaceFileCompat>,
}

#[derive(Debug, Deserialize, Default)]
struct GitPathCompatQuery {
    directory: Option<String>,
    path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GitDiffCompatQuery {
    directory: Option<String>,
    path: Option<String>,
    staged: Option<bool>,
    context_lines: Option<usize>,
    include_meta: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct GitFileDiffCompatQuery {
    directory: Option<String>,
    path: Option<String>,
    staged: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GitWatchCompatQuery {
    directory: Option<String>,
    interval_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GitPatchCompatBody {
    patch: Option<String>,
    mode: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GitBlameLineCompat {
    line: usize,
    hash: String,
    author: String,
    author_email: String,
    author_time: u64,
    summary: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
struct GitBlameResponseCompat {
    lines: Vec<GitBlameLineCompat>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GitFileDiffResponseCompat {
    original: String,
    modified: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GitDiffSummaryCompat {
    files: usize,
    hunks: usize,
    changed_lines: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GitDiffHunkMetaCompat {
    id: String,
    header: String,
    range: String,
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    additions: usize,
    deletions: usize,
    anchor_line: usize,
    lines: Vec<String>,
    patch: String,
    patch_ready: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GitDiffMetaCompat {
    file_header: Vec<String>,
    has_patch_header: bool,
    hunks: Vec<GitDiffHunkMetaCompat>,
    summary: GitDiffSummaryCompat,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GitDiffResponseCompat {
    diff: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<GitDiffMetaCompat>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GitWatchStatusPayloadCompat {
    current: String,
    tracking: Option<String>,
    ahead: u64,
    behind: u64,
    staged_count: u64,
    unstaged_count: u64,
    untracked_count: u64,
    merge_count: u64,
    is_clean: bool,
    worktree_signature: String,
}

static DIFF_HUNK_HEADER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^@@\s*-(\d+)(?:,(\d+))?\s+\+(\d+)(?:,(\d+))?\s*@@")
        .expect("diff hunk header regex should compile")
});

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

fn compat_decode_utf8_chunk(bytes: &[u8]) -> CompatResult<(String, usize)> {
    if bytes.is_empty() {
        return Ok((String::new(), 0));
    }

    match std::str::from_utf8(bytes) {
        Ok(content) => Ok((content.to_string(), bytes.len())),
        Err(error) => {
            if error.error_len().is_some() {
                return Err(compat_bad_request("Specified file is not UTF-8 text"));
            }

            let valid_up_to = error.valid_up_to();
            let content = std::str::from_utf8(&bytes[..valid_up_to])
                .map_err(|_| compat_bad_request("Specified file is not UTF-8 text"))?;
            Ok((content.to_string(), valid_up_to))
        }
    }
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

async fn compat_resolve_scoped_path(
    directory: Option<&str>,
    path: Option<&str>,
) -> CompatResult<(PathBuf, PathBuf)> {
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

    Ok((base, absolute))
}

fn compat_normalize_relative_search_path(root: &Path, target: &Path) -> String {
    let rel = target
        .strip_prefix(root)
        .ok()
        .and_then(|path| {
            if path.as_os_str().is_empty() {
                None
            } else {
                Some(path)
            }
        })
        .unwrap_or_else(|| target.file_name().map(Path::new).unwrap_or(target));
    rel.to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn compat_is_safe_repo_rel_path(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return false;
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return false;
    }
    !candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
}

async fn compat_require_directory(raw: Option<&str>) -> CompatResult<PathBuf> {
    let raw = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("directory parameter is required"))?;
    compat_validate_directory(raw).await
}

async fn compat_normalize_scope_paths(
    root: &Path,
    paths: &[String],
    include_hidden: bool,
    respect_gitignore: bool,
) -> CompatResult<Vec<PathBuf>> {
    let mut resolved = Vec::new();
    let mut seen = HashSet::<PathBuf>::new();

    for raw in paths.iter().take(MAX_CONTENT_SCOPE_PATHS) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let path = root.join(trimmed).clean();
        if !path.starts_with(root) {
            return Err(compat_bad_request("Path is outside of active directory"));
        }
        let metadata = match tokio::fs::metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(compat_forbidden("Access denied"));
            }
            Err(error) => return Err(compat_internal(error.to_string())),
        };

        if metadata.is_file() {
            if seen.insert(path.clone()) {
                resolved.push(path);
            }
            continue;
        }

        if metadata.is_dir() {
            let nested =
                compat_walk_workspace_files(&path, include_hidden, respect_gitignore, MAX_CONTENT_SCOPE_PATHS);
            for file in nested {
                if seen.insert(file.clone()) {
                    resolved.push(file);
                }
            }
        }
    }

    Ok(resolved)
}

fn compat_walk_workspace_files(
    root: &Path,
    include_hidden: bool,
    respect_gitignore: bool,
    limit: usize,
) -> Vec<PathBuf> {
    let excluded: HashSet<&'static str> = COMPAT_FILE_SEARCH_EXCLUDED_DIRS.iter().copied().collect();
    let root_for_filter = root.to_path_buf();
    let mut builder = WalkBuilder::new(root);
    builder.hidden(!include_hidden);
    if !respect_gitignore {
        builder.git_ignore(false);
        builder.git_global(false);
        builder.git_exclude(false);
        builder.parents(false);
    }
    builder.follow_links(false);

    let mut files = Vec::new();
    for result in builder
        .filter_entry(move |entry| {
            let path = entry.path();
            if path == root_for_filter {
                return true;
            }

            let Some(name) = path.file_name().and_then(|segment| segment.to_str()) else {
                return true;
            };

            let lower = name.to_ascii_lowercase();
            if excluded.contains(lower.as_str()) {
                return false;
            }
            if !include_hidden && name.starts_with('.') {
                return false;
            }
            true
        })
        .build()
    {
        let entry = match result {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        files.push(entry.path().to_path_buf());
        if files.len() >= limit {
            break;
        }
    }
    files
}

async fn compat_read_searchable_text(path: &Path) -> Option<String> {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    if !metadata.is_file() || metadata.len() > MAX_COMPAT_FILE_BYTES {
        return None;
    }
    tokio::fs::read_to_string(path).await.ok()
}

fn compat_build_content_regex(
    query: &str,
    is_regex: bool,
    case_sensitive: bool,
    whole_word: bool,
) -> CompatResult<Regex> {
    let mut pattern = if is_regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    if whole_word {
        pattern = format!(r"\b(?:{pattern})\b");
    }
    let prefix = if case_sensitive { "" } else { "(?i)" };
    Regex::new(format!("{prefix}{pattern}").as_str())
        .map_err(|error| compat_bad_request(format!("Invalid search pattern: {error}")))
}

fn compat_collect_content_matches(
    content: &str,
    regex: &Regex,
    max_matches: usize,
    context_chars: usize,
) -> (Vec<FsContentSearchMatchCompat>, bool) {
    let mut matches = Vec::new();
    let mut line_start = 0usize;
    let mut truncated = false;

    for (line_index, line_text) in content.split('\n').enumerate() {
        let line_len = line_text.len();
        for capture in regex.find_iter(line_text) {
            if matches.len() >= max_matches {
                truncated = true;
                return (matches, truncated);
            }

            let before_full = &line_text[..capture.start()];
            let matched_full = &line_text[capture.start()..capture.end()];
            let after_full = &line_text[capture.end()..];

            let before = before_full
                .chars()
                .rev()
                .take(context_chars)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>();
            let after = after_full.chars().take(context_chars).collect::<String>();

            let start_column = before_full.chars().count() + 1;
            let end_column = start_column + matched_full.chars().count();
            let start_offset = line_start + capture.start();
            let end_offset = line_start + capture.end();

            matches.push(FsContentSearchMatchCompat {
                line: line_index + 1,
                start_column,
                end_column,
                start_offset,
                end_offset,
                before,
                matched: matched_full.to_string(),
                after,
            });
        }
        line_start += line_len + 1;
    }

    (matches, truncated)
}

fn compat_run_git(dir: &Path, args: &[&str]) -> Result<(i32, String, String), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    Ok((
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    ))
}

fn compat_run_git_with_input(
    dir: &Path,
    args: &[&str],
    input: &str,
) -> Result<(i32, String, String), String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|error| error.to_string())?;
    }

    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    Ok((
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    ))
}

async fn compat_git_repo_root(directory: &Path) -> CompatResult<PathBuf> {
    let (code, stdout, stderr) = compat_run_git(directory, &["rev-parse", "--show-toplevel"])
        .map_err(compat_internal)?;
    if code != 0 {
        let message = stderr.trim();
        return Err(compat_bad_request(if message.is_empty() {
            "Not a git repository".to_string()
        } else {
            message.to_string()
        }));
    }
    Ok(compat_resolve_path(stdout.trim()))
}

async fn compat_git_require_file_path(
    directory: Option<&str>,
    path: Option<&str>,
) -> CompatResult<(PathBuf, PathBuf, String)> {
    let dir = compat_require_directory(directory).await?;
    let repo_root = compat_git_repo_root(&dir).await?;
    let relative = path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("path parameter is required"))?;
    if !compat_is_safe_repo_rel_path(relative) {
        return Err(compat_bad_request("Invalid path"));
    }
    let absolute = repo_root.join(relative).clean();
    if !absolute.starts_with(&repo_root) {
        return Err(compat_bad_request("Invalid path"));
    }
    Ok((dir, repo_root, relative.replace('\\', "/")))
}

fn compat_parse_hunk_header(header: &str) -> Option<(usize, usize, usize, usize)> {
    let captures = DIFF_HUNK_HEADER_RE.captures(header)?;
    let old_start = captures.get(1)?.as_str().parse::<usize>().ok()?;
    let old_count = captures
        .get(2)
        .and_then(|value| value.as_str().parse::<usize>().ok())
        .unwrap_or(1);
    let new_start = captures.get(3)?.as_str().parse::<usize>().ok()?;
    let new_count = captures
        .get(4)
        .and_then(|value| value.as_str().parse::<usize>().ok())
        .unwrap_or(1);
    Some((old_start, old_count, new_start, new_count))
}

fn compat_compute_hunk_anchor_line(new_start: usize, old_start: usize, lines: &[String]) -> usize {
    let mut next_new_line = new_start.max(1);
    for line in lines {
        let prefix = line.chars().next().unwrap_or_default();
        if prefix == ' ' {
            next_new_line += 1;
            continue;
        }
        if prefix == '+' || prefix == '-' {
            return next_new_line.max(1);
        }
    }
    new_start.max(old_start).max(1)
}

fn compat_parse_diff_meta(diff: &str) -> GitDiffMetaCompat {
    let mut lines: Vec<String> = diff.lines().map(|line| line.to_string()).collect();
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }

    let mut file_header = Vec::<String>::new();
    let mut hunks = Vec::<GitDiffHunkMetaCompat>::new();
    let mut current_header = String::new();
    let mut current_lines: Vec<String> = Vec::new();

    let push_hunk = |file_header: &[String],
                     hunks: &mut Vec<GitDiffHunkMetaCompat>,
                     header: &str,
                     lines: &[String]| {
        if header.is_empty() {
            return;
        }
        let (old_start, old_count, new_start, new_count) =
            compat_parse_hunk_header(header).unwrap_or((0, 0, 0, 0));
        let additions = lines
            .iter()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .count();
        let deletions = lines
            .iter()
            .filter(|line| line.starts_with('-') && !line.starts_with("---"))
            .count();
        let patch_ready = file_header
            .iter()
            .any(|line| line.starts_with("diff --git ") || line.starts_with("--- "));
        let patch = if patch_ready {
            let mut patch = String::new();
            for line in file_header {
                patch.push_str(line);
                patch.push('\n');
            }
            patch.push_str(header);
            patch.push('\n');
            for line in lines {
                patch.push_str(line);
                patch.push('\n');
            }
            patch
        } else {
            String::new()
        };
        hunks.push(GitDiffHunkMetaCompat {
            id: (hunks.len() + 1).to_string(),
            header: header.to_string(),
            range: format!("-{old_start},{old_count} +{new_start},{new_count}"),
            old_start,
            old_count,
            new_start,
            new_count,
            additions,
            deletions,
            anchor_line: compat_compute_hunk_anchor_line(new_start, old_start, lines),
            lines: lines.to_vec(),
            patch,
            patch_ready,
        });
    };

    for line in lines {
        if line.starts_with("diff --git ") {
            if !current_header.is_empty() {
                push_hunk(&file_header, &mut hunks, &current_header, &current_lines);
                current_header.clear();
                current_lines.clear();
            }
            file_header.clear();
            file_header.push(line);
            continue;
        }

        if line.starts_with("@@") {
            if !current_header.is_empty() {
                push_hunk(&file_header, &mut hunks, &current_header, &current_lines);
                current_lines.clear();
            }
            current_header = line;
            continue;
        }

        if current_header.is_empty() {
            file_header.push(line);
        } else {
            current_lines.push(line);
        }
    }

    if !current_header.is_empty() {
        push_hunk(&file_header, &mut hunks, &current_header, &current_lines);
    }

    let changed_lines = hunks
        .iter()
        .map(|hunk| hunk.additions + hunk.deletions)
        .sum::<usize>();

    GitDiffMetaCompat {
        file_header: file_header.clone(),
        has_patch_header: file_header
            .iter()
            .any(|line| line.starts_with("diff --git ") || line.starts_with("--- ")),
        summary: GitDiffSummaryCompat {
            files: usize::from(!file_header.is_empty() || !hunks.is_empty()),
            hunks: hunks.len(),
            changed_lines,
        },
        hunks,
    }
}

fn compat_worktree_signature(status: &str) -> String {
    let mut hash: u64 = 1469598103934665603;
    for byte in status.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{hash:016x}")
}

fn compat_git_text_from_spec(repo_root: &Path, spec: &str) -> String {
    match compat_run_git(repo_root, &["show", spec]) {
        Ok((0, stdout, _)) => stdout,
        _ => String::new(),
    }
}

fn compat_fuzzy_match_score_normalized(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let c = candidate.to_ascii_lowercase();

    if let Some(idx) = c.find(query) {
        let bonus = if idx == 0 {
            20
        } else {
            let prev = c.as_bytes()[idx.saturating_sub(1)] as char;
            if prev == '/' || prev == '_' || prev == '-' || prev == '.' || prev == ' ' {
                15
            } else {
                0
            }
        };
        let score = 100 + bonus - (idx.min(20) as i32) - ((c.len() / 5) as i32);
        return Some(score);
    }

    let mut score: i32 = 0;
    let mut last_index: i32 = -1;
    let mut consecutive: i32 = 0;

    for ch in query.chars() {
        if ch == ' ' {
            continue;
        }
        let start = (last_index + 1).max(0) as usize;
        let idx = match c[start..].find(ch) {
            Some(pos) => (start + pos) as i32,
            None => return None,
        };

        let gap = idx - last_index - 1;
        if gap == 0 {
            consecutive += 1;
        } else {
            consecutive = 0;
        }

        score += 10;
        score += (18 - idx).max(0);
        score -= gap.min(10);

        if idx == 0 {
            score += 12;
        } else {
            let prev = c.as_bytes()[(idx - 1) as usize] as char;
            if prev == '/' || prev == '_' || prev == '-' || prev == '.' || prev == ' ' {
                score += 10;
            }
        }

        if consecutive > 0 {
            score += 12;
        }

        last_index = idx;
    }

    score += (24 - (c.len() as i32 / 3)).max(0);
    Some(score)
}

async fn compat_fs_read_file_text(path: &Path) -> CompatResult<String> {
    tokio::fs::read_to_string(path)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => compat_not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => compat_forbidden("Access to file denied"),
            std::io::ErrorKind::InvalidData => {
                compat_bad_request("Specified file is not UTF-8 text")
            }
            _ => compat_internal(error.to_string()),
        })
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

async fn compat_fs_read(Query(query): Query<FsFileCompatQuery>) -> CompatResult<Response> {
    let path =
        compat_resolve_scoped_file(query.directory.as_deref(), query.path.as_deref()).await?;
    let content = compat_fs_read_file_text(&path).await?;

    Response::builder()
        .status(StatusCode::OK)
        .header("cache-control", "no-store")
        .header("content-type", "text/plain")
        .body(Body::from(content))
        .map_err(|error| compat_internal(error.to_string()))
}

async fn compat_fs_read_chunk(
    Query(query): Query<FsReadChunkCompatQuery>,
) -> CompatResult<Json<FsReadChunkCompatResponse>> {
    let path =
        compat_resolve_scoped_file(query.directory.as_deref(), query.path.as_deref()).await?;

    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => compat_not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => compat_forbidden("Access to file denied"),
            _ => compat_internal("Failed to read file"),
        })?;

    let total_bytes_u64 = metadata.len();
    let total_bytes = usize::try_from(total_bytes_u64).unwrap_or(usize::MAX);
    let offset = query.offset.unwrap_or(0);
    if (offset as u64) > total_bytes_u64 {
        return Err(compat_bad_request("Offset is out of range"));
    }

    let limit = query
        .limit
        .unwrap_or(DEFAULT_COMPAT_READ_CHUNK_LIMIT)
        .min(MAX_COMPAT_READ_CHUNK_LIMIT);

    if limit == 0 {
        return Ok(Json(FsReadChunkCompatResponse {
            path: compat_path_string(&path),
            content: String::new(),
            offset,
            limit,
            loaded_bytes: offset,
            total_bytes,
            has_more: offset < total_bytes,
            next_offset: (offset < total_bytes).then_some(offset),
        }));
    }

    let mut file = tokio::fs::File::open(&path)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => compat_not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => compat_forbidden("Access to file denied"),
            _ => compat_internal(error.to_string()),
        })?;

    file.seek(SeekFrom::Start(offset as u64))
        .await
        .map_err(|error| compat_internal(error.to_string()))?;

    let mut buffer = Vec::with_capacity(limit);
    file.take(limit as u64)
        .read_to_end(&mut buffer)
        .await
        .map_err(|error| compat_internal(error.to_string()))?;

    let (content, consumed_bytes) = compat_decode_utf8_chunk(&buffer)?;
    let loaded_bytes = offset.saturating_add(consumed_bytes);
    let has_more = (loaded_bytes as u64) < total_bytes_u64;

    Ok(Json(FsReadChunkCompatResponse {
        path: compat_path_string(&path),
        content,
        offset,
        limit,
        loaded_bytes,
        total_bytes,
        has_more,
        next_offset: has_more.then_some(loaded_bytes),
    }))
}

async fn compat_fs_write(
    Query(query): Query<FsWriteCompatQuery>,
    Json(body): Json<FsWriteCompatBody>,
) -> CompatResult<Json<FsWriteCompatResponse>> {
    let path = body
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("Path is required"))?;
    let content = body
        .content
        .ok_or_else(|| compat_bad_request("Content is required"))?;

    if content.len() as u64 > MAX_COMPAT_FILE_BYTES {
        return Err(compat_payload_too_large("Content too large"));
    }

    let (_base, absolute) =
        compat_resolve_scoped_path(query.directory.as_deref(), Some(path)).await?;

    if let Some(parent) = absolute.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::PermissionDenied => compat_forbidden("Access denied"),
                _ => compat_internal(error.to_string()),
            })?;
    }

    tokio::fs::write(&absolute, content)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::PermissionDenied => compat_forbidden("Access denied"),
            _ => compat_internal(error.to_string()),
        })?;

    Ok(Json(FsWriteCompatResponse { success: true }))
}

async fn compat_fs_mkdir(
    Query(query): Query<FsWriteCompatQuery>,
    Json(body): Json<FsMkdirCompatBody>,
) -> CompatResult<Json<FsWriteCompatResponse>> {
    let path = body
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("Path is required"))?;

    let (_base, absolute) =
        compat_resolve_scoped_path(query.directory.as_deref(), Some(path)).await?;

    tokio::fs::create_dir_all(&absolute)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::PermissionDenied => compat_forbidden("Access denied"),
            _ => compat_internal(error.to_string()),
        })?;

    Ok(Json(FsWriteCompatResponse { success: true }))
}

async fn compat_fs_delete(
    Query(query): Query<FsWriteCompatQuery>,
    Json(body): Json<FsDeleteCompatBody>,
) -> CompatResult<Json<FsWriteCompatResponse>> {
    let path = body
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("Path is required"))?;

    let (_base, absolute) =
        compat_resolve_scoped_path(query.directory.as_deref(), Some(path)).await?;

    let metadata = match tokio::fs::symlink_metadata(&absolute).await {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(compat_forbidden("Access denied"));
        }
        Err(error) => return Err(compat_internal(error.to_string())),
    };

    if let Some(metadata) = metadata {
        let remove_result = if metadata.is_dir() {
            tokio::fs::remove_dir_all(&absolute).await
        } else {
            tokio::fs::remove_file(&absolute).await
        };
        remove_result.map_err(|error| match error.kind() {
            std::io::ErrorKind::PermissionDenied => compat_forbidden("Access denied"),
            _ => compat_internal(error.to_string()),
        })?;
    }

    Ok(Json(FsWriteCompatResponse { success: true }))
}

async fn compat_fs_rename(
    Query(query): Query<FsWriteCompatQuery>,
    Json(body): Json<FsRenameCompatBody>,
) -> CompatResult<Json<FsWriteCompatResponse>> {
    let old_path = body
        .old_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("oldPath is required"))?;
    let new_path = body
        .new_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("newPath is required"))?;

    let (_base, absolute_old) =
        compat_resolve_scoped_path(query.directory.as_deref(), Some(old_path)).await?;
    let (_base, absolute_new) =
        compat_resolve_scoped_path(query.directory.as_deref(), Some(new_path)).await?;

    tokio::fs::rename(&absolute_old, &absolute_new)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => compat_not_found("Source path not found"),
            std::io::ErrorKind::PermissionDenied => compat_forbidden("Access denied"),
            _ => compat_internal(error.to_string()),
        })?;

    Ok(Json(FsWriteCompatResponse { success: true }))
}

async fn compat_fs_search(
    Query(query): Query<FsSearchCompatQuery>,
) -> CompatResult<Json<FsSearchCompatResponse>> {
    let raw_root = query
        .root
        .or(query.directory)
        .unwrap_or_else(|| compat_default_list_root().display().to_string());
    let root = compat_validate_directory(&raw_root).await?;
    let raw_query = query.q.unwrap_or_default();
    let include_hidden = query.include_hidden.unwrap_or(false);
    let respect_gitignore = query.respect_gitignore.unwrap_or(true);
    let limit = query
        .limit
        .unwrap_or(DEFAULT_COMPAT_SEARCH_LIMIT)
        .clamp(1, MAX_COMPAT_SEARCH_LIMIT);

    let query_norm = raw_query.trim().to_ascii_lowercase();
    let match_all = query_norm.is_empty();
    let collect_limit = if match_all {
        limit
    } else {
        (limit * 3).max(200)
    };

    let excluded: HashSet<&'static str> =
        COMPAT_FILE_SEARCH_EXCLUDED_DIRS.iter().copied().collect();
    let started = Instant::now();
    let root_for_filter = root.clone();
    let mut builder = WalkBuilder::new(&root);
    builder.hidden(!include_hidden);
    if !respect_gitignore {
        builder.git_ignore(false);
        builder.git_global(false);
        builder.git_exclude(false);
        builder.parents(false);
    }
    builder.follow_links(false);

    let mut candidates: Vec<(FsSearchCompatFile, i32)> = Vec::new();

    for result in builder
        .filter_entry(move |entry| {
            let path = entry.path();
            if path == root_for_filter {
                return true;
            }

            let Some(name) = path.file_name().and_then(|segment| segment.to_str()) else {
                return true;
            };

            let lower = name.to_ascii_lowercase();
            if excluded.contains(lower.as_str()) {
                return false;
            }
            if !include_hidden && name.starts_with('.') {
                return false;
            }
            true
        })
        .build()
    {
        let entry = match result {
            Ok(entry) => entry,
            Err(_) => continue,
        };

        if !entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false)
        {
            continue;
        }

        let path = entry.path().to_path_buf();
        let name = path
            .file_name()
            .and_then(|segment| segment.to_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }

        let relative_path = compat_normalize_relative_search_path(&root, &path);
        let score = if match_all {
            0
        } else {
            match compat_fuzzy_match_score_normalized(&query_norm, &relative_path) {
                Some(score) => score,
                None => continue,
            }
        };

        candidates.push((
            FsSearchCompatFile {
                name,
                path: compat_path_string(&path),
                relative_path,
            },
            score,
        ));

        if candidates.len() >= collect_limit {
            break;
        }
    }

    if !match_all {
        candidates.sort_by(|(left, left_score), (right, right_score)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.relative_path.len().cmp(&right.relative_path.len()))
                .then_with(|| left.relative_path.cmp(&right.relative_path))
        });
    }

    let files = candidates
        .into_iter()
        .take(limit)
        .map(|(file, _)| file)
        .collect::<Vec<_>>();

    tracing::debug!(
        "compat_fs_search root={} q='{}' count={} elapsed_ms={}",
        root.display(),
        raw_query,
        files.len(),
        started.elapsed().as_millis()
    );

    Ok(Json(FsSearchCompatResponse {
        root: compat_path_string(&root),
        count: files.len(),
        files,
    }))
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

async fn compat_fs_search_content(
    Query(query): Query<FsWriteCompatQuery>,
    Json(body): Json<FsContentSearchCompatBody>,
) -> CompatResult<Json<FsContentSearchResponseCompat>> {
    let root = compat_require_directory(query.directory.as_deref()).await?;
    let raw_query = body
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("Search query is required"))?;

    let include_hidden = body.include_hidden.unwrap_or(false);
    let respect_gitignore = body.respect_gitignore.unwrap_or(true);
    let is_regex = body.is_regex.unwrap_or(false);
    let case_sensitive = body.case_sensitive.unwrap_or(false);
    let whole_word = body.whole_word.unwrap_or(false);
    let max_results = body
        .max_results
        .unwrap_or(DEFAULT_CONTENT_SEARCH_MAX_RESULTS)
        .clamp(1, MAX_CONTENT_SEARCH_MAX_RESULTS);
    let max_matches_per_file = body
        .max_matches_per_file
        .unwrap_or(DEFAULT_CONTENT_SEARCH_MAX_MATCHES_PER_FILE)
        .clamp(1, MAX_CONTENT_SEARCH_MAX_MATCHES_PER_FILE);
    let context_chars = body
        .context_chars
        .unwrap_or(DEFAULT_CONTENT_SEARCH_CONTEXT_CHARS)
        .clamp(0, MAX_CONTENT_SEARCH_CONTEXT_CHARS);

    let regex =
        compat_build_content_regex(raw_query, is_regex, case_sensitive, whole_word)?;
    let candidates = if let Some(paths) = body.paths.as_deref() {
        compat_normalize_scope_paths(&root, paths, include_hidden, respect_gitignore).await?
    } else {
        compat_walk_workspace_files(&root, include_hidden, respect_gitignore, MAX_CONTENT_SCOPE_PATHS)
    };

    let mut files = Vec::new();
    let mut total_matches = 0usize;
    let mut truncated = false;

    for path in candidates {
        if total_matches >= max_results {
            truncated = true;
            break;
        }

        let Some(content) = compat_read_searchable_text(&path).await else {
            continue;
        };

        let remaining = max_results.saturating_sub(total_matches);
        let max_for_file = max_matches_per_file.min(remaining);
        if max_for_file == 0 {
            truncated = true;
            break;
        }

        let (matches, file_truncated) =
            compat_collect_content_matches(&content, &regex, max_for_file, context_chars);
        if matches.is_empty() {
            continue;
        }

        total_matches += matches.len();
        truncated |= file_truncated;
        files.push(FsContentSearchFileCompat {
            path: compat_path_string(&path),
            relative_path: compat_normalize_relative_search_path(&root, &path),
            match_count: matches.len(),
            matches,
        });
    }

    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    Ok(Json(FsContentSearchResponseCompat {
        root: compat_path_string(&root),
        query: raw_query.to_string(),
        file_count: files.len(),
        match_count: total_matches,
        files,
        truncated,
    }))
}

async fn compat_fs_replace_content(
    Query(query): Query<FsWriteCompatQuery>,
    Json(body): Json<FsContentReplaceCompatBody>,
) -> CompatResult<Json<FsContentReplaceResponseCompat>> {
    let root = compat_require_directory(query.directory.as_deref()).await?;
    let replacement = body
        .replace
        .clone()
        .ok_or_else(|| compat_bad_request("Replace text is required"))?;

    if let Some(target) = body.r#match.clone() {
        let path = target
            .path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| compat_bad_request("Match path is required"))?;
        let expected = target
            .expected
            .ok_or_else(|| compat_bad_request("Match expected text is required"))?;
        let start_offset = target
            .start_offset
            .ok_or_else(|| compat_bad_request("Match startOffset is required"))?;
        let end_offset = target
            .end_offset
            .ok_or_else(|| compat_bad_request("Match endOffset is required"))?;

        if end_offset <= start_offset {
            return Err(compat_bad_request("Invalid match range"));
        }

        let (_, absolute) = compat_resolve_scoped_path(Some(root.display().to_string().as_str()), Some(path)).await?;
        let Some(content) = compat_read_searchable_text(&absolute).await else {
            return Err(compat_bad_request("Target file is not a searchable text file"));
        };

        if end_offset > content.len()
            || !content.is_char_boundary(start_offset)
            || !content.is_char_boundary(end_offset)
        {
            return Err(compat_bad_request("Match range is no longer valid"));
        }

        let current = &content[start_offset..end_offset];
        if current != expected {
            return Err(compat_bad_request(
                "Selected match changed; run search again before replacing",
            ));
        }

        let mut updated =
            String::with_capacity(content.len() + replacement.len().saturating_sub(expected.len()));
        updated.push_str(&content[..start_offset]);
        updated.push_str(&replacement);
        updated.push_str(&content[end_offset..]);

        tokio::fs::write(&absolute, updated)
            .await
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::PermissionDenied => compat_forbidden("Access denied"),
                _ => compat_internal(error.to_string()),
            })?;

        return Ok(Json(FsContentReplaceResponseCompat {
            root: compat_path_string(&root),
            file_count: 1,
            replacement_count: 1,
            skipped: 0,
            files: vec![FsContentReplaceFileCompat {
                path: compat_path_string(&absolute),
                relative_path: compat_normalize_relative_search_path(&root, &absolute),
                replacements: 1,
            }],
        }));
    }

    let raw_query = body
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("Search query is required"))?;
    let include_hidden = body.include_hidden.unwrap_or(false);
    let respect_gitignore = body.respect_gitignore.unwrap_or(true);
    let is_regex = body.is_regex.unwrap_or(false);
    let case_sensitive = body.case_sensitive.unwrap_or(false);
    let whole_word = body.whole_word.unwrap_or(false);

    let regex =
        compat_build_content_regex(raw_query, is_regex, case_sensitive, whole_word)?;
    let candidates = if let Some(paths) = body.paths.as_deref() {
        compat_normalize_scope_paths(&root, paths, true, false).await?
    } else {
        compat_walk_workspace_files(&root, include_hidden, respect_gitignore, MAX_CONTENT_SCOPE_PATHS)
    };

    let mut files = Vec::new();
    let mut replacement_count = 0usize;
    let mut skipped = 0usize;

    for path in candidates {
        let Some(content) = compat_read_searchable_text(&path).await else {
            skipped += 1;
            continue;
        };
        let replacements = regex.find_iter(&content).count();
        if replacements == 0 {
            continue;
        }
        let updated = regex.replace_all(&content, replacement.as_str()).into_owned();
        if updated == content {
            continue;
        }
        if let Err(error) = tokio::fs::write(&path, updated).await {
            skipped += 1;
            tracing::warn!("compat_fs_replace_content failed to write {}: {}", path.display(), error);
            continue;
        }
        replacement_count += replacements;
        files.push(FsContentReplaceFileCompat {
            path: compat_path_string(&path),
            relative_path: compat_normalize_relative_search_path(&root, &path),
            replacements,
        });
    }

    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    Ok(Json(FsContentReplaceResponseCompat {
        root: compat_path_string(&root),
        file_count: files.len(),
        replacement_count,
        skipped,
        files,
    }))
}

fn compat_fs_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/fs/home", get(compat_fs_home))
        .route("/api/fs/list", get(compat_fs_list))
        .route("/api/fs/search", get(compat_fs_search))
        .route("/api/fs/search-content", post(compat_fs_search_content))
        .route("/api/fs/replace-content", post(compat_fs_replace_content))
        .route("/api/fs/read", get(compat_fs_read))
        .route("/api/fs/read-chunk", get(compat_fs_read_chunk))
        .route("/api/fs/write", post(compat_fs_write))
        .route("/api/fs/mkdir", post(compat_fs_mkdir))
        .route("/api/fs/delete", post(compat_fs_delete))
        .route("/api/fs/rename", post(compat_fs_rename))
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

async fn compat_git_blame(
    Query(query): Query<GitPathCompatQuery>,
) -> CompatResult<Json<GitBlameResponseCompat>> {
    let (_dir, repo_root, relative) =
        compat_git_require_file_path(query.directory.as_deref(), query.path.as_deref()).await?;
    let (code, stdout, stderr) = compat_run_git(
        &repo_root,
        &["blame", "--line-porcelain", "--", relative.as_str()],
    )
    .map_err(compat_internal)?;
    if code != 0 {
        let message = stderr.trim();
        return Err(compat_bad_request(if message.is_empty() {
            "git blame failed".to_string()
        } else {
            message.to_string()
        }));
    }

    let mut lines = Vec::<GitBlameLineCompat>::new();
    let mut current_hash: Option<String> = None;
    let mut current_line = 0usize;
    let mut author = String::new();
    let mut author_email = String::new();
    let mut author_time = 0u64;
    let mut summary = String::new();

    for line in stdout.lines() {
        if line.starts_with('\t') {
            if let Some(hash) = current_hash.as_ref()
                && current_line > 0
            {
                lines.push(GitBlameLineCompat {
                    line: current_line,
                    hash: hash.clone(),
                    author: author.clone(),
                    author_email: author_email.clone(),
                    author_time,
                    summary: summary.clone(),
                });
            }
            current_hash = None;
            current_line = 0;
            author.clear();
            author_email.clear();
            author_time = 0;
            summary.clear();
            continue;
        }

        if let Some((hash, rest)) = line.split_once(' ')
            && hash.len() == 40
            && hash.chars().all(|ch| ch.is_ascii_hexdigit())
        {
            let mut fields = rest.split_whitespace();
            let _source_line = fields.next();
            current_line = fields
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            current_hash = Some(hash.to_string());
            continue;
        }
        if let Some(value) = line.strip_prefix("author ") {
            author = value.to_string();
        } else if let Some(value) = line.strip_prefix("author-mail ") {
            author_email = value.trim_matches(&['<', '>'][..]).to_string();
        } else if let Some(value) = line.strip_prefix("author-time ") {
            author_time = value.parse::<u64>().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("summary ") {
            summary = value.to_string();
        }
    }

    Ok(Json(GitBlameResponseCompat { lines }))
}

async fn compat_git_diff(
    Query(query): Query<GitDiffCompatQuery>,
) -> CompatResult<Json<GitDiffResponseCompat>> {
    let (_dir, repo_root, relative) =
        compat_git_require_file_path(query.directory.as_deref(), query.path.as_deref()).await?;
    let context = query.context_lines.unwrap_or(3).clamp(0, 500);
    let staged = query.staged.unwrap_or(false);
    let mut args = vec!["diff".to_string()];
    if staged {
        args.push("--cached".to_string());
    }
    args.push(format!("-U{context}"));
    args.push("--".to_string());
    args.push(relative.clone());
    let args_ref = args.iter().map(String::as_str).collect::<Vec<_>>();

    let (code, stdout, stderr) = compat_run_git(&repo_root, &args_ref).map_err(compat_internal)?;
    if code != 0 {
        let message = stderr.trim();
        return Err(compat_bad_request(if message.is_empty() {
            "git diff failed".to_string()
        } else {
            message.to_string()
        }));
    }

    let meta = query.include_meta.unwrap_or(false).then(|| compat_parse_diff_meta(&stdout));
    Ok(Json(GitDiffResponseCompat { diff: stdout, meta }))
}

async fn compat_git_file_diff(
    Query(query): Query<GitFileDiffCompatQuery>,
) -> CompatResult<Json<GitFileDiffResponseCompat>> {
    let (_dir, repo_root, relative) =
        compat_git_require_file_path(query.directory.as_deref(), query.path.as_deref()).await?;
    let staged = query.staged.unwrap_or(false);
    let original = if staged {
        compat_git_text_from_spec(&repo_root, format!("HEAD:{relative}").as_str())
    } else {
        compat_git_text_from_spec(&repo_root, format!(":{relative}").as_str())
    };
    let modified = if staged {
        compat_git_text_from_spec(&repo_root, format!(":{relative}").as_str())
    } else {
        let absolute = repo_root.join(&relative);
        tokio::fs::read_to_string(&absolute).await.unwrap_or_default()
    };

    Ok(Json(GitFileDiffResponseCompat { original, modified }))
}

async fn compat_git_patch(
    Query(query): Query<FsWriteCompatQuery>,
    Json(body): Json<GitPatchCompatBody>,
) -> CompatResult<Json<FsWriteCompatResponse>> {
    let repo_root = compat_require_directory(query.directory.as_deref()).await?;
    let patch = body
        .patch
        .as_deref()
        .map(str::trim_end)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("patch is required"))?;
    let mode = body
        .mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("mode is required"))?;

    let mut args = vec!["apply"];
    match mode {
        "stage" => args.push("--cached"),
        "unstage" => {
            args.push("--cached");
            args.push("--reverse");
        }
        "discard" => args.push("--reverse"),
        _ => return Err(compat_bad_request("Unsupported patch mode")),
    }

    let patch = if patch.ends_with('\n') {
        patch.to_string()
    } else {
        format!("{patch}\n")
    };

    let (code, _stdout, stderr) =
        compat_run_git_with_input(&repo_root, &args, patch.as_str()).map_err(compat_internal)?;
    if code != 0 {
        let message = stderr.trim();
        return Err(compat_bad_request(if message.is_empty() {
            "git apply failed".to_string()
        } else {
            message.to_string()
        }));
    }

    Ok(Json(FsWriteCompatResponse { success: true }))
}

async fn compat_git_watch(
    Query(query): Query<GitWatchCompatQuery>,
) -> CompatResult<Response> {
    let directory = compat_require_directory(query.directory.as_deref()).await?;
    let interval_ms = query.interval_ms.unwrap_or(1500).clamp(500, 10_000);

    let stream = stream! {
        let mut last: Option<GitWatchStatusPayloadCompat> = None;
        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
        loop {
            ticker.tick().await;

            let current = git_output(&directory, &["branch", "--show-current"]).unwrap_or_default();
            let tracking = git_output(
                &directory,
                &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
            )
            .and_then(|value| {
                let trimmed = value.trim();
                (!trimmed.is_empty()).then_some(trimmed.to_string())
            });
            let ahead_behind = tracking.as_ref().and_then(|_| {
                git_output(
                    &directory,
                    &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
                )
            });
            let (ahead, behind) = parse_ahead_behind(ahead_behind.as_deref());
            let status = git_output(&directory, &["status", "--porcelain"]).unwrap_or_default();
            let (staged_count, unstaged_count, untracked_count, total_files) =
                summarize_git_status(status.as_str());

            let payload = GitWatchStatusPayloadCompat {
                current,
                tracking,
                ahead,
                behind,
                staged_count,
                unstaged_count,
                untracked_count,
                merge_count: 0,
                is_clean: total_files == 0,
                worktree_signature: compat_worktree_signature(status.as_str()),
            };
            if last.as_ref().is_some_and(|previous| previous == &payload) {
                continue;
            }
            last = Some(payload.clone());
            let json = serde_json::to_string(&serde_json::json!({
                "type": "git.watch.status",
                "properties": payload,
            }))
            .unwrap_or_else(|_| "{}".to_string());
            yield Ok::<Event, Infallible>(Event::default().event("status").data(json));
        }
    };

    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("ping"))
        .into_response())
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
        .route("/api/git/watch", get(compat_git_watch))
        .route("/api/git/blame", get(compat_git_blame))
        .route("/api/git/diff", get(compat_git_diff))
        .route("/api/git/file-diff", get(compat_git_file_diff))
        .route("/api/git/patch", post(compat_git_patch))
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
            post(crate::terminal_sessions::terminal_create),
        )
        .route(
            "/api/terminal/{session_id}",
            get(crate::terminal_sessions::terminal_get)
                .delete(crate::terminal_sessions::terminal_delete),
        )
        .route(
            "/api/terminal/{session_id}/stream",
            get(crate::terminal_sessions::terminal_stream),
        )
        .route(
            "/api/terminal/{session_id}/input",
            post(crate::terminal_sessions::terminal_input),
        )
        .route(
            "/api/terminal/{session_id}/resize",
            post(crate::terminal_sessions::terminal_resize),
        )
        .route(
            "/api/terminal/{session_id}/start",
            post(crate::terminal_sessions::terminal_start),
        )
        .route(
            "/api/terminal/{session_id}/stop",
            post(crate::terminal_sessions::terminal_stop),
        )
        .route(
            "/api/terminal/{session_id}/restart",
            post(crate::terminal_sessions::terminal_restart),
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
    use serde_json::json;
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

    #[tokio::test]
    async fn compat_fs_read_route_returns_plain_text_for_scoped_file() {
        let temp = tempdir().expect("tempdir should be created");
        let file = temp.path().join("notes.txt");
        std::fs::write(&file, "hello studio").expect("file should be written");

        let directory_path = temp.path().display().to_string();
        let directory = urlencoding::encode(&directory_path);
        let read_uri = format!("/api/fs/read?directory={directory}&path=notes.txt");

        let response = compat_fs_router::<()>()
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

        let directory_path = temp.path().display().to_string();
        let directory = urlencoding::encode(&directory_path);
        let read_chunk_uri =
            format!("/api/fs/read-chunk?directory={directory}&path=notes.txt&offset=0&limit=5");

        let response = compat_fs_router::<()>()
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
        let payload: FsReadChunkCompatResponse =
            serde_json::from_slice(&body).expect("response should be valid json");
        assert_eq!(payload.path, compat_path_string(&file));
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

        let response = compat_fs_router::<()>()
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
        let payload: FsWriteCompatResponse =
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

        let response = compat_fs_router::<()>()
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
        let payload: FsWriteCompatResponse =
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

        let response = compat_fs_router::<()>()
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
        let payload: FsWriteCompatResponse =
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

        let first_response = compat_fs_router::<()>()
            .clone()
            .oneshot(request())
            .await
            .expect("request should succeed");
        assert_eq!(first_response.status(), StatusCode::OK);
        assert!(!temp.path().join("nested").exists());

        let second_response = compat_fs_router::<()>()
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
        let payload: FsWriteCompatResponse =
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
        let payload: FsSearchCompatResponse =
            serde_json::from_slice(&body).expect("response should be valid json");
        assert_eq!(payload.root, compat_path_string(temp.path()));
        assert_eq!(payload.count, 2);
        assert_eq!(payload.files[0].relative_path, "src/app.ts");
        assert_eq!(payload.files[1].relative_path, "src/app.test.ts");
        assert!(
            payload
                .files
                .iter()
                .all(|file| !file.path.contains("node_modules"))
        );
    }

    #[tokio::test]
    async fn compat_fs_content_search_and_replace_routes_work() {
        let temp = tempdir().expect("tempdir should be created");
        std::fs::create_dir_all(temp.path().join("src")).expect("src dir should exist");
        let file = temp.path().join("src/app.txt");
        std::fs::write(&file, "hello world\nhello studio\n").expect("file should be written");
        let directory_value = temp.path().display().to_string();
        let directory = urlencoding::encode(&directory_value);

        let search_response = compat_fs_router::<()>()
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
        let search_payload: FsContentSearchResponseCompat = serde_json::from_slice(
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

        let replace_response = compat_fs_router::<()>()
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
        let replace_payload: FsContentReplaceResponseCompat = serde_json::from_slice(
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
    async fn compat_git_blame_diff_patch_and_watch_routes_work() {
        let temp = tempdir().expect("tempdir should be created");
        let repo = temp.path();
        assert!(Command::new("git").arg("--version").output().is_ok(), "git is required for this test");
        Command::new("git")
            .arg("init")
            .arg(repo)
            .status()
            .expect("git init should run");
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["config", "user.name", "Agena Test"])
            .status()
            .expect("git config user.name should run");
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["config", "user.email", "test@example.com"])
            .status()
            .expect("git config user.email should run");

        let file = repo.join("notes.txt");
        std::fs::write(&file, "alpha\nbeta\n").expect("file should be written");
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["add", "notes.txt"])
            .status()
            .expect("git add should run");
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["commit", "-m", "init"])
            .status()
            .expect("git commit should run");

        std::fs::write(&file, "alpha\nbeta changed\n").expect("file should be rewritten");
        let directory_value = repo.display().to_string();
        let directory = urlencoding::encode(&directory_value);

        let blame_response = compat_git_blame(
            Query(GitPathCompatQuery {
                directory: Some(repo.display().to_string()),
                path: Some("notes.txt".to_string()),
            }),
        )
        .await
        .expect("blame should succeed");
        assert_eq!(blame_response.0.lines.len(), 2);

        let diff_response = compat_git_diff(
            Query(GitDiffCompatQuery {
                directory: Some(repo.display().to_string()),
                path: Some("notes.txt".to_string()),
                staged: Some(false),
                context_lines: Some(3),
                include_meta: Some(true),
            }),
        )
        .await
        .expect("diff should succeed");
        assert!(diff_response.0.diff.contains("beta changed"));
        assert!(diff_response.0.meta.as_ref().is_some_and(|meta| !meta.hunks.is_empty()));

        let file_diff_response = compat_git_file_diff(
            Query(GitFileDiffCompatQuery {
                directory: Some(repo.display().to_string()),
                path: Some("notes.txt".to_string()),
                staged: Some(false),
            }),
        )
        .await
        .expect("file diff should succeed");
        assert!(file_diff_response.0.original.contains("beta"));
        assert!(file_diff_response.0.modified.contains("beta changed"));

        let patch_response = compat_git_patch(
            Query(FsWriteCompatQuery {
                directory: Some(repo.display().to_string()),
            }),
            Json(GitPatchCompatBody {
                patch: Some(diff_response.0.diff.clone()),
                mode: Some("discard".to_string()),
            }),
        )
        .await
        .expect("patch should succeed");
        assert!(patch_response.0.success);
        assert_eq!(
            std::fs::read_to_string(&file).expect("file should stay readable"),
            "alpha\nbeta\n"
        );

        let watch_response = compat_git_watch(Query(GitWatchCompatQuery {
            directory: Some(repo.display().to_string()),
            interval_ms: Some(500),
        }))
        .await
        .expect("watch should succeed");
        assert_eq!(watch_response.status(), StatusCode::OK);
        assert_eq!(
            watch_response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/event-stream")
        );

        let route_watch_response = Router::new()
            .route("/api/git/watch", get(compat_git_watch))
            .oneshot(
                Request::builder()
                    .uri(format!("/api/git/watch?directory={directory}&intervalMs=500"))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(route_watch_response.status(), StatusCode::OK);
    }
}
