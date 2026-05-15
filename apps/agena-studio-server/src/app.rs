use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    env,
    io::Write as _,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use agena::config::{ConfigLoader, ProviderAuthConfig, provider_auth_data};
use agena::runtime::AgenaRuntime;
use agena::storage::StorageConfig;
use agena::tracing as tracing_config;
use agena_api_server::AppState as ApiV2State;
use agena_api_server::local_api::{
    SessionListQuery, SessionResource, WorkspaceListQuery, WorkspaceResource,
};
use anyhow::{Context, Result, anyhow};
use async_stream::stream;
use axum::{
    Json, Router,
    body::Body,
    extract::{Path as AxumPath, Query},
    http::{
        HeaderMap, HeaderValue, Method, StatusCode,
        header::{self, HeaderName},
    },
    middleware,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, patch, post},
};
use axum_extra::extract::cookie::SameSite;
use ignore::WalkBuilder;
use mime_guess::MimeGuess;
use path_clean::PathClean;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
    pub(crate) compat_api_service: agena_api_server::local_api::ApiService,
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
struct CompatConfigQuery {
    directory: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatConfigReloadResponse {
    success: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatProvidersResponse {
    providers: Vec<CompatProviderEntry>,
    default: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatProviderEntry {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    env: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    models: Vec<CompatProviderModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth: Option<CompatProviderAuthStatus>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatProviderAuthStatus {
    configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    credential_type: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatProviderSourceResponse {
    provider_id: String,
    sources: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatProviderSourceEntry {
    exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatProviderModel {
    id: String,
    provider_id: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GitStatusCompatQuery {
    directory: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
    scope: Option<String>,
    summary: Option<bool>,
    include_diff_stats: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    diff_stats: Option<HashMap<String, GitStatusCompatDiffStat>>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GitStatusCompatFile {
    path: String,
    index: String,
    working_dir: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
struct GitStatusCompatDiffStat {
    insertions: u64,
    deletions: u64,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CompatSessionListQuery {
    offset: Option<usize>,
    limit: Option<usize>,
    include_total: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CompatSessionStatusQuery {
    session_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CompatSessionMessagesQuery {
    offset: Option<usize>,
    limit: Option<usize>,
    include_total: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct CompatDirectoriesQuery {
    offset: Option<usize>,
    limit: Option<usize>,
    query: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CompatEnvCheckRequest {
    vars: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CompatEnvCheckResponse {
    present: Vec<String>,
    missing: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
struct CompatDirectoryEntry {
    id: String,
    path: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CompatPagedResponse<T> {
    items: Vec<T>,
    total: usize,
    offset: usize,
    limit: usize,
    has_more: bool,
    next_offset: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CompatSessionDiffQuery {
    directory: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
    #[serde(default, alias = "messageID")]
    message_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CompatSessionCreateBody {
    title: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CompatSessionPatchBody {
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CompatSessionForkBody {
    #[serde(rename = "messageID", alias = "messageId")]
    message_id: String,
}

#[derive(Debug, Deserialize)]
struct CompatSessionRevertBody {
    #[serde(rename = "messageID", alias = "messageId")]
    message_id: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CompatSessionSummarizeBody {
    #[serde(default, alias = "providerID")]
    provider_id: Option<String>,
    #[serde(default, alias = "modelID")]
    model_id: Option<String>,
    #[serde(default)]
    auto: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CompatSessionMessageModelBody {
    #[serde(default, alias = "providerID")]
    provider_id: Option<String>,
    #[serde(default, alias = "modelID")]
    model_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CompatSessionMessageBody {
    #[serde(default)]
    parts: Vec<Value>,
    #[serde(default)]
    model: Option<CompatSessionMessageModelBody>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    agent_profile: Option<String>,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    system: Option<String>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    max_turn_loops: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatSessionListResponse {
    sessions: Vec<Value>,
    total: usize,
    offset: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_offset: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatSessionMessageListResponse {
    entries: Vec<Value>,
    total: usize,
    offset: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<usize>,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_offset: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatSessionDiffListResponse {
    entries: Vec<Value>,
    total: usize,
    offset: usize,
    limit: usize,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_offset: Option<usize>,
}

const MAX_COMPAT_FILE_BYTES: u64 = 50 * 1024 * 1024;
const MAX_COMPAT_CONFLICT_FILE_BYTES: u64 = 512 * 1024;
const MAX_COMPAT_LIST_LIMIT: usize = 2000;
const DEFAULT_COMPAT_DIRECTORIES_LIMIT: usize = 50;
const MAX_COMPAT_DIRECTORIES_LIMIT: usize = 400;
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
struct GitCommitFileCompatQuery {
    directory: Option<String>,
    commit: Option<String>,
    path: Option<String>,
    context_lines: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
struct GitCommitFileContentCompatQuery {
    directory: Option<String>,
    commit: Option<String>,
    path: Option<String>,
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

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GitConflictResolveCompatBody {
    path: Option<String>,
    strategy: Option<String>,
    stage: Option<bool>,
    choices: Option<Vec<GitConflictResolveChoiceCompat>>,
}

#[derive(Debug, Deserialize, Clone, Default)]
struct GitConflictResolveChoiceCompat {
    id: Option<usize>,
    choice: Option<String>,
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
struct GitCommitFileDiffResponseCompat {
    diff: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GitCommitFileContentResponseCompat {
    content: String,
    exists: bool,
    binary: bool,
    truncated: bool,
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
struct GitConflictBlockCompat {
    id: usize,
    ours_label: Option<String>,
    base_label: Option<String>,
    theirs_label: Option<String>,
    ours: String,
    base: String,
    theirs: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct GitConflictFileResponseCompat {
    path: String,
    text: String,
    blocks: Vec<GitConflictBlockCompat>,
    has_markers: bool,
    is_unmerged: bool,
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

async fn compat_config_reload(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> CompatResult<Json<CompatConfigReloadResponse>> {
    state
        .runtime
        .reload()
        .await
        .map_err(|error| compat_internal(error.to_string()))?;

    Ok(Json(CompatConfigReloadResponse { success: true }))
}

fn compat_provider_env_names(auth: &ProviderAuthConfig) -> Vec<String> {
    match auth {
        ProviderAuthConfig::Api(config) => config
            .api_key_env
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| vec![value.to_owned()])
            .unwrap_or_default(),
        ProviderAuthConfig::SapAiCore(config) => config
            .api
            .api_key_env
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| vec![value.to_owned()])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn compat_provider_auth_status(
    resolved: &agena::config::ResolvedProviderConfig,
) -> Option<CompatProviderAuthStatus> {
    let auth = provider_auth_data(resolved);
    let credential_type = auth.as_ref().map(|data| match data {
        agena::provider::auth::AuthData::Api { .. } => "api",
        agena::provider::auth::AuthData::OAuth { .. } => "oauth",
        agena::provider::auth::AuthData::WellKnown { .. } => "well_known",
    });

    (credential_type.is_some() || !matches!(resolved.auth, ProviderAuthConfig::None)).then_some(
        CompatProviderAuthStatus {
            configured: auth.is_some(),
            credential_type,
        },
    )
}

fn compat_provider_model_id(model_ref: &str) -> String {
    model_ref
        .split_once('/')
        .map(|(_, model_id)| model_id)
        .unwrap_or(model_ref)
        .to_owned()
}

fn compat_provider_models(
    provider_id: &str,
    resolved: &agena::config::ResolvedProviderConfig,
) -> Vec<CompatProviderModel> {
    let mut ids = resolved
        .models
        .keys()
        .map(|id| compat_provider_model_id(id))
        .collect::<Vec<_>>();
    ids.push(compat_provider_model_id(resolved.default_model.as_str()));
    ids.sort();
    ids.dedup();

    ids.into_iter()
        .map(|id| CompatProviderModel {
            id,
            provider_id: provider_id.to_owned(),
        })
        .collect()
}

fn compat_has_value(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn compat_provider_layer_path(path: Option<&Path>) -> Option<String> {
    path.map(|path| path.display().to_string())
}

fn compat_provider_project_config_path(directory: &Path) -> PathBuf {
    directory.join(".agena").join("config.toml")
}

fn compat_provider_source_entry(exists: bool, path: Option<&Path>) -> CompatProviderSourceEntry {
    CompatProviderSourceEntry {
        exists,
        path: compat_provider_layer_path(path),
    }
}

fn compat_provider_auth_exists(resolved: &agena::config::ResolvedProviderConfig) -> bool {
    match &resolved.auth {
        ProviderAuthConfig::None => false,
        ProviderAuthConfig::Api(config) => {
            compat_has_value(config.api_key.as_deref()) || compat_has_value(config.api_key_env.as_deref())
        }
        ProviderAuthConfig::Credential(config) => config.credential.is_some(),
        ProviderAuthConfig::BedrockSigv4(config) => {
            compat_has_value(config.profile.as_deref())
                || compat_has_value(config.access_key_id.as_deref())
                || compat_has_value(config.secret_access_key.as_deref())
                || compat_has_value(config.session_token.as_deref())
        }
        ProviderAuthConfig::GoogleAdc(_) => true,
        ProviderAuthConfig::SapAiCore(config) => {
            compat_has_value(config.api.api_key.as_deref())
                || compat_has_value(config.api.api_key_env.as_deref())
                || compat_has_value(Some(config.service_key_env.as_str()))
        }
    }
}

fn compat_provider_sources_value(
    provider_exists: bool,
    auth_exists: bool,
    config_path: &Path,
    user_path: &Path,
    project_path: Option<&Path>,
) -> Value {
    let is_user = config_path == user_path;
    let is_project = project_path.is_some_and(|path| config_path == path);
    let is_custom = provider_exists && !is_user && !is_project;

    json!({
        "auth": compat_provider_source_entry(auth_exists, None),
        "user": compat_provider_source_entry(provider_exists && is_user, Some(user_path)),
        "project": compat_provider_source_entry(provider_exists && is_project, project_path),
        "custom": compat_provider_source_entry(
            is_custom,
            (!is_user && !is_project).then_some(config_path),
        ),
    })
}

async fn compat_config_providers(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Query(query): Query<CompatConfigQuery>,
) -> CompatResult<Json<CompatProvidersResponse>> {
    let _directory = query.directory;
    let snapshot = state.runtime.current_snapshot();
    let resolution = snapshot.config_resolution();
    let mut provider_ids = resolution
        .config
        .providers
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    provider_ids.sort();

    let mut providers = Vec::with_capacity(provider_ids.len());
    let mut default_models = HashMap::with_capacity(provider_ids.len());

    for provider_id in provider_ids {
        let Some(resolved) = resolution.config.providers.get(provider_id.as_str()) else {
            continue;
        };
        default_models.insert(
            provider_id.clone(),
            compat_provider_model_id(resolved.default_model.as_str()),
        );

        providers.push(CompatProviderEntry {
            id: provider_id.clone(),
            name: Some(provider_id.clone()),
            env: compat_provider_env_names(&resolved.auth),
            key: provider_auth_data(resolved)
                .as_ref()
                .and_then(|auth| auth.api_key().map(|_| "configured".to_owned())),
            models: compat_provider_models(provider_id.as_str(), resolved),
            auth: compat_provider_auth_status(resolved),
        });
    }

    Ok(Json(CompatProvidersResponse {
        providers,
        default: default_models,
    }))
}

async fn compat_provider_source(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<CompatConfigQuery>,
    AxumPath(provider_id): AxumPath<String>,
) -> CompatResult<Json<CompatProviderSourceResponse>> {
    let provider_id = provider_id.trim().to_owned();
    if provider_id.is_empty() {
        return Err(compat_bad_request("Provider ID is required"));
    }

    let requested_directory = headers
        .get("x-opencode-directory")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            query
                .directory
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        });
    let validated_directory = match requested_directory {
        Some(value) => Some(compat_validate_directory(&value).await?),
        None => None,
    };
    let snapshot = state.runtime.current_snapshot();
    let resolution = snapshot.config_resolution();
    let provider = resolution.config.providers.get(provider_id.as_str());
    let provider_exists = provider.is_some();
    let auth_exists = provider.map(compat_provider_auth_exists).unwrap_or(false);
    let user_path = ConfigLoader::default().default_config_path();
    let project_path = validated_directory
        .as_deref()
        .map(compat_provider_project_config_path);
    let sources = compat_provider_sources_value(
        provider_exists,
        auth_exists,
        resolution.meta.config_path.as_path(),
        user_path.as_path(),
        project_path.as_deref(),
    );

    Ok(Json(CompatProviderSourceResponse {
        provider_id,
        sources,
    }))
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

fn compat_parse_git_status_files(status: &str) -> Vec<GitStatusCompatFile> {
    let mut files = Vec::new();

    for line in status
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
    {
        if line.len() < 3 {
            continue;
        }

        let bytes = line.as_bytes();
        let x = bytes.first().copied().unwrap_or(b' ') as char;
        let y = bytes.get(1).copied().unwrap_or(b' ') as char;
        if x == '!' && y == '!' {
            continue;
        }

        let mut path = line.get(3..).unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }

        if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
            if let Some((_, renamed)) = path.rsplit_once(" -> ") {
                path = renamed.trim();
            }
        }

        let index = if x == ' ' {
            String::new()
        } else {
            x.to_string()
        };
        let working_dir = if y == ' ' {
            String::new()
        } else {
            y.to_string()
        };
        let (index, working_dir) = if x == '?' && y == '?' {
            ("?".to_string(), "?".to_string())
        } else {
            (index, working_dir)
        };
        if index.is_empty() && working_dir.is_empty() {
            continue;
        }

        files.push(GitStatusCompatFile {
            path: path.replace('\\', "/"),
            index,
            working_dir,
        });
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}

fn compat_git_status_is_merge(file: &GitStatusCompatFile) -> bool {
    file.index.trim() == "U" || file.working_dir.trim() == "U"
}

fn compat_git_status_is_untracked(file: &GitStatusCompatFile) -> bool {
    file.index.trim() == "?" && file.working_dir.trim() == "?"
}

fn compat_git_status_is_staged(file: &GitStatusCompatFile) -> bool {
    if compat_git_status_is_merge(file) {
        return false;
    }
    let index = file.index.trim();
    !index.is_empty() && index != "?"
}

fn compat_git_status_is_unstaged(file: &GitStatusCompatFile) -> bool {
    if compat_git_status_is_merge(file) || compat_git_status_is_untracked(file) {
        return false;
    }
    !file.working_dir.trim().is_empty()
}

fn compat_count_git_status_files(files: &[GitStatusCompatFile]) -> (u64, u64, u64, u64, u64) {
    let total_files = files.len() as u64;
    let staged_count = files
        .iter()
        .filter(|file| compat_git_status_is_staged(file))
        .count() as u64;
    let unstaged_count = files
        .iter()
        .filter(|file| compat_git_status_is_unstaged(file))
        .count() as u64;
    let untracked_count = files
        .iter()
        .filter(|file| compat_git_status_is_untracked(file))
        .count() as u64;
    let merge_count = files
        .iter()
        .filter(|file| compat_git_status_is_merge(file))
        .count() as u64;

    (
        staged_count,
        unstaged_count,
        untracked_count,
        merge_count,
        total_files,
    )
}

fn compat_parse_git_numstat(raw: &str, map: &mut HashMap<String, GitStatusCompatDiffStat>) {
    for line in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let parts = line.split('\t').collect::<Vec<_>>();
        if parts.len() < 3 {
            continue;
        }

        let insertions = if parts[0] == "-" {
            0
        } else {
            parts[0].parse::<u64>().unwrap_or(0)
        };
        let deletions = if parts[1] == "-" {
            0
        } else {
            parts[1].parse::<u64>().unwrap_or(0)
        };
        let path = parts[2..].join("\t");
        if path.is_empty() {
            continue;
        }

        let entry = map.entry(path).or_insert(GitStatusCompatDiffStat {
            insertions: 0,
            deletions: 0,
        });
        entry.insertions = entry.insertions.saturating_add(insertions);
        entry.deletions = entry.deletions.saturating_add(deletions);
    }
}

async fn compat_estimate_new_file_lines(
    repo_root: &Path,
    file_rel: &str,
) -> Option<GitStatusCompatDiffStat> {
    let absolute = repo_root.join(file_rel);
    let metadata = tokio::fs::metadata(&absolute).await.ok()?;
    if !metadata.is_file() || metadata.len() > MAX_COMPAT_FILE_BYTES {
        return None;
    }

    let bytes = tokio::fs::read(&absolute).await.ok()?;
    if bytes.contains(&0) {
        return Some(GitStatusCompatDiffStat {
            insertions: 0,
            deletions: 0,
        });
    }

    let content = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
    if content.is_empty() {
        return Some(GitStatusCompatDiffStat {
            insertions: 0,
            deletions: 0,
        });
    }

    let mut insertions = content.split('\n').count() as u64;
    if content.ends_with('\n') {
        insertions = insertions.saturating_sub(1);
    }

    Some(GitStatusCompatDiffStat {
        insertions,
        deletions: 0,
    })
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
            let nested = compat_walk_workspace_files(
                &path,
                include_hidden,
                respect_gitignore,
                MAX_CONTENT_SCOPE_PATHS,
            );
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
    let excluded: HashSet<&'static str> =
        COMPAT_FILE_SEARCH_EXCLUDED_DIRS.iter().copied().collect();
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

fn compat_run_git_bytes(dir: &Path, args: &[&str]) -> Result<(i32, Vec<u8>, String), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    Ok((
        output.status.code().unwrap_or(1),
        output.stdout,
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

    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    Ok((
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    ))
}

async fn compat_git_repo_root(directory: &Path) -> CompatResult<PathBuf> {
    let (code, stdout, stderr) =
        compat_run_git(directory, &["rev-parse", "--show-toplevel"]).map_err(compat_internal)?;
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

async fn compat_git_path_is_unmerged(repo_root: &Path, path: &str) -> bool {
    match compat_run_git(repo_root, &["ls-files", "-u", "--", path]) {
        Ok((0, stdout, _)) => stdout.lines().any(|line| !line.trim().is_empty()),
        _ => false,
    }
}

fn compat_parse_conflict_markers(text: &str) -> Vec<GitConflictBlockCompat> {
    let mut blocks = Vec::<GitConflictBlockCompat>::new();
    let mut state = 0;
    let mut ours = Vec::<String>::new();
    let mut base = Vec::<String>::new();
    let mut theirs = Vec::<String>::new();
    let mut ours_label: Option<String> = None;
    let mut base_label: Option<String> = None;
    let mut id = 0usize;

    for line in text.lines() {
        if line.starts_with("<<<<<<<") {
            state = 1;
            ours.clear();
            base.clear();
            theirs.clear();
            ours_label = line
                .strip_prefix("<<<<<<<")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            base_label = None;
            continue;
        }

        if state == 1 && line.starts_with("|||||||") {
            state = 2;
            base_label = line
                .strip_prefix("|||||||")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            continue;
        }

        if (state == 1 || state == 2) && line.starts_with("=======") {
            state = 3;
            continue;
        }

        if state == 3 && line.starts_with(">>>>>>>") {
            let theirs_label = line
                .strip_prefix(">>>>>>>")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            blocks.push(GitConflictBlockCompat {
                id,
                ours_label: ours_label.clone(),
                base_label: base_label.clone(),
                theirs_label,
                ours: ours.join("\n"),
                base: base.join("\n"),
                theirs: theirs.join("\n"),
            });
            id += 1;
            state = 0;
            continue;
        }

        match state {
            1 => ours.push(line.to_string()),
            2 => base.push(line.to_string()),
            3 => theirs.push(line.to_string()),
            _ => {}
        }
    }

    blocks
}

fn compat_apply_conflict_choices(
    text: &str,
    choices: &HashMap<usize, String>,
    default_choice: &str,
) -> String {
    let mut out = Vec::<String>::new();
    let mut state = 0;
    let mut ours = Vec::<String>::new();
    let mut base = Vec::<String>::new();
    let mut theirs = Vec::<String>::new();
    let mut id = 0usize;

    for line in text.lines() {
        if line.starts_with("<<<<<<<") {
            state = 1;
            ours.clear();
            base.clear();
            theirs.clear();
            continue;
        }

        if state == 1 && line.starts_with("|||||||") {
            state = 2;
            continue;
        }

        if (state == 1 || state == 2) && line.starts_with("=======") {
            state = 3;
            continue;
        }

        if state == 3 && line.starts_with(">>>>>>>") {
            match choices
                .get(&id)
                .map(String::as_str)
                .unwrap_or(default_choice)
            {
                "base" => out.extend(base.iter().cloned()),
                "theirs" => out.extend(theirs.iter().cloned()),
                "both" => {
                    out.extend(ours.iter().cloned());
                    out.extend(theirs.iter().cloned());
                }
                _ => out.extend(ours.iter().cloned()),
            }
            id += 1;
            state = 0;
            continue;
        }

        match state {
            0 => out.push(line.to_string()),
            1 => ours.push(line.to_string()),
            2 => base.push(line.to_string()),
            _ => theirs.push(line.to_string()),
        }
    }

    format!("{}\n", out.join("\n"))
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

    let regex = compat_build_content_regex(raw_query, is_regex, case_sensitive, whole_word)?;
    let candidates = if let Some(paths) = body.paths.as_deref() {
        compat_normalize_scope_paths(&root, paths, include_hidden, respect_gitignore).await?
    } else {
        compat_walk_workspace_files(
            &root,
            include_hidden,
            respect_gitignore,
            MAX_CONTENT_SCOPE_PATHS,
        )
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

        let (_, absolute) =
            compat_resolve_scoped_path(Some(root.display().to_string().as_str()), Some(path))
                .await?;
        let Some(content) = compat_read_searchable_text(&absolute).await else {
            return Err(compat_bad_request(
                "Target file is not a searchable text file",
            ));
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

    let regex = compat_build_content_regex(raw_query, is_regex, case_sensitive, whole_word)?;
    let candidates = if let Some(paths) = body.paths.as_deref() {
        compat_normalize_scope_paths(&root, paths, true, false).await?
    } else {
        compat_walk_workspace_files(
            &root,
            include_hidden,
            respect_gitignore,
            MAX_CONTENT_SCOPE_PATHS,
        )
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
        let updated = regex
            .replace_all(&content, replacement.as_str())
            .into_owned();
        if updated == content {
            continue;
        }
        if let Err(error) = tokio::fs::write(&path, updated).await {
            skipped += 1;
            tracing::warn!(
                "compat_fs_replace_content failed to write {}: {}",
                path.display(),
                error
            );
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

fn compat_session_manager(
    state: &AppState,
) -> Result<Arc<agena::session::SessionManager>, (StatusCode, String)> {
    state
        .runtime
        .session_manager()
        .ok_or_else(|| compat_internal("Session runtime is unavailable"))
}

async fn compat_list_all_workspaces(state: &AppState) -> CompatResult<Vec<WorkspaceResource>> {
    let mut cursor = None;
    let mut items = Vec::<WorkspaceResource>::new();

    loop {
        let page = state
            .compat_api_service
            .list_workspaces(WorkspaceListQuery {
                cursor: cursor.take(),
                limit: Some(200),
                search: None,
                include_session_count: false,
            })
            .await
            .map_err(|error| compat_internal(format!("{error:?}")))?;
        let next = page.page.next_cursor.clone();
        let has_more = page.page.has_more;
        items.extend(page.items);
        if !has_more {
            break;
        }
        cursor = next;
    }

    Ok(items)
}

async fn compat_list_all_sessions(state: &AppState) -> CompatResult<Vec<SessionResource>> {
    let mut cursor = None;
    let mut items = Vec::<SessionResource>::new();

    loop {
        let page = state
            .compat_api_service
            .list_sessions(SessionListQuery {
                cursor: cursor.take(),
                limit: Some(200),
                workspace_id: None,
                parent_id: None,
                roots: false,
                search: None,
            })
            .await
            .map_err(|error| compat_internal(format!("{error:?}")))?;
        let next = page.page.next_cursor.clone();
        let has_more = page.page.has_more;
        items.extend(page.items);
        if !has_more {
            break;
        }
        cursor = next;
    }

    Ok(items)
}

fn compat_normalize_env_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 80 {
        return None;
    }
    trimmed
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        .then(|| trimmed.to_string())
}

fn compat_workspace_directory_entry(workspace: &WorkspaceResource) -> CompatDirectoryEntry {
    CompatDirectoryEntry {
        id: workspace.id.to_string(),
        path: workspace.path.clone(),
    }
}

fn compat_page<T>(items: Vec<T>, offset: usize, limit: usize) -> CompatPagedResponse<T> {
    let total = items.len();
    let limit = limit.max(1);
    let offset = offset.min(total);
    let mut tail = items.into_iter().skip(offset);
    let page_items = tail.by_ref().take(limit).collect::<Vec<_>>();
    let has_more = offset + page_items.len() < total;

    CompatPagedResponse {
        items: page_items,
        total,
        offset,
        limit,
        has_more,
        next_offset: has_more.then_some(offset + limit),
    }
}

fn compat_parse_id(value: &str, label: &str) -> Result<i64, (StatusCode, String)> {
    value
        .trim()
        .parse::<i64>()
        .map_err(|_| compat_bad_request(format!("{label} must be a non-empty numeric identifier")))
}

fn compat_session_summary_value(summary: &agena::session::SessionSummary) -> Value {
    json!({
        "id": summary.id.to_string(),
        "parentID": summary.parent_id.map(|value| value.to_string()),
        "rootID": summary.root_id.to_string(),
        "title": summary.title.clone(),
        "messageCount": summary.message_count,
        "childSessionCount": summary.child_session_count,
        "goal": summary.goal.as_ref().map(|goal| json!({
            "id": goal.id.to_string(),
            "objective": goal.objective.clone(),
            "status": goal.status,
        })),
        "time": {
            "created": summary.created_at.timestamp_millis(),
            "updated": summary.updated_at.timestamp_millis(),
        }
    })
}

fn compat_session_resource_value(resource: &agena_api_server::local_api::SessionResource) -> Value {
    json!({
        "id": resource.id.to_string(),
        "parentID": resource.parent_id.map(|value| value.to_string()),
        "rootID": resource.root_id.to_string(),
        "title": resource.title.clone(),
        "messageCount": resource.message_count,
        "childSessionCount": resource.child_session_count,
        "goal": resource.goal.as_ref().map(|goal| json!({
            "id": goal.id.to_string(),
            "objective": goal.objective.clone(),
            "status": goal.status,
        })),
        "time": {
            "created": resource.created_at.timestamp_millis(),
            "updated": resource.updated_at.timestamp_millis(),
        }
    })
}

async fn compat_require_session(
    state: &AppState,
    session_id: i64,
) -> Result<(), (StatusCode, String)> {
    state
        .compat_api_service
        .get_session(session_id)
        .await
        .map_err(|_| compat_internal("Failed to load session"))?
        .ok_or_else(|| compat_not_found("session not found"))?;
    Ok(())
}

async fn compat_require_projected_message(
    manager: &agena::session::SessionManager,
    session_id: i64,
    message_id: i64,
) -> Result<(), (StatusCode, String)> {
    manager
        .find_projected_message(session_id, message_id, false)
        .await
        .map_err(|error| compat_internal(error.to_string()))?
        .ok_or_else(|| compat_not_found("message not found"))?;
    Ok(())
}

async fn compat_latest_rewind_target(
    manager: &agena::session::SessionManager,
    session_id: i64,
) -> Result<Option<i64>, (StatusCode, String)> {
    let checkpoints = manager
        .list_rewind_checkpoints(session_id)
        .await
        .map_err(|error| compat_internal(error.to_string()))?;
    Ok(checkpoints
        .last()
        .map(|checkpoint| checkpoint.target_message_id))
}

fn compat_session_share_url(session_id: i64) -> String {
    format!("/chat?session={session_id}")
}

fn compat_session_diff_kind(kind: agena::message::FileChangeKind) -> &'static str {
    match kind {
        agena::message::FileChangeKind::Added => "added",
        agena::message::FileChangeKind::Updated => "updated",
        agena::message::FileChangeKind::Deleted => "deleted",
        agena::message::FileChangeKind::Moved => "moved",
    }
}

fn compat_normalize_session_diff_path(path: &str) -> String {
    path.trim().replace('\\', "/")
}

fn compat_count_patch_changes(diff: Option<&str>) -> (usize, usize) {
    let mut additions = 0_usize;
    let mut deletions = 0_usize;
    let Some(diff) = diff else {
        return (0, 0);
    };

    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            additions = additions.saturating_add(1);
        } else if line.starts_with('-') {
            deletions = deletions.saturating_add(1);
        }
    }

    (additions, deletions)
}

fn compat_push_session_diff_entry(
    entries: &mut Vec<Value>,
    session_id: i64,
    message: &agena::message::Message,
    part: &agena::message::MessagePart,
    entry_index: usize,
    path: String,
    from_path: Option<String>,
    kind: &'static str,
    diff: Option<String>,
) {
    let (additions, deletions) = compat_count_patch_changes(diff.as_deref());
    entries.push(json!({
        "id": format!("{}:{}:{entry_index}", message.id, part.id),
        "sessionId": session_id.to_string(),
        "sessionID": session_id.to_string(),
        "messageId": message.id.to_string(),
        "messageID": message.id.to_string(),
        "partId": part.id.to_string(),
        "partID": part.id.to_string(),
        "operationId": part.operation_id.clone(),
        "file": path.clone(),
        "path": path,
        "fromPath": from_path,
        "kind": kind,
        "patch": diff.clone(),
        "diff": diff,
        "additions": additions,
        "deletions": deletions,
        "summary": part.summary.clone(),
        "time": {
            "created": message.created_at.timestamp_millis(),
        }
    }));
}

fn compat_collect_session_diff_entries(
    session_id: i64,
    messages: &[agena::message::Message],
) -> Vec<Value> {
    let mut entries = Vec::new();

    for message in messages {
        let has_apply_patch_part = message.parts.iter().any(|part| {
            matches!(
                part.content.as_ref(),
                Some(agena::message::PartContent::ToolExecution(
                    agena::message::ToolExecutionPart::Completed { details, .. }
                    | agena::message::ToolExecutionPart::Failed { details, .. }
                )) if matches!(
                    details.as_first_party(),
                    Some(agena::message::FirstPartyToolOutput::ApplyPatch { .. })
                )
            )
        });

        for part in &message.parts {
            match part.content.as_ref() {
                Some(agena::message::PartContent::ToolExecution(
                    agena::message::ToolExecutionPart::Completed { details, .. }
                    | agena::message::ToolExecutionPart::Failed { details, .. },
                )) => {
                    if let Some(agena::message::FirstPartyToolOutput::ApplyPatch {
                        changes,
                        diff,
                        ..
                    }) = details.as_first_party()
                    {
                        let diff = compat_trimmed_text(Some(diff.as_str()));
                        for (entry_index, change) in changes.iter().enumerate() {
                            let path = compat_normalize_session_diff_path(&change.path);
                            if path.is_empty() {
                                continue;
                            }
                            compat_push_session_diff_entry(
                                &mut entries,
                                session_id,
                                message,
                                part,
                                entry_index,
                                path,
                                change
                                    .from_path
                                    .as_deref()
                                    .map(compat_normalize_session_diff_path),
                                compat_session_diff_kind(change.kind),
                                diff.clone(),
                            );
                        }
                    }
                }
                Some(agena::message::PartContent::FileChange(change_part))
                    if !has_apply_patch_part =>
                {
                    for (entry_index, change) in change_part.changes.iter().enumerate() {
                        let path = compat_normalize_session_diff_path(&change.path);
                        if path.is_empty() {
                            continue;
                        }
                        compat_push_session_diff_entry(
                            &mut entries,
                            session_id,
                            message,
                            part,
                            entry_index,
                            path,
                            change
                                .from_path
                                .as_deref()
                                .map(compat_normalize_session_diff_path),
                            compat_session_diff_kind(change.kind),
                            None,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    entries
}

fn compat_trimmed_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn compat_attachment_source_from_legacy_part(
    record: &serde_json::Map<String, Value>,
) -> Option<agena::message::AttachmentSource> {
    if let Some(path) = compat_trimmed_text(record.get("serverPath").and_then(Value::as_str)) {
        return Some(agena::message::AttachmentSource::LocalPath { path });
    }

    let url = compat_trimmed_text(record.get("url").and_then(Value::as_str))?;
    if url.starts_with("data:") {
        Some(agena::message::AttachmentSource::DataUrl { url })
    } else {
        Some(agena::message::AttachmentSource::Url { url })
    }
}

fn compat_attachment_part_from_legacy_value(
    value: &Value,
) -> Result<agena::message::PartContent, (StatusCode, String)> {
    let record = value
        .as_object()
        .ok_or_else(|| compat_bad_request("legacy file part must be an object"))?;
    let mime = record
        .get("mime")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let filename = compat_trimmed_text(record.get("filename").and_then(Value::as_str));
    let title = compat_trimmed_text(record.get("title").and_then(Value::as_str));
    let source = compat_attachment_source_from_legacy_part(record)
        .ok_or_else(|| compat_bad_request("legacy file part requires url or serverPath"))?;
    let hint = filename.as_deref().or_else(|| match &source {
        agena::message::AttachmentSource::Url { url }
        | agena::message::AttachmentSource::DataUrl { url } => Some(url.as_str()),
        agena::message::AttachmentSource::LocalPath { path } => Some(path.as_str()),
        agena::message::AttachmentSource::FileId { file_id } => Some(file_id.as_str()),
        agena::message::AttachmentSource::Base64 { .. } => None,
    });

    Ok(agena::message::PartContent::attachments(vec![
        agena::message::AttachmentItem {
            kind: agena::message::AttachmentKind::detect(mime.as_str(), hint),
            mime,
            source,
            filename,
            title,
            size_bytes: None,
            sha256: None,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        },
    ]))
}

fn compat_message_parts_from_payload(
    parts: &[Value],
) -> Result<Vec<agena::message::PartContent>, (StatusCode, String)> {
    parts
        .iter()
        .map(|part| {
            if matches!(part.get("type").and_then(Value::as_str), Some("file")) {
                return compat_attachment_part_from_legacy_value(part);
            }

            serde_json::from_value::<agena::message::PartContent>(part.clone()).map_err(|error| {
                compat_bad_request(format!("invalid message part payload: {error}"))
            })
        })
        .collect()
}

async fn compat_resolve_session_message_options(
    manager: &agena::session::SessionManager,
    session_id: i64,
    body: &CompatSessionMessageBody,
) -> Result<agena::session::SessionRunOptions, (StatusCode, String)> {
    let mut options = if let Some(model) = body.model.as_ref() {
        let provider_id = compat_trimmed_text(model.provider_id.as_deref())
            .ok_or_else(|| compat_bad_request("model.providerID is required"))?;
        let model_id = compat_trimmed_text(model.model_id.as_deref())
            .ok_or_else(|| compat_bad_request("model.modelID is required"))?;
        let model = agena::model::ModelRef::try_new(provider_id, model_id)
            .map_err(|error| compat_bad_request(format!("invalid model reference: {error}")))?;
        agena::session::SessionRunOptions {
            model,
            variant: None,
            thinking: None,
            system: None,
            temperature: None,
            max_output_tokens: None,
            agent_profile: None,
            max_turn_loops: None,
        }
    } else {
        match manager.resolve_scheduled_run_options(session_id).await {
            Ok(options) => options,
            Err(agena::AppError::Internal(message))
                if message.contains("model is required")
                    || message.contains("no providers configured") =>
            {
                return Err(compat_bad_request(message));
            }
            Err(error) => return Err(compat_internal(error.to_string())),
        }
    };

    if let Some(variant) = compat_trimmed_text(body.variant.as_deref()) {
        options.variant = Some(variant);
    }
    if let Some(agent_profile) = compat_trimmed_text(body.agent.as_deref())
        .or_else(|| compat_trimmed_text(body.agent_profile.as_deref()))
    {
        options.agent_profile = Some(agent_profile);
    }
    if let Some(system) = compat_trimmed_text(body.system.as_deref()) {
        options.system = Some(system);
    }
    if let Some(temperature) = body.temperature {
        options.temperature = Some(temperature);
    }
    if let Some(max_output_tokens) = body.max_output_tokens {
        options.max_output_tokens = Some(max_output_tokens);
    }
    if let Some(max_turn_loops) = body.max_turn_loops {
        options.max_turn_loops = Some(max_turn_loops);
    }

    Ok(options)
}

fn compat_message_part_value(session_id: i64, part: &agena::message::MessagePart) -> Value {
    let mut value = json!({
        "id": part.id.to_string(),
        "sessionId": session_id.to_string(),
        "sessionID": session_id.to_string(),
        "messageId": part.message_id.to_string(),
        "messageID": part.message_id.to_string(),
        "partId": part.id.to_string(),
        "partID": part.id.to_string(),
        "type": part.kind.to_string(),
        "status": part.status.to_string(),
        "name": part.name.clone(),
        "summary": part.summary.clone(),
        "hasDetail": part.has_detail,
        "operationId": part.operation_id.clone(),
        "createdAt": part.created_at.timestamp_millis(),
        "content": part.content.clone(),
    });

    if let Some(obj) = value.as_object_mut() {
        match part.content.as_ref() {
            Some(agena::message::PartContent::Text(text)) => {
                obj.insert("text".to_string(), Value::String(text.text.clone()));
            }
            Some(agena::message::PartContent::Reasoning(reasoning)) => {
                obj.insert(
                    "text".to_string(),
                    Value::String(reasoning.summary.join("\n")),
                );
            }
            Some(agena::message::PartContent::ToolExecution(tool)) => {
                obj.insert(
                    "tool".to_string(),
                    serde_json::to_value(tool).unwrap_or(Value::Null),
                );
            }
            Some(agena::message::PartContent::CommandExecution(command)) => {
                obj.insert("text".to_string(), Value::String(command.command.clone()));
            }
            _ => {}
        }
    }

    value
}

fn compat_message_entry_value(session_id: i64, message: &agena::message::Message) -> Value {
    let parts = message
        .parts
        .iter()
        .map(|part| compat_message_part_value(session_id, part))
        .collect::<Vec<_>>();
    let model_id = message.metadata.model_id.trim();
    let provider_id = message.metadata.model_provider_id.trim();

    json!({
        "info": {
            "id": message.id.to_string(),
            "sessionID": session_id.to_string(),
            "role": message.role.to_string(),
            "finish": message.finish.clone(),
            "modelID": (!model_id.is_empty()).then_some(model_id),
            "providerID": (!provider_id.is_empty()).then_some(provider_id),
            "time": {
                "created": message.created_at.timestamp_millis(),
            }
        },
        "parts": parts,
    })
}

async fn compat_session_list(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Query(query): Query<CompatSessionListQuery>,
) -> Result<Response, (StatusCode, String)> {
    let manager = compat_session_manager(state.as_ref())?;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.map(|value| value.min(MAX_COMPAT_LIST_LIMIT));
    let include_total = query.include_total.unwrap_or(false);
    let fetch_limit = limit.map(|value| value.saturating_add(1) as u64);

    let mut sessions = manager
        .list_session_summaries(agena::session::SessionListRequest {
            offset: offset as u64,
            limit: fetch_limit,
            include_subagents: false,
        })
        .await
        .map_err(|error| compat_internal(error.to_string()))?;

    let has_more = limit.is_some_and(|value| sessions.len() > value);
    if let Some(limit) = limit {
        sessions.truncate(limit);
    }

    let session_values = sessions
        .iter()
        .map(compat_session_summary_value)
        .collect::<Vec<_>>();
    if !include_total {
        return Ok(Json(Value::Array(session_values)).into_response());
    }

    let total = manager
        .list_session_summaries(agena::session::SessionListRequest {
            offset: 0,
            limit: None,
            include_subagents: false,
        })
        .await
        .map_err(|error| compat_internal(error.to_string()))?
        .len();

    Ok(Json(CompatSessionListResponse {
        sessions: session_values,
        total,
        offset,
        limit,
        has_more,
        next_offset: has_more.then_some(offset.saturating_add(sessions.len())),
    })
    .into_response())
}

async fn compat_session_create(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(body): Json<CompatSessionCreateBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let manager = compat_session_manager(state.as_ref())?;
    let created = manager
        .create_session(agena::session::SessionCreateRequest {
            title: body.title.unwrap_or_default(),
            parent_session_id: None,
        })
        .await
        .map_err(|error| compat_internal(error.to_string()))?;

    let summary = manager
        .list_session_summaries(agena::session::SessionListRequest::default())
        .await
        .map_err(|error| compat_internal(error.to_string()))?
        .into_iter()
        .find(|summary| summary.id == created.id)
        .ok_or_else(|| compat_internal("Created session summary is unavailable"))?;

    Ok(Json(compat_session_summary_value(&summary)))
}

async fn compat_session_status(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Query(query): Query<CompatSessionStatusQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let manager = compat_session_manager(state.as_ref())?;
    let session_ids = if let Some(session_id) = query
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        vec![compat_parse_id(session_id, "sessionId")?]
    } else {
        manager
            .list_session_summaries(agena::session::SessionListRequest::default())
            .await
            .map_err(|error| compat_internal(error.to_string()))?
            .into_iter()
            .map(|session| session.id)
            .collect::<Vec<_>>()
    };

    let mut payload = serde_json::Map::new();
    for session_id in session_ids {
        let status = if manager.is_turn_active(session_id).await {
            "busy"
        } else {
            let session = manager
                .get_session(session_id)
                .await
                .map_err(|error| compat_internal(error.to_string()))?;
            if session.blocked() || session.status() != agena::session::SessionStatus::Idle {
                "busy"
            } else {
                "idle"
            }
        };
        payload.insert(session_id.to_string(), json!({ "type": status }));
    }

    Ok(Json(Value::Object(payload)))
}

async fn compat_provider_env_check(
    Json(body): Json<CompatEnvCheckRequest>,
) -> Json<CompatEnvCheckResponse> {
    let mut present = Vec::<String>::new();
    let mut missing = Vec::<String>::new();
    let mut unique = std::collections::BTreeSet::<String>::new();

    for name in body.vars.into_iter().take(200) {
        if let Some(normalized) = compat_normalize_env_name(&name) {
            unique.insert(normalized);
        }
    }

    for name in unique {
        let configured = std::env::var_os(&name)
            .and_then(|value| value.into_string().ok())
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        if configured {
            present.push(name);
        } else {
            missing.push(name);
        }
    }

    Json(CompatEnvCheckResponse { present, missing })
}

async fn compat_directories(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Query(query): Query<CompatDirectoriesQuery>,
) -> CompatResult<Json<CompatPagedResponse<CompatDirectoryEntry>>> {
    let limit = query
        .limit
        .unwrap_or(DEFAULT_COMPAT_DIRECTORIES_LIMIT)
        .clamp(1, MAX_COMPAT_DIRECTORIES_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let query_norm = query.query.unwrap_or_default().trim().to_lowercase();

    let mut items = compat_list_all_workspaces(state.as_ref())
        .await?
        .into_iter()
        .map(|workspace| compat_workspace_directory_entry(&workspace))
        .collect::<Vec<_>>();
    if !query_norm.is_empty() {
        items.retain(|entry| {
            entry.id.to_lowercase().contains(&query_norm)
                || entry.path.to_lowercase().contains(&query_norm)
        });
    }

    Ok(Json(compat_page(items, offset, limit)))
}

async fn compat_session_activity(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> CompatResult<Json<Value>> {
    let Some(manager) = state.runtime.session_manager() else {
        return Ok(Json(json!({})));
    };

    let mut payload = serde_json::Map::new();
    for session in compat_list_all_sessions(state.as_ref()).await? {
        let loaded = manager
            .get_session(session.id)
            .await
            .map_err(|error| compat_internal(error.to_string()))?;
        if matches!(loaded.status(), agena::session::SessionStatus::AwaitingModel) {
            payload.insert(session.id.to_string(), json!({ "type": "busy" }));
        }
    }

    Ok(Json(Value::Object(payload)))
}

async fn compat_session_patch(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<CompatSessionPatchBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session_id = compat_parse_id(&session_id, "session_id")?;
    let title = body
        .title
        .ok_or_else(|| compat_bad_request("title is required"))?;

    let existing = state
        .compat_api_service
        .get_session(session_id)
        .await
        .map_err(|_| compat_internal("Failed to load session"))?
        .ok_or_else(|| compat_not_found("session not found"))?;

    let updated = state
        .compat_api_service
        .replace_session(
            session_id,
            agena_api_server::local_api::SessionReplaceRequest {
                title,
                parent_id: existing.parent_id,
            },
        )
        .await
        .map_err(|_| compat_internal("Failed to update session"))?;

    Ok(Json(compat_session_resource_value(&updated)))
}

async fn compat_session_fork(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<CompatSessionForkBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session_id = compat_parse_id(&session_id, "session_id")?;
    let message_id = compat_parse_id(&body.message_id, "messageID")?;
    compat_require_session(state.as_ref(), session_id).await?;

    let manager = compat_session_manager(state.as_ref())?;
    compat_require_projected_message(manager.as_ref(), session_id, message_id).await?;

    let forked = manager
        .fork_session(agena::session::SessionForkRequest {
            session_id,
            at_message_id: Some(message_id),
            title: None,
            expected_version: None,
        })
        .await
        .map_err(|error| compat_internal(error.to_string()))?;

    Ok(Json(json!({ "id": forked.id.to_string() })))
}

async fn compat_session_delete(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session_id = compat_parse_id(&session_id, "session_id")?;
    state
        .compat_api_service
        .get_session(session_id)
        .await
        .map_err(|_| compat_internal("Failed to load session"))?
        .ok_or_else(|| compat_not_found("session not found"))?;

    let deleted = state
        .compat_api_service
        .delete_session(session_id)
        .await
        .map_err(|_| compat_internal("Failed to delete session"))?;

    Ok(Json(compat_session_resource_value(&deleted)))
}

async fn compat_session_revert(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<CompatSessionRevertBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session_id = compat_parse_id(&session_id, "session_id")?;
    let message_id = compat_parse_id(&body.message_id, "messageID")?;
    compat_require_session(state.as_ref(), session_id).await?;

    let manager = compat_session_manager(state.as_ref())?;
    compat_require_projected_message(manager.as_ref(), session_id, message_id).await?;
    manager
        .rewind_session(agena::session::SessionRewindRequest {
            session_id,
            message_id,
            expected_version: None,
        })
        .await
        .map_err(|error| compat_internal(error.to_string()))?;

    Ok(Json(json!({
        "id": session_id.to_string(),
        "revert": {
            "messageID": message_id.to_string(),
        }
    })))
}

async fn compat_session_unrevert(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session_id = compat_parse_id(&session_id, "session_id")?;
    compat_require_session(state.as_ref(), session_id).await?;

    let manager = compat_session_manager(state.as_ref())?;
    if let Some(message_id) = compat_latest_rewind_target(manager.as_ref(), session_id).await? {
        manager
            .unrewind_session(agena::session::SessionUnrewindRequest {
                session_id,
                message_id,
                expected_version: None,
            })
            .await
            .map_err(|error| compat_internal(error.to_string()))?;
    }

    Ok(Json(json!({
        "id": session_id.to_string(),
        "revert": Value::Null,
    })))
}

async fn compat_session_message_post(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<CompatSessionMessageBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session_id = compat_parse_id(&session_id, "session_id")?;
    if body.parts.is_empty() {
        return Ok(Json(json!({ "queued": false })));
    }

    state
        .compat_api_service
        .get_session(session_id)
        .await
        .map_err(|_| compat_internal("Failed to load session"))?
        .ok_or_else(|| compat_not_found("session not found"))?;

    let manager = compat_session_manager(state.as_ref())?;
    let options =
        compat_resolve_session_message_options(manager.as_ref(), session_id, &body).await?;
    let parts = compat_message_parts_from_payload(&body.parts)?;
    let manager = manager.clone();
    tokio::spawn(async move {
        if let Err(error) = manager
            .submit_user_turn(agena::session::SessionUserTurnRequest {
                session_id,
                options,
                parts,
            })
            .await
        {
            tracing::warn!(
                session_id,
                error = %error,
                "compat session message submission failed"
            );
        }
    });

    Ok(Json(json!({ "queued": true })))
}

async fn compat_session_abort(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session_id = compat_parse_id(&session_id, "session_id")?;
    state
        .compat_api_service
        .get_session(session_id)
        .await
        .map_err(|_| compat_internal("Failed to load session"))?
        .ok_or_else(|| compat_not_found("session not found"))?;

    let manager = compat_session_manager(state.as_ref())?;
    let _ = manager.cancel_active_turn(session_id).await;

    Ok(Json(json!({ "aborted": true })))
}

async fn compat_session_share(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session_id = compat_parse_id(&session_id, "session_id")?;
    compat_require_session(state.as_ref(), session_id).await?;

    Ok(Json(json!({
        "id": session_id.to_string(),
        "share": {
            "url": compat_session_share_url(session_id),
        }
    })))
}

async fn compat_session_unshare(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session_id = compat_parse_id(&session_id, "session_id")?;
    compat_require_session(state.as_ref(), session_id).await?;

    Ok(Json(json!({
        "id": session_id.to_string(),
        "share": Value::Null,
    })))
}

async fn compat_session_summarize(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<CompatSessionSummarizeBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session_id = compat_parse_id(&session_id, "session_id")?;
    compat_require_session(state.as_ref(), session_id).await?;

    let provider_id = compat_trimmed_text(body.provider_id.as_deref())
        .ok_or_else(|| compat_bad_request("providerID is required"))?;
    let model_id = compat_trimmed_text(body.model_id.as_deref())
        .ok_or_else(|| compat_bad_request("modelID is required"))?;
    let model = agena::model::ModelRef::try_new(provider_id.clone(), model_id.clone())
        .map_err(|error| compat_bad_request(format!("invalid model reference: {error}")))?;

    let manager = compat_session_manager(state.as_ref())?.clone();
    let options = agena::session::SessionRunOptions {
        model,
        variant: None,
        thinking: None,
        system: None,
        temperature: None,
        max_output_tokens: None,
        agent_profile: None,
        max_turn_loops: None,
    };

    tokio::spawn(async move {
        if let Err(error) = manager.compact_session(session_id, options).await {
            tracing::warn!(
                session_id,
                provider_id = %provider_id,
                model_id = %model_id,
                error = %error,
                "compat session summarize failed"
            );
        }
    });

    Ok(Json(json!({
        "ok": true,
        "queued": true,
        "auto": body.auto.unwrap_or(false),
    })))
}

async fn compat_session_diff(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<CompatSessionDiffQuery>,
) -> Result<Json<CompatSessionDiffListResponse>, (StatusCode, String)> {
    let session_id = compat_parse_id(&session_id, "session_id")?;
    compat_require_session(state.as_ref(), session_id).await?;

    if let Some(directory) = query.directory.as_deref() {
        let trimmed = directory.trim();
        if !trimmed.is_empty() {
            let _ = compat_validate_directory(trimmed).await?;
        }
    }

    let limit = query
        .limit
        .ok_or_else(|| compat_bad_request("limit parameter is required"))?
        .min(MAX_COMPAT_LIST_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let message_filter = query
        .message_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| compat_parse_id(value, "messageID"))
        .transpose()?;

    let manager = compat_session_manager(state.as_ref())?;
    let messages = manager
        .list_projected_messages(session_id, true)
        .await
        .map_err(|error| compat_internal(error.to_string()))?;
    let filtered_messages = messages
        .into_iter()
        .filter(|message| message_filter.is_none_or(|message_id| message.id == message_id))
        .collect::<Vec<_>>();

    let all_entries = compat_collect_session_diff_entries(session_id, &filtered_messages);
    let total = all_entries.len();
    let mut page_entries = all_entries
        .into_iter()
        .rev()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    page_entries.reverse();

    let has_more = offset.saturating_add(page_entries.len()) < total;
    let next_offset = has_more.then_some(offset.saturating_add(page_entries.len()));
    Ok(Json(CompatSessionDiffListResponse {
        entries: page_entries,
        total,
        offset,
        limit,
        has_more,
        next_offset,
    }))
}

async fn compat_session_message_list(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<CompatSessionMessagesQuery>,
) -> Result<Response, (StatusCode, String)> {
    let manager = compat_session_manager(state.as_ref())?;
    let session_id = compat_parse_id(&session_id, "session_id")?;
    manager
        .get_session(session_id)
        .await
        .map_err(|error| compat_not_found(error.to_string()))?;

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.map(|value| value.min(MAX_COMPAT_LIST_LIMIT));
    let include_total = query.include_total.unwrap_or(false);

    let all_messages = manager
        .list_projected_messages(session_id, true)
        .await
        .map_err(|error| compat_internal(error.to_string()))?;
    let total = all_messages.len();

    let mut newest_slice = all_messages
        .into_iter()
        .rev()
        .skip(offset)
        .take(limit.unwrap_or(usize::MAX))
        .collect::<Vec<_>>();
    newest_slice.reverse();

    let entries = newest_slice
        .iter()
        .map(|message| compat_message_entry_value(session_id, message))
        .collect::<Vec<_>>();

    if !include_total {
        return Ok(Json(Value::Array(entries)).into_response());
    }

    let has_more = offset.saturating_add(entries.len()) < total;
    Ok(Json(CompatSessionMessageListResponse {
        entries,
        total,
        offset,
        limit,
        has_more,
        next_offset: has_more.then_some(offset.saturating_add(newest_slice.len())),
    })
    .into_response())
}

async fn compat_session_message_part_detail(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    AxumPath((session_id, message_id, part_id)): AxumPath<(String, String, String)>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let manager = compat_session_manager(state.as_ref())?;
    let session_id = compat_parse_id(&session_id, "session_id")?;
    let message_id = compat_parse_id(&message_id, "message_id")?;
    let part_id = compat_parse_id(&part_id, "part_id")?;

    let message = manager
        .find_projected_message(session_id, message_id, true)
        .await
        .map_err(|error| compat_internal(error.to_string()))?
        .ok_or_else(|| compat_not_found("message not found"))?;
    let part = manager
        .find_projected_part(part_id)
        .await
        .map_err(|error| compat_internal(error.to_string()))?
        .filter(|part| part.message_id == message.id)
        .ok_or_else(|| compat_not_found("part not found"))?;

    Ok(Json(compat_message_part_value(session_id, &part)))
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
        .map(compat_resolve_path)
        .unwrap_or_else(|| state.runtime.workspace_root().to_path_buf());
    let summary_only = query.summary.unwrap_or(false);
    let scope = query
        .scope
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("all")
        .to_ascii_lowercase();

    let empty_response = || {
        Json(GitStatusCompatResponse {
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
            scope: scope.clone(),
            diff_stats: None,
        })
    };

    if !command_available("git") {
        return empty_response();
    }

    let Some(repo_root) = git_output(&workspace_root, &["rev-parse", "--show-toplevel"])
        .map(|path| compat_resolve_path(path.as_str()))
    else {
        return empty_response();
    };

    let current = git_output(&repo_root, &["branch", "--show-current"]).unwrap_or_default();
    let tracking = git_output(
        &repo_root,
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
            &repo_root,
            &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        )
    });
    let (ahead, behind) = parse_ahead_behind(ahead_behind.as_deref());
    let status = compat_run_git(
        &repo_root,
        &["status", "--porcelain", "--untracked-files=all"],
    )
    .ok()
    .and_then(|(code, stdout, _)| (code == 0).then_some(stdout))
    .unwrap_or_default();
    let files = compat_parse_git_status_files(status.as_str());
    let (staged_count, unstaged_count, untracked_count, merge_count, total_files) =
        compat_count_git_status_files(&files);

    let mut scoped = match scope.as_str() {
        "staged" => files
            .into_iter()
            .filter(compat_git_status_is_staged)
            .collect::<Vec<_>>(),
        "unstaged" => files
            .into_iter()
            .filter(compat_git_status_is_unstaged)
            .collect::<Vec<_>>(),
        "merge" => files
            .into_iter()
            .filter(compat_git_status_is_merge)
            .collect::<Vec<_>>(),
        "untracked" => files
            .into_iter()
            .filter(compat_git_status_is_untracked)
            .collect::<Vec<_>>(),
        _ => files,
    };

    let offset = if summary_only {
        0
    } else {
        query.offset.unwrap_or(0)
    };
    let limit = if summary_only {
        0
    } else {
        query.limit.unwrap_or(200).min(500)
    };
    let scoped_total = scoped.len();
    let end = offset.saturating_add(limit).min(scoped_total);
    let has_more = if summary_only {
        false
    } else {
        end < scoped_total
    };
    let page_files = if summary_only || limit == 0 || offset >= scoped_total {
        Vec::new()
    } else {
        scoped.drain(offset..end).collect::<Vec<_>>()
    };

    let mut diff_stats = None;
    if query.include_diff_stats.unwrap_or(false) && !summary_only && !page_files.is_empty() {
        let mut map = HashMap::<String, GitStatusCompatDiffStat>::new();
        let mut paths = page_files
            .iter()
            .map(|file| file.path.trim().to_string())
            .filter(|path| !path.is_empty())
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();

        if !paths.is_empty() {
            let mut staged_args = vec![
                "diff".to_string(),
                "--cached".to_string(),
                "--numstat".to_string(),
                "--".to_string(),
            ];
            staged_args.extend(paths.iter().cloned());
            let staged_refs = staged_args.iter().map(String::as_str).collect::<Vec<_>>();
            if let Ok((code, stdout, _)) = compat_run_git(&repo_root, &staged_refs)
                && code == 0
            {
                compat_parse_git_numstat(&stdout, &mut map);
            }

            let mut working_args = vec![
                "diff".to_string(),
                "--numstat".to_string(),
                "--".to_string(),
            ];
            working_args.extend(paths.iter().cloned());
            let working_refs = working_args.iter().map(String::as_str).collect::<Vec<_>>();
            if let Ok((code, stdout, _)) = compat_run_git(&repo_root, &working_refs)
                && code == 0
            {
                compat_parse_git_numstat(&stdout, &mut map);
            }
        }

        for file in &page_files {
            let status_code = if file.working_dir.trim().is_empty() {
                file.index.trim()
            } else {
                file.working_dir.trim()
            };
            if status_code != "?" && status_code != "A" {
                continue;
            }
            if map
                .get(&file.path)
                .is_some_and(|existing| existing.insertions > 0)
            {
                continue;
            }
            if let Some(stat) = compat_estimate_new_file_lines(&repo_root, &file.path).await {
                map.insert(file.path.clone(), stat);
            }
        }

        let allowed = page_files
            .iter()
            .map(|file| file.path.clone())
            .collect::<HashSet<_>>();
        map.retain(|path, _| allowed.contains(path));
        diff_stats = Some(map);
    }

    Json(GitStatusCompatResponse {
        current,
        tracking,
        ahead,
        behind,
        files: page_files,
        total_files,
        staged_count,
        unstaged_count,
        untracked_count,
        merge_count,
        offset: offset as u64,
        limit: limit as u64,
        has_more,
        scope,
        diff_stats,
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

    let meta = query
        .include_meta
        .unwrap_or(false)
        .then(|| compat_parse_diff_meta(&stdout));
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
        tokio::fs::read_to_string(&absolute)
            .await
            .unwrap_or_default()
    };

    Ok(Json(GitFileDiffResponseCompat { original, modified }))
}

async fn compat_git_commit_file_diff(
    Query(query): Query<GitCommitFileCompatQuery>,
) -> CompatResult<Json<GitCommitFileDiffResponseCompat>> {
    let (_dir, repo_root, relative) =
        compat_git_require_file_path(query.directory.as_deref(), query.path.as_deref()).await?;
    let commit = query
        .commit
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("commit parameter is required"))?;
    let context = query.context_lines.unwrap_or(3).clamp(0, 500);
    let args = vec![
        "show".to_string(),
        "--no-color".to_string(),
        "--no-ext-diff".to_string(),
        format!("-U{context}"),
        "--format=".to_string(),
        commit.to_string(),
        "--".to_string(),
        relative,
    ];
    let args_ref = args.iter().map(String::as_str).collect::<Vec<_>>();

    let (code, stdout, stderr) = compat_run_git(&repo_root, &args_ref).map_err(compat_internal)?;
    if code != 0 {
        let message = stderr.trim();
        return Err(compat_bad_request(if message.is_empty() {
            "git show failed".to_string()
        } else {
            message.to_string()
        }));
    }

    Ok(Json(GitCommitFileDiffResponseCompat { diff: stdout }))
}

async fn compat_git_commit_file_content(
    Query(query): Query<GitCommitFileContentCompatQuery>,
) -> CompatResult<Json<GitCommitFileContentResponseCompat>> {
    let (_dir, repo_root, relative) =
        compat_git_require_file_path(query.directory.as_deref(), query.path.as_deref()).await?;
    let commit = query
        .commit
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("commit parameter is required"))?;
    let verify_spec = format!("{commit}^{{commit}}");
    let (code, _stdout, stderr) =
        compat_run_git(&repo_root, &["rev-parse", "--verify", verify_spec.as_str()])
            .map_err(compat_internal)?;
    if code != 0 {
        let message = stderr.trim();
        return Err(compat_bad_request(if message.is_empty() {
            "Invalid commit".to_string()
        } else {
            message.to_string()
        }));
    }

    let object_spec = format!("{commit}:{relative}");
    let (code, _stdout, _stderr) =
        compat_run_git(&repo_root, &["cat-file", "-e", object_spec.as_str()])
            .map_err(compat_internal)?;
    if code != 0 {
        return Ok(Json(GitCommitFileContentResponseCompat {
            content: String::new(),
            exists: false,
            binary: false,
            truncated: false,
        }));
    }

    let (code, stdout, stderr) =
        compat_run_git(&repo_root, &["cat-file", "-t", object_spec.as_str()])
            .map_err(compat_internal)?;
    if code != 0 {
        let message = stderr.trim();
        return Err(compat_bad_request(if message.is_empty() {
            "git cat-file failed".to_string()
        } else {
            message.to_string()
        }));
    }

    if stdout.trim() != "blob" {
        return Ok(Json(GitCommitFileContentResponseCompat {
            content: String::new(),
            exists: true,
            binary: true,
            truncated: false,
        }));
    }

    let (code, stdout, stderr) = compat_run_git_bytes(&repo_root, &["show", object_spec.as_str()])
        .map_err(compat_internal)?;
    if code != 0 {
        let message = stderr.trim();
        return Err(compat_bad_request(if message.is_empty() {
            "git show failed".to_string()
        } else {
            message.to_string()
        }));
    }

    let truncated = stdout.len() as u64 > MAX_COMPAT_FILE_BYTES;
    let limit = MAX_COMPAT_FILE_BYTES as usize;
    let payload = if truncated {
        &stdout[..limit]
    } else {
        stdout.as_slice()
    };

    Ok(Json(GitCommitFileContentResponseCompat {
        content: String::from_utf8_lossy(payload).to_string(),
        exists: true,
        binary: std::str::from_utf8(payload).is_err(),
        truncated,
    }))
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

async fn compat_git_conflict_file(
    Query(query): Query<GitPathCompatQuery>,
) -> CompatResult<Json<GitConflictFileResponseCompat>> {
    let (_dir, repo_root, relative) =
        compat_git_require_file_path(query.directory.as_deref(), query.path.as_deref()).await?;
    let absolute = repo_root.join(&relative).clean();
    let metadata = tokio::fs::metadata(&absolute)
        .await
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => compat_not_found("File not found"),
            std::io::ErrorKind::PermissionDenied => compat_forbidden("Access to file denied"),
            _ => compat_internal(error.to_string()),
        })?;
    if !metadata.is_file() {
        return Err(compat_bad_request("Specified path is not a file"));
    }
    if metadata.len() > MAX_COMPAT_CONFLICT_FILE_BYTES {
        return Err(compat_bad_request("File too large"));
    }

    let text = compat_fs_read_file_text(&absolute).await?;
    let blocks = compat_parse_conflict_markers(&text);
    let has_markers = !blocks.is_empty()
        && text.contains("<<<<<<<")
        && text.contains("=======")
        && text.contains(">>>>>>>");

    Ok(Json(GitConflictFileResponseCompat {
        path: relative.clone(),
        text,
        blocks,
        has_markers,
        is_unmerged: compat_git_path_is_unmerged(&repo_root, &relative).await,
    }))
}

async fn compat_git_conflict_resolve(
    Query(query): Query<FsWriteCompatQuery>,
    Json(body): Json<GitConflictResolveCompatBody>,
) -> CompatResult<Json<FsWriteCompatResponse>> {
    let path = body
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compat_bad_request("path is required"))?;
    let (_dir, repo_root, relative) =
        compat_git_require_file_path(query.directory.as_deref(), Some(path)).await?;
    let absolute = repo_root.join(&relative).clean();
    let strategy = body
        .strategy
        .as_deref()
        .unwrap_or("manual")
        .trim()
        .to_ascii_lowercase();
    let stage = body.stage.unwrap_or(true);

    if strategy == "ours" || strategy == "theirs" {
        let flag = if strategy == "ours" {
            "--ours"
        } else {
            "--theirs"
        };
        let (code, _stdout, stderr) =
            compat_run_git(&repo_root, &["checkout", flag, "--", relative.as_str()])
                .map_err(compat_internal)?;
        if code != 0 {
            let message = stderr.trim();
            return Err(compat_bad_request(if message.is_empty() {
                "git checkout failed".to_string()
            } else {
                message.to_string()
            }));
        }
    } else {
        let text = compat_fs_read_file_text(&absolute).await?;
        if !text.contains("<<<<<<<") {
            return Err(compat_bad_request("No conflict markers found"));
        }

        let mut choices = HashMap::<usize, String>::new();
        if let Some(list) = body.choices {
            for item in list {
                let Some(id) = item.id else {
                    continue;
                };
                let Some(choice) = item.choice.as_deref() else {
                    continue;
                };
                let choice = choice.trim().to_ascii_lowercase();
                if matches!(choice.as_str(), "ours" | "theirs" | "base" | "both") {
                    choices.insert(id, choice);
                }
            }
        }

        let default_choice = if strategy == "both" {
            "both"
        } else if strategy == "base" {
            "base"
        } else {
            "ours"
        };
        let resolved = compat_apply_conflict_choices(&text, &choices, default_choice);
        tokio::fs::write(&absolute, resolved)
            .await
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::PermissionDenied => compat_forbidden("Access denied"),
                _ => compat_internal(error.to_string()),
            })?;
    }

    if stage {
        let (code, _stdout, stderr) = compat_run_git(&repo_root, &["add", "--", relative.as_str()])
            .map_err(compat_internal)?;
        if code != 0 {
            let message = stderr.trim();
            return Err((
                StatusCode::CONFLICT,
                if message.is_empty() {
                    "git add failed".to_string()
                } else {
                    message.to_string()
                },
            ));
        }
    }

    Ok(Json(FsWriteCompatResponse { success: true }))
}

async fn compat_git_watch(Query(query): Query<GitWatchCompatQuery>) -> CompatResult<Response> {
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
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
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
        compat_api_service: agena_api_server::local_api::ApiService::new(
            db.clone(),
            runtime.workspace_root().display().to_string(),
            runtime
                .session_manager()
                .map(|manager| manager.event_publisher()),
        ),
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
        .route("/api/config/reload", post(compat_config_reload))
        .route("/api/config/providers", get(compat_config_providers))
        .route(
            "/api/provider/{provider_id}/source",
            get(compat_provider_source),
        )
        .route("/api/provider/env/check", post(compat_provider_env_check))
        .route("/api/directories", get(compat_directories))
        .route(
            "/api/session",
            get(compat_session_list).post(compat_session_create),
        )
        .route("/api/session-activity", get(compat_session_activity))
        .route("/api/session/status", get(compat_session_status))
        .route(
            "/api/session/{session_id}",
            patch(compat_session_patch).delete(compat_session_delete),
        )
        .route("/api/session/{session_id}/fork", post(compat_session_fork))
        .route(
            "/api/session/{session_id}/revert",
            post(compat_session_revert),
        )
        .route(
            "/api/session/{session_id}/unrevert",
            post(compat_session_unrevert),
        )
        .route(
            "/api/session/{session_id}/share",
            post(compat_session_share).delete(compat_session_unshare),
        )
        .route(
            "/api/session/{session_id}/summarize",
            post(compat_session_summarize),
        )
        .route("/api/session/{session_id}/diff", get(compat_session_diff))
        .route(
            "/api/session/{session_id}/message",
            get(compat_session_message_list).post(compat_session_message_post),
        )
        .route(
            "/api/session/{session_id}/message/{message_id}/part/{part_id}",
            get(compat_session_message_part_detail),
        )
        .route(
            "/api/session/{session_id}/abort",
            post(compat_session_abort),
        )
        .route("/api/git/status", get(compat_git_status))
        .route("/api/git/watch", get(compat_git_watch))
        .route("/api/git/blame", get(compat_git_blame))
        .route("/api/git/diff", get(compat_git_diff))
        .route("/api/git/file-diff", get(compat_git_file_diff))
        .route(
            "/api/git/commit-file-diff",
            get(compat_git_commit_file_diff),
        )
        .route(
            "/api/git/commit-file-content",
            get(compat_git_commit_file_content),
        )
        .route("/api/git/conflicts/file", get(compat_git_conflict_file))
        .route(
            "/api/git/conflicts/resolve",
            post(compat_git_conflict_resolve),
        )
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
    use agena_api_server::local_api::WorkspaceResolveRequest;
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
        let status = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .status()
            .expect("git command should run");
        assert!(
            status.success(),
            "git -C {} {} should succeed",
            repo.display(),
            args.join(" ")
        );
    }

    fn init_git_repo(repo: &Path) {
        assert_git_available();
        let status = Command::new("git")
            .arg("init")
            .arg(repo)
            .status()
            .expect("git init should run");
        assert!(status.success(), "git init should succeed");
        git_ok(repo, &["config", "user.name", "Agena Test"]);
        git_ok(repo, &["config", "user.email", "test@example.com"]);
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

        (
            Arc::new(AppState {
                ui_auth: crate::ui_auth::init_ui_auth(None),
                ui_cookie_same_site: SameSite::Lax,
                cors_allowed_origins: Vec::new(),
                cors_allow_all: false,
                compat_api_service: agena_api_server::local_api::ApiService::new(
                    db.clone(),
                    runtime.workspace_root().display().to_string(),
                    runtime
                        .session_manager()
                        .map(|manager| manager.event_publisher()),
                ),
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

    async fn compat_seed_workspace(state: &AppState, path: &Path) -> WorkspaceResource {
        state
            .compat_api_service
            .resolve_workspace(WorkspaceResolveRequest {
                path: path.display().to_string(),
                create_if_missing: true,
            })
            .await
            .expect("workspace should resolve")
    }

    async fn spawn_mock_openai_server() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|| async {
                Json(json!({
                    "model": "gpt-4.1-mini",
                    "choices": [{
                        "finish_reason": "stop",
                        "message": {
                            "role": "assistant",
                            "content": "mock assistant response"
                        }
                    }],
                    "usage": {
                        "prompt_tokens": 1,
                        "completion_tokens": 1,
                        "total_tokens": 2
                    }
                }))
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener.local_addr().expect("listener should have address");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock openai server should serve");
        });

        (format!("http://{address}/v1"), handle)
    }

    async fn seed_compat_session_with_user_message(
        state: &Arc<AppState>,
        db: &Arc<sea_orm::DatabaseConnection>,
    ) -> (i64, i64, i64) {
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let session = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "compat-session".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should create");

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_millis()
            .min(i64::MAX as u128) as i64;
        let message_id = session.id.saturating_mul(1000).saturating_add(1);
        let part_id = session.id.saturating_mul(1000).saturating_add(2);
        let content = agena::message::PartContent::text("hello compat session");

        use sea_orm::{ActiveModelTrait as _, ActiveValue::Set};
        agena::db::entities::activity_message::ActiveModel {
            message_id: Set(message_id),
            session_id: Set(session.id),
            role: Set(agena::role::Role::User),
            state: Set(agena::message::ExecutionStatus::Completed),
            created_at_ms: Set(now_ms),
            updated_at_ms: Set(now_ms),
            metadata: Set(agena::message::MessageMetadata::default()),
            usage: Set(None),
            finish: Set(None),
            part_count: Set(1),
            is_compacted: Set(false),
        }
        .insert(db.as_ref())
        .await
        .expect("activity message should insert");
        agena::db::entities::activity_part::ActiveModel {
            part_id: Set(part_id),
            message_id: Set(message_id),
            session_id: Set(session.id),
            part_index: Set(0),
            status: Set(agena::message::ExecutionStatus::Completed),
            kind: Set(agena::message::PartKind::Text),
            name: Set(None),
            summary: Set(Some("hello compat session".to_string())),
            has_detail: Set(true),
            operation_id: Set(None),
            created_at_ms: Set(now_ms),
            content: Set(Some(content)),
        }
        .insert(db.as_ref())
        .await
        .expect("activity part should insert");
        (session.id, message_id, part_id)
    }

    async fn seed_compat_session_with_runtime_user_message(state: &Arc<AppState>) -> (i64, i64) {
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let session = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "compat-runtime-session".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should create");

        let _ = manager
            .submit_user_turn(agena::session::SessionUserTurnRequest {
                session_id: session.id,
                options: agena::session::SessionRunOptions {
                    model: agena::model::ModelRef::new("openai", "gpt-4.1-mini"),
                    variant: None,
                    thinking: None,
                    system: None,
                    temperature: None,
                    max_output_tokens: Some(32),
                    agent_profile: None,
                    max_turn_loops: None,
                },
                parts: vec![agena::message::PartContent::text("compat runtime message")],
            })
            .await;

        tokio::time::sleep(Duration::from_millis(50)).await;

        let message_id = manager
            .list_projected_messages(session.id, false)
            .await
            .expect("messages should load")
            .into_iter()
            .find(|message| message.role == agena::role::Role::User)
            .map(|message| message.id)
            .expect("user message should persist");

        (session.id, message_id)
    }

    async fn seed_compat_session_with_apply_patch_messages(
        state: &Arc<AppState>,
        db: &Arc<sea_orm::DatabaseConnection>,
    ) -> (i64, i64, i64) {
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let session = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "compat-diff-session".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should create");

        async fn insert_apply_patch_message(
            db: &Arc<sea_orm::DatabaseConnection>,
            session_id: i64,
            message_id: i64,
            part_id: i64,
            created_at_ms: i64,
            path: &str,
            diff: &str,
        ) {
            use sea_orm::{ActiveModelTrait as _, ActiveValue::Set};

            let content = agena::message::PartContent::ToolExecution(
                agena::message::ToolExecutionPart::Completed {
                    call_id: part_id,
                    invocation: agena::message::ToolInvocation::new(
                        "apply_patch",
                        agena::message::StructuredObject::default(),
                    ),
                    output_text: format!("patched {path}"),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: agena::message::ToolOutput::Custom {
                        output: agena::message::FirstPartyToolOutput::ApplyPatch {
                            operation_id: format!("op-{message_id}"),
                            changes: vec![agena::message::FileChangeEntry {
                                path: path.to_string(),
                                kind: agena::message::FileChangeKind::Updated,
                                from_path: None,
                            }],
                            before_hash: None,
                            after_hash: None,
                            inverse_patch: String::new(),
                            diff: diff.to_string(),
                            progress: Vec::new(),
                        }
                        .into_custom_output(),
                    },
                    lifecycle: agena::message::TimeRange::default(),
                },
            );
            agena::db::entities::activity_message::ActiveModel {
                message_id: Set(message_id),
                session_id: Set(session_id),
                role: Set(agena::role::Role::Tool),
                state: Set(agena::message::ExecutionStatus::Completed),
                created_at_ms: Set(created_at_ms),
                updated_at_ms: Set(created_at_ms),
                metadata: Set(agena::message::MessageMetadata::default()),
                usage: Set(None),
                finish: Set(None),
                part_count: Set(1),
                is_compacted: Set(false),
            }
            .insert(db.as_ref())
            .await
            .expect("activity message should insert");
            agena::db::entities::activity_part::ActiveModel {
                part_id: Set(part_id),
                message_id: Set(message_id),
                session_id: Set(session_id),
                part_index: Set(0),
                status: Set(agena::message::ExecutionStatus::Completed),
                kind: Set(content.kind()),
                name: Set(None),
                summary: Set(None),
                has_detail: Set(true),
                operation_id: Set(None),
                created_at_ms: Set(created_at_ms),
                content: Set(Some(content)),
            }
            .insert(db.as_ref())
            .await
            .expect("activity part should insert");
        }

        let base = session.id.saturating_mul(1000);
        let older_message_id = base.saturating_add(11);
        let newer_message_id = base.saturating_add(21);
        insert_apply_patch_message(
            db,
            session.id,
            older_message_id,
            base.saturating_add(12),
            1_700_000_000_000,
            "src/older.rs",
            "diff --git a/src/older.rs b/src/older.rs\n--- a/src/older.rs\n+++ b/src/older.rs\n@@ -1 +1 @@\n-old\n+older\n",
        )
        .await;
        insert_apply_patch_message(
            db,
            session.id,
            newer_message_id,
            base.saturating_add(22),
            1_700_000_000_100,
            "src/newer.rs",
            "diff --git a/src/newer.rs b/src/newer.rs\n--- a/src/newer.rs\n+++ b/src/newer.rs\n@@ -1 +1 @@\n-old\n+newer\n",
        )
        .await;

        (session.id, older_message_id, newer_message_id)
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
            .route("/api/provider/env/check", post(compat_provider_env_check))
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
        let payload: CompatEnvCheckResponse = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert!(payload.present.iter().any(|value| value == "PATH"));
        assert!(
            payload
                .missing
                .iter()
                .any(|value| value == "AGENA_STUDIO_COMPAT_MISSING_ENV")
        );
        assert!(!payload.present.iter().any(|value| value == "bad-name"));
        assert!(!payload.missing.iter().any(|value| value == "path"));

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_config_reload_returns_success_shape() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let router = Router::new()
            .route("/api/config/reload", post(compat_config_reload))
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
        assert_eq!(payload, json!({ "success": true }));

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_config_providers_returns_provider_defaults_and_models() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let router = Router::new()
            .route("/api/config/providers", get(compat_config_providers))
            .with_state(state.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/config/providers")
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

        let providers = payload["providers"]
            .as_array()
            .expect("providers should be an array");
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0]["id"], json!("openai"));
        assert_eq!(providers[0]["key"], json!("configured"));
        assert_eq!(providers[0]["auth"]["configured"], json!(true));
        assert_eq!(providers[0]["auth"]["credentialType"], json!("api"));

        let models = providers[0]["models"]
            .as_array()
            .expect("models should be an array");
        assert!(
            models.iter().any(|model| {
                model["id"] == json!("gpt-4.1-mini") && model["providerId"] == json!("openai")
            }),
            "expected default model in provider models: {models:?}"
        );

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_provider_source_returns_expected_shape_for_runtime_config() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let router = Router::new()
            .route(
                "/api/provider/{provider_id}/source",
                get(compat_provider_source),
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
        assert_eq!(payload["sources"]["auth"]["exists"], json!(true));
        assert_eq!(payload["sources"]["user"]["exists"], json!(false));
        assert_eq!(payload["sources"]["project"]["exists"], json!(false));
        assert_eq!(payload["sources"]["custom"]["exists"], json!(true));
        assert!(payload["sources"]["custom"]["path"].is_string());
        assert!(payload["sources"]["user"]["path"].is_string());

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_provider_source_uses_project_path_for_directory_scoped_lookup() {
        let (state, _db, _config, workspace) = compat_test_app_state().await;
        let project_config_dir = workspace.path().join(".agena");
        std::fs::create_dir_all(&project_config_dir).expect("project config dir should exist");
        let router = Router::new()
            .route(
                "/api/provider/{provider_id}/source",
                get(compat_provider_source),
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
        assert_eq!(payload["sources"]["project"]["exists"], json!(false));
        assert_eq!(
            payload["sources"]["project"]["path"],
            json!(workspace.path().join(".agena").join("config.toml").display().to_string())
        );
        assert_eq!(payload["sources"]["custom"]["exists"], json!(true));

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_provider_source_returns_empty_sources_for_unknown_provider() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let router = Router::new()
            .route(
                "/api/provider/{provider_id}/source",
                get(compat_provider_source),
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
    async fn compat_directories_pages_workspace_entries() {
        let (state, _db, _config, workspace) = compat_test_app_state().await;
        let alpha = workspace.path().join("alpha");
        let beta = workspace.path().join("beta");
        let gamma = workspace.path().join("gamma");
        std::fs::create_dir_all(&alpha).expect("alpha should exist");
        std::fs::create_dir_all(&beta).expect("beta should exist");
        std::fs::create_dir_all(&gamma).expect("gamma should exist");
        compat_seed_workspace(state.as_ref(), &alpha).await;
        compat_seed_workspace(state.as_ref(), &beta).await;
        compat_seed_workspace(state.as_ref(), &gamma).await;

        let router = Router::new()
            .route("/api/directories", get(compat_directories))
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
        let payload: CompatPagedResponse<CompatDirectoryEntry> = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload.total, 3);
        assert_eq!(payload.offset, 1);
        assert_eq!(payload.limit, 2);
        assert_eq!(payload.items.len(), 2);
        assert!(!payload.has_more);
        assert_eq!(payload.next_offset, None);
        assert_eq!(
            payload
                .items
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
            vec![beta.display().to_string(), alpha.display().to_string()]
        );

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_activity_returns_empty_snapshot_when_no_sessions_are_waiting() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let router = Router::new()
            .route("/api/session-activity", get(compat_session_activity))
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
        assert_eq!(payload, json!({}));

        state.runtime.shutdown();
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
            .route("/api/git/status", get(compat_git_status))
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
        let payload: GitStatusCompatResponse = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload.scope, "staged");
        assert_eq!(payload.total_files, 4);
        assert_eq!(payload.staged_count, 2);
        assert_eq!(payload.unstaged_count, 2);
        assert_eq!(payload.untracked_count, 1);
        assert_eq!(payload.merge_count, 0);
        assert_eq!(payload.offset, 0);
        assert_eq!(payload.limit, 1);
        assert!(payload.has_more);
        assert_eq!(
            payload.files,
            vec![GitStatusCompatFile {
                path: "mixed.txt".to_string(),
                index: "M".to_string(),
                working_dir: "M".to_string(),
            }]
        );
        assert!(payload.diff_stats.is_none());

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_list_route_returns_paginated_sessions() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let first = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "first".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("first session should create");
        let second = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "second".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("second session should create");

        let router = Router::new()
            .route("/api/session", get(compat_session_list))
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/session?offset=0&limit=1&includeTotal=true")
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
        let returned_id = sessions[0]
            .get("id")
            .and_then(Value::as_str)
            .expect("id should be present");
        assert!(returned_id == first.id.to_string() || returned_id == second.id.to_string());

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_create_route_returns_session_summary_shape() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let router = Router::new()
            .route("/api/session", post(compat_session_create))
            .with_state(state.clone());

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/session")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({ "title": "created compat" }).to_string()))
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

        let session_id = payload
            .get("id")
            .and_then(Value::as_str)
            .expect("id should be present");
        assert_eq!(
            payload.get("title").and_then(Value::as_str),
            Some("created compat")
        );
        assert!(
            payload
                .get("time")
                .and_then(|value| value.get("created"))
                .and_then(Value::as_i64)
                .is_some()
        );
        assert!(
            payload
                .get("time")
                .and_then(|value| value.get("updated"))
                .and_then(Value::as_i64)
                .is_some()
        );

        let created = state
            .compat_api_service
            .get_session(session_id.parse().expect("id should be numeric"))
            .await
            .expect("created session should load");
        assert!(created.is_some(), "created session should persist");

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_patch_route_updates_title() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let session = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "before rename".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should create");

        let router = Router::new()
            .route("/api/session/{session_id}", patch(compat_session_patch))
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/session/{}", session.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(json!({ "title": "after rename" }).to_string()))
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
            payload.get("id").and_then(Value::as_str),
            Some(session.id.to_string().as_str())
        );
        assert_eq!(
            payload.get("title").and_then(Value::as_str),
            Some("after rename")
        );
        assert!(
            payload
                .get("time")
                .and_then(|value| value.get("updated"))
                .and_then(Value::as_i64)
                .is_some()
        );

        let updated = state
            .compat_api_service
            .get_session(session.id)
            .await
            .expect("updated session should load")
            .expect("updated session should exist");
        assert_eq!(updated.title, "after rename");

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_delete_route_returns_deleted_session_and_removes_it() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let session = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "delete me".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should create");

        let router = Router::new()
            .route(
                "/api/session/{session_id}",
                patch(compat_session_patch).delete(compat_session_delete),
            )
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/session/{}", session.id))
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
            payload.get("id").and_then(Value::as_str),
            Some(session.id.to_string().as_str())
        );
        assert_eq!(
            payload.get("title").and_then(Value::as_str),
            Some("delete me")
        );

        let deleted = state
            .compat_api_service
            .get_session(session.id)
            .await
            .expect("deleted session lookup should succeed");
        assert!(deleted.is_none(), "deleted session should be removed");

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_share_routes_return_expected_shape() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let session = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "share compat".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should create");

        let router = Router::new()
            .route(
                "/api/session/{session_id}/share",
                post(compat_session_share).delete(compat_session_unshare),
            )
            .with_state(state.clone());

        let share_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/session/{}/share", session.id))
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
            Some(session.id.to_string().as_str())
        );
        assert_eq!(
            share_payload
                .get("share")
                .and_then(|value| value.get("url"))
                .and_then(Value::as_str),
            Some(format!("/chat?session={}", session.id).as_str())
        );

        let unshare_response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/session/{}/share", session.id))
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
            Some(session.id.to_string().as_str())
        );
        assert!(unshare_payload.get("share").is_some_and(Value::is_null));

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_summarize_route_returns_queued_ack_again() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let session = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "summarize compat".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should create");

        let router = Router::new()
            .route(
                "/api/session/{session_id}/summarize",
                post(compat_session_summarize),
            )
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/session/{}/summarize", session.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "providerID": "openai",
                            "modelID": "gpt-4.1-mini",
                            "auto": false
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
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        assert_eq!(payload.get("queued").and_then(Value::as_bool), Some(true));
        assert_eq!(payload.get("auto").and_then(Value::as_bool), Some(false));

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_diff_route_returns_apply_patch_entries() {
        let (state, db, _config, workspace) = compat_test_app_state().await;
        let (session_id, older_message_id, newer_message_id) =
            seed_compat_session_with_apply_patch_messages(&state, &db).await;

        let workspace_path = workspace.path().display().to_string();
        let directory = urlencoding::encode(&workspace_path);
        let router = Router::new()
            .route("/api/session/{session_id}/diff", get(compat_session_diff))
            .with_state(state.clone());

        let latest_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/session/{session_id}/diff?directory={directory}&offset=0&limit=1"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(latest_response.status(), StatusCode::OK);
        let latest_payload: Value = serde_json::from_slice(
            &latest_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        let latest_entries = latest_payload
            .get("entries")
            .and_then(Value::as_array)
            .expect("entries should be present");
        assert_eq!(latest_entries.len(), 1);
        assert_eq!(latest_payload.get("total").and_then(Value::as_u64), Some(2));
        assert_eq!(
            latest_entries[0].get("messageID").and_then(Value::as_str),
            Some(newer_message_id.to_string().as_str())
        );
        assert_eq!(
            latest_entries[0].get("path").and_then(Value::as_str),
            Some("src/newer.rs")
        );
        assert!(
            latest_entries[0]
                .get("diff")
                .and_then(Value::as_str)
                .is_some_and(|diff| diff.contains("+newer"))
        );

        let filtered_response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/session/{session_id}/diff?limit=10&messageID={older_message_id}"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(filtered_response.status(), StatusCode::OK);
        let filtered_payload: Value = serde_json::from_slice(
            &filtered_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        let filtered_entries = filtered_payload
            .get("entries")
            .and_then(Value::as_array)
            .expect("entries should be present");
        assert_eq!(filtered_entries.len(), 1);
        assert_eq!(
            filtered_entries[0].get("messageID").and_then(Value::as_str),
            Some(older_message_id.to_string().as_str())
        );
        assert_eq!(
            filtered_entries[0].get("path").and_then(Value::as_str),
            Some("src/older.rs")
        );
        assert_eq!(
            filtered_entries[0].get("sessionID").and_then(Value::as_str),
            Some(session_id.to_string().as_str())
        );

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_fork_route_returns_forked_session_id() {
        let (base_url, server_handle) = spawn_mock_openai_server().await;
        let (state, _db, _config, _workspace) =
            compat_test_app_state_with_openai_base_url(&base_url).await;
        let (session_id, message_id) = seed_compat_session_with_runtime_user_message(&state).await;

        let router = Router::new()
            .route("/api/session/{session_id}/fork", post(compat_session_fork))
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/session/{session_id}/fork"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({ "messageID": message_id.to_string() }).to_string(),
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
        let fork_id = payload
            .get("id")
            .and_then(Value::as_str)
            .expect("fork id should be present");
        assert_ne!(fork_id, session_id.to_string());

        let forked = state
            .compat_api_service
            .get_session(fork_id.parse().expect("fork id should be numeric"))
            .await
            .expect("forked session should load")
            .expect("forked session should exist");
        assert_eq!(forked.parent_id, Some(session_id));

        state.runtime.shutdown();
        server_handle.abort();
    }

    #[tokio::test]
    async fn compat_session_revert_and_unrevert_routes_toggle_rewind_state() {
        let (base_url, server_handle) = spawn_mock_openai_server().await;
        let (state, _db, _config, _workspace) =
            compat_test_app_state_with_openai_base_url(&base_url).await;
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let (session_id, message_id) = seed_compat_session_with_runtime_user_message(&state).await;

        let router = Router::new()
            .route(
                "/api/session/{session_id}/revert",
                post(compat_session_revert),
            )
            .route(
                "/api/session/{session_id}/unrevert",
                post(compat_session_unrevert),
            )
            .with_state(state.clone());

        let revert_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/session/{session_id}/revert"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({ "messageID": message_id.to_string() }).to_string(),
                    ))
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
            revert_payload.get("id").and_then(Value::as_str),
            Some(session_id.to_string().as_str())
        );
        assert_eq!(
            revert_payload
                .get("revert")
                .and_then(|value| value.get("messageID"))
                .and_then(Value::as_str),
            Some(message_id.to_string().as_str())
        );
        assert_eq!(
            manager
                .list_rewind_checkpoints(session_id)
                .await
                .expect("rewind checkpoints should load")
                .last()
                .map(|checkpoint| checkpoint.target_message_id),
            Some(message_id)
        );
        let mut rewind_hidden = false;
        for _ in 0..10 {
            if manager
                .list_projected_messages(session_id, false)
                .await
                .expect("messages should load")
                .iter()
                .all(|message| message.id != message_id)
            {
                rewind_hidden = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(rewind_hidden, "rewind should hide the original message");

        let unrevert_response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/session/{session_id}/unrevert"))
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
            Some(session_id.to_string().as_str())
        );
        assert!(unrevert_payload.get("revert").is_some_and(Value::is_null));
        let mut unrevert_restored = false;
        for _ in 0..10 {
            if manager
                .list_projected_messages(session_id, false)
                .await
                .expect("messages should load")
                .iter()
                .any(|message| message.id == message_id)
            {
                unrevert_restored = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            unrevert_restored,
            "unrewind should restore the compacted message"
        );

        state.runtime.shutdown();
        server_handle.abort();
    }

    #[tokio::test]
    async fn compat_session_share_and_unshare_routes_return_share_state() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let session = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "shared".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should create");

        let router = Router::new()
            .route(
                "/api/session/{session_id}/share",
                post(compat_session_share).delete(compat_session_unshare),
            )
            .with_state(state.clone());

        let share_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/session/{}/share", session.id))
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
            Some(session.id.to_string().as_str())
        );
        assert_eq!(
            share_payload
                .get("share")
                .and_then(|value| value.get("url"))
                .and_then(Value::as_str),
            Some(format!("/chat?session={}", session.id).as_str())
        );

        let unshare_response = router
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/session/{}/share", session.id))
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
            Some(session.id.to_string().as_str())
        );
        assert!(unshare_payload.get("share").is_some_and(Value::is_null));

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_summarize_route_returns_queued_ack() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let session = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "summarize".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should create");

        let router = Router::new()
            .route(
                "/api/session/{session_id}/summarize",
                post(compat_session_summarize),
            )
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/session/{}/summarize", session.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "providerID": "openai",
                            "modelID": "gpt-4.1-mini",
                            "auto": false
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
        assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(true));
        assert_eq!(payload.get("queued").and_then(Value::as_bool), Some(true));
        assert_eq!(payload.get("auto").and_then(Value::as_bool), Some(false));

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_diff_route_returns_paginated_entries_and_message_filter() {
        let (state, db, _config, _workspace) = compat_test_app_state().await;
        let (session_id, older_message_id, newer_message_id) =
            seed_compat_session_with_apply_patch_messages(&state, &db).await;

        let router = Router::new()
            .route("/api/session/{session_id}/diff", get(compat_session_diff))
            .with_state(state.clone());

        let page_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/session/{session_id}/diff?offset=0&limit=1"))
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
        let entries = page_payload
            .get("entries")
            .and_then(Value::as_array)
            .expect("entries should be present");
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].get("file").and_then(Value::as_str),
            Some("src/newer.rs")
        );
        assert_eq!(entries[0].get("additions").and_then(Value::as_u64), Some(1));
        assert_eq!(entries[0].get("deletions").and_then(Value::as_u64), Some(1));

        let filtered_response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/session/{session_id}/diff?offset=0&limit=10&messageID={older_message_id}"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(filtered_response.status(), StatusCode::OK);
        let filtered_payload: Value = serde_json::from_slice(
            &filtered_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(
            filtered_payload.get("total").and_then(Value::as_u64),
            Some(1)
        );
        let filtered_entries = filtered_payload
            .get("entries")
            .and_then(Value::as_array)
            .expect("entries should be present");
        assert_eq!(filtered_entries.len(), 1);
        assert_eq!(
            filtered_entries[0].get("messageID").and_then(Value::as_str),
            Some(older_message_id.to_string().as_str())
        );
        assert_eq!(
            filtered_entries[0].get("file").and_then(Value::as_str),
            Some("src/older.rs")
        );
        assert_ne!(older_message_id, newer_message_id);

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_status_route_returns_idle_status_snapshot() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let session = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "idle".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should create");

        let router = Router::new()
            .route("/api/session/status", get(compat_session_status))
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/api/session/status?sessionId={}", session.id))
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
                .get(session.id.to_string())
                .and_then(|entry| entry.get("type"))
                .and_then(Value::as_str),
            Some("idle")
        );

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_message_routes_return_entries_and_part_detail() {
        let (state, db, _config, _workspace) = compat_test_app_state().await;
        let (session_id, message_id, part_id) =
            seed_compat_session_with_user_message(&state, &db).await;

        let router = Router::new()
            .route(
                "/api/session/{session_id}/message",
                get(compat_session_message_list),
            )
            .route(
                "/api/session/{session_id}/message/{message_id}/part/{part_id}",
                get(compat_session_message_part_detail),
            )
            .with_state(state.clone());

        let list_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/session/{session_id}/message?offset=0&limit=10&includeTotal=true"
                    ))
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
        let message_id_string = message_id.to_string();
        let part_id_string = part_id.to_string();
        assert_eq!(
            entries[0]
                .get("info")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str),
            Some(message_id_string.as_str())
        );
        assert_eq!(
            entries[0]
                .get("parts")
                .and_then(Value::as_array)
                .and_then(|parts| parts.first())
                .and_then(|part| part.get("id"))
                .and_then(Value::as_str),
            Some(part_id_string.as_str())
        );

        let part_response = router
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/session/{session_id}/message/{message_id}/part/{part_id}"
                    ))
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
            Some(part_id_string.as_str())
        );
        assert_eq!(
            part_payload.get("text").and_then(Value::as_str),
            Some("hello compat session")
        );

        state.runtime.shutdown();
    }

    #[tokio::test]
    async fn compat_session_message_post_route_returns_queued_immediately() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let session = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "message compat".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should create");

        let router = Router::new()
            .route(
                "/api/session/{session_id}/message",
                post(compat_session_message_post),
            )
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/session/{}/message", session.id))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "parts": [
                                { "type": "text", "text": "hello compat" },
                                {
                                    "type": "file",
                                    "mime": "text/plain",
                                    "url": "https://example.com/demo.txt",
                                    "filename": "demo.txt"
                                }
                            ]
                        })
                        .to_string(),
                    ))
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
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(body.as_ref())
        );
        let payload: Value =
            serde_json::from_slice(body.as_ref()).expect("response should be valid json");
        assert_eq!(payload.get("queued").and_then(Value::as_bool), Some(true));

        tokio::time::sleep(Duration::from_millis(50)).await;
        let messages = manager
            .list_projected_messages(session.id, true)
            .await
            .expect("messages should load");
        assert!(
            messages
                .iter()
                .any(|message| message.role == agena::role::Role::User),
            "queued compat turn should persist the user message"
        );

        state.runtime.shutdown();
    }

    #[test]
    fn compat_session_message_payload_normalizes_legacy_file_parts() {
        let parts = compat_message_parts_from_payload(&[
            json!({ "type": "text", "text": "hello compat" }),
            json!({
                "type": "file",
                "mime": "text/plain",
                "url": "https://example.com/demo.txt",
                "filename": "demo.txt"
            }),
        ])
        .expect("payload should normalize");

        assert!(matches!(
            parts.first(),
            Some(agena::message::PartContent::Text(text)) if text.text == "hello compat"
        ));
        let attachment = match parts.get(1) {
            Some(agena::message::PartContent::Attachment(part)) => part.attachments.first(),
            _ => None,
        }
        .expect("second part should be an attachment");
        assert_eq!(attachment.filename.as_deref(), Some("demo.txt"));
        assert!(matches!(
            &attachment.source,
            agena::message::AttachmentSource::Url { url }
                if url == "https://example.com/demo.txt"
        ));
    }

    #[tokio::test]
    async fn compat_session_abort_route_returns_ack_when_idle() {
        let (state, _db, _config, _workspace) = compat_test_app_state().await;
        let manager = state
            .runtime
            .session_manager()
            .expect("session manager should exist");
        let session = manager
            .create_session(agena::session::SessionCreateRequest {
                title: "abort compat".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session should create");

        let router = Router::new()
            .route(
                "/api/session/{session_id}/abort",
                post(compat_session_abort),
            )
            .with_state(state.clone());
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/session/{}/abort", session.id))
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
        assert_eq!(payload.get("aborted").and_then(Value::as_bool), Some(true));

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
            .route("/api/git/status", get(compat_git_status))
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
        let payload: GitStatusCompatResponse = serde_json::from_slice(
            &response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert_eq!(payload.scope, "all");
        assert_eq!(payload.total_files, 2);
        assert_eq!(payload.staged_count, 0);
        assert_eq!(payload.unstaged_count, 1);
        assert_eq!(payload.untracked_count, 1);
        assert_eq!(payload.merge_count, 0);
        assert_eq!(payload.files.len(), 2);
        let files_len = payload.files.len();

        let diff_stats = payload.diff_stats.expect("diff stats should be present");
        assert_eq!(
            diff_stats.get("tracked.txt"),
            Some(&GitStatusCompatDiffStat {
                insertions: 1,
                deletions: 0,
            })
        );
        assert_eq!(
            diff_stats.get("untracked.txt"),
            Some(&GitStatusCompatDiffStat {
                insertions: 2,
                deletions: 0,
            })
        );
        assert_eq!(diff_stats.len(), files_len);

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
                get(compat_git_commit_file_diff),
            )
            .route(
                "/api/git/commit-file-content",
                get(compat_git_commit_file_content),
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
        let diff_payload: GitCommitFileDiffResponseCompat = serde_json::from_slice(
            &diff_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert!(diff_payload.diff.contains("beta changed"));
        assert!(diff_payload.diff.contains("charlie"));

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
        let content_payload: GitCommitFileContentResponseCompat = serde_json::from_slice(
            &content_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert!(content_payload.exists);
        assert!(!content_payload.binary);
        assert!(!content_payload.truncated);
        assert_eq!(content_payload.content, "alpha\nbeta changed\ncharlie\n");

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
        let missing_payload: GitCommitFileContentResponseCompat = serde_json::from_slice(
            &missing_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert!(!missing_payload.exists);
        assert!(!missing_payload.binary);
        assert!(!missing_payload.truncated);
        assert!(missing_payload.content.is_empty());
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
            .route("/api/git/conflicts/file", get(compat_git_conflict_file))
            .route(
                "/api/git/conflicts/resolve",
                post(compat_git_conflict_resolve),
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
        let conflict_payload: GitConflictFileResponseCompat = serde_json::from_slice(
            &conflict_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert!(conflict_payload.has_markers);
        assert!(conflict_payload.is_unmerged);
        assert_eq!(conflict_payload.blocks.len(), 1);
        assert_eq!(conflict_payload.blocks[0].ours, "main line");
        assert_eq!(conflict_payload.blocks[0].theirs, "feature line");

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
        let resolve_payload: FsWriteCompatResponse = serde_json::from_slice(
            &resolve_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert!(resolve_payload.success);
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
        let after_payload: GitConflictFileResponseCompat = serde_json::from_slice(
            &after_response
                .into_body()
                .collect()
                .await
                .expect("body should collect")
                .to_bytes(),
        )
        .expect("response should be valid json");
        assert!(!after_payload.has_markers);
        assert!(!after_payload.is_unmerged);
        assert!(after_payload.blocks.is_empty());
    }

    #[tokio::test]
    async fn compat_git_blame_diff_patch_and_watch_routes_work() {
        let temp = tempdir().expect("tempdir should be created");
        let repo = temp.path();
        assert!(
            Command::new("git").arg("--version").output().is_ok(),
            "git is required for this test"
        );
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

        let blame_response = compat_git_blame(Query(GitPathCompatQuery {
            directory: Some(repo.display().to_string()),
            path: Some("notes.txt".to_string()),
        }))
        .await
        .expect("blame should succeed");
        assert_eq!(blame_response.0.lines.len(), 2);

        let diff_response = compat_git_diff(Query(GitDiffCompatQuery {
            directory: Some(repo.display().to_string()),
            path: Some("notes.txt".to_string()),
            staged: Some(false),
            context_lines: Some(3),
            include_meta: Some(true),
        }))
        .await
        .expect("diff should succeed");
        assert!(diff_response.0.diff.contains("beta changed"));
        assert!(
            diff_response
                .0
                .meta
                .as_ref()
                .is_some_and(|meta| !meta.hunks.is_empty())
        );

        let file_diff_response = compat_git_file_diff(Query(GitFileDiffCompatQuery {
            directory: Some(repo.display().to_string()),
            path: Some("notes.txt".to_string()),
            staged: Some(false),
        }))
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
                    .uri(format!(
                        "/api/git/watch?directory={directory}&intervalMs=500"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(route_watch_response.status(), StatusCode::OK);
    }
}
