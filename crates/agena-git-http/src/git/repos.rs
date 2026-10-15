use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

static REPO_DISCOVERY_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
const MAX_REPO_DISCOVERY_ENTRIES: usize = 1_000_000;
const REPO_DISCOVERY_CACHE_TTL: Duration = Duration::from_secs(30);
const MAX_REPO_DISCOVERY_CACHE_ENTRIES: usize = 64;

use super::{
    DirectoryQuery, git_command_transport_error_response, git_io_error_response,
    is_safe_repo_rel_path, map_git_failure, path_slash, rel_path_slash, require_directory,
    require_directory_raw, run_git, run_git_checked, spawn_libgit2,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Query for git repositories.
pub struct GitReposQuery {
    pub directory: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    pub search: Option<String>,
    pub refresh: Option<bool>,
    /// `current` resolves the containing repository and immediate children only.
    pub scope: Option<String>,
}

#[derive(Debug, Serialize)]
/// Response indicating whether a path is a git repository.
pub struct GitCheckResponse {
    #[serde(rename = "isGitRepository")]
    pub is_git_repository: bool,
}

pub async fn git_check(Query(q): Query<DirectoryQuery>) -> Response {
    let dir = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    // IMPORTANT: do not silently coerce git errors into `false`.
    // - If the repo is blocked by safe.directory (dubious ownership), surface the structured
    //   error so the UI can show the trust prompt.
    // - If git fails for other reasons, return the mapped error instead of hiding it.
    let (code, out, err) = match run_git(&dir, &["rev-parse", "--is-inside-work-tree"]).await {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": e,
                    "code": "git_spawn_failed",
                    "hint": "Ensure Git is installed and available on PATH, then retry.",
                })),
            )
                .into_response();
        }
    };

    if let Some(resp) = map_git_failure(code, &out, &err) {
        return resp;
    }

    let is_repo = out.trim() == "true";
    Json(GitCheckResponse {
        is_git_repository: is_repo,
    })
    .into_response()
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
/// Information about a git repository.
pub struct GitRepoInfo {
    pub root: String,
    pub relative: String,
    pub kind: String,
}

#[derive(Clone)]
struct RepoDiscoveryResult {
    repos: Vec<GitRepoInfo>,
    parent_repos: Vec<String>,
    truncated: bool,
    discovered_at: Instant,
}

type RepoDiscoveryEntry = Arc<tokio::sync::Mutex<Option<RepoDiscoveryResult>>>;

fn repo_discovery_entry(base: &Path) -> RepoDiscoveryEntry {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, RepoDiscoveryEntry>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    let mut entries = cache.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(entry) = entries.get(base) {
        return entry.clone();
    }
    if entries.len() >= MAX_REPO_DISCOVERY_CACHE_ENTRIES
        && let Some(oldest_unused) = entries
            .iter()
            .find(|(_, entry)| Arc::strong_count(entry) == 1)
            .map(|(path, _)| path.clone())
    {
        entries.remove(&oldest_unused);
    }
    let entry = Arc::new(tokio::sync::Mutex::new(None));
    entries.insert(base.to_path_buf(), entry.clone());
    entry
}

fn discover_parent_repos(base: &Path) -> Vec<String> {
    // Inspect metadata once per ancestor instead of spawning Git at every level.
    base.ancestors()
        .skip(1)
        .filter(|path| path.join(".git").exists())
        .map(path_slash)
        .collect()
}

fn repo_info(base: &Path, root: &Path) -> GitRepoInfo {
    GitRepoInfo {
        root: path_slash(root),
        relative: if root == base || base.starts_with(root) {
            ".".into()
        } else {
            rel_path_slash(base, root)
        },
        kind: if root.join(".git").is_file() {
            "worktree"
        } else {
            "dir"
        }
        .into(),
    }
}

fn discover_current_repos(base: &Path) -> RepoDiscoveryResult {
    let parent_repos = discover_parent_repos(base);
    let mut repos = Vec::new();
    let mut truncated = false;
    if let Some(root) = base.ancestors().find(|path| path.join(".git").exists()) {
        repos.push(repo_info(base, root));
    } else if let Ok(children) = std::fs::read_dir(base) {
        // A directory of projects is useful immediately, too. Deep discovery is
        // an explicit picker action and has its own cache/worker budget.
        for (index, child) in children.enumerate() {
            if index >= 2_000 {
                truncated = true;
                break;
            }
            let Ok(child) = child else {
                truncated = true;
                continue;
            };
            let root = child.path();
            if root.join(".git").exists() {
                repos.push(repo_info(base, &root));
            }
        }
    }
    repos.sort_by(|a, b| a.relative.cmp(&b.relative));
    RepoDiscoveryResult {
        repos,
        parent_repos,
        truncated,
        discovered_at: Instant::now(),
    }
}

fn discover_nested_repos(base: &Path) -> (Vec<GitRepoInfo>, bool) {
    let mut roots = HashSet::new();
    let mut truncated = false;
    let mut entries = 0_usize;
    let mut it = WalkDir::new(base).follow_links(false).into_iter();

    while let Some(next) = it.next() {
        entries = entries.saturating_add(1);
        if entries > MAX_REPO_DISCOVERY_ENTRIES {
            truncated = true;
            break;
        }
        let entry = match next {
            Ok(entry) => entry,
            Err(error) => {
                truncated = true;
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "repository discovery skipped an unreadable workspace entry",
                        &error,
                    ),
                    "repository discovery result is partial"
                );
                continue;
            }
        };

        let name = entry.file_name().to_string_lossy();
        if entry.file_type().is_dir() {
            if matches!(
                name.as_ref(),
                "node_modules" | "target" | "dist" | "build" | ".next" | ".agena"
            ) {
                it.skip_current_dir();
                continue;
            }

            if name == ".git" {
                if let Some(parent) = entry.path().parent() {
                    roots.insert(parent.to_path_buf());
                }
                it.skip_current_dir();
                continue;
            }
        }

        if entry.file_type().is_file()
            && name == ".git"
            && let Some(parent) = entry.path().parent()
        {
            roots.insert(parent.to_path_buf());
        }
    }
    let mut repos: Vec<GitRepoInfo> = roots
        .into_iter()
        .map(|root| {
            let git_path = root.join(".git");
            let kind = match std::fs::metadata(&git_path) {
                Ok(metadata) if metadata.is_file() => "worktree".to_owned(),
                Ok(metadata) if metadata.is_dir() => "dir".to_owned(),
                Ok(_) => "unknown".to_owned(),
                Err(error) => {
                    tracing::warn!(
                        path = %git_path.display(),
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "failed to inspect a discovered Git metadata path",
                            &error,
                        ),
                        "discovered repository kind is unknown"
                    );
                    "unknown".to_owned()
                }
            };
            GitRepoInfo {
                root: path_slash(&root),
                relative: rel_path_slash(base, &root),
                kind,
            }
        })
        .collect();
    repos.sort_by(|a, b| {
        let al = a.relative.matches('/').count();
        let bl = b.relative.matches('/').count();
        (a.relative != ".")
            .cmp(&(b.relative != "."))
            .then(al.cmp(&bl))
            .then(a.relative.cmp(&b.relative))
    });
    (repos, truncated)
}

pub async fn git_repos(Query(q): Query<GitReposQuery>) -> Response {
    let base = match require_directory_raw(q.directory.as_deref()) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let search = q.search.as_deref().map(str::trim).unwrap_or("").to_string();
    let normalized_search = search.to_lowercase();
    let page_size = q.page_size.unwrap_or(30).clamp(1, 200);
    let page_requested = q.page.is_some() || q.page_size.is_some() || !normalized_search.is_empty();

    let result = if q.scope.as_deref() == Some("current") {
        let scan_base = base.clone();
        match spawn_libgit2(move || discover_current_repos(&scan_base)).await {
            Ok(result) => result,
            Err(error) => return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to resolve Git repository: {error}")})),
            ).into_response(),
        }
    } else {
        let entry = repo_discovery_entry(&base);
        let mut cached = entry.lock().await;
        if q.refresh.unwrap_or(false) {
            *cached = None;
        }
        if cached
            .as_ref()
            .is_none_or(|result| result.discovered_at.elapsed() >= REPO_DISCOVERY_CACHE_TTL)
        {
            let permit = match REPO_DISCOVERY_WORKERS.acquire().await {
                Ok(permit) => permit,
                Err(error) => {
                    tracing::error!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "repository discovery worker pool is unavailable",
                            &error,
                        ),
                        "repository discovery request cannot run"
                    );
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(serde_json::json!({"error":"repository discovery is unavailable"})),
                    )
                        .into_response();
                }
            };
            let scan_base = base.clone();
            let scan = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let (repos, truncated) = discover_nested_repos(&scan_base);
                (repos, truncated, discover_parent_repos(&scan_base))
            })
            .await;
            let (repos, truncated, parent_repos) = match scan {
                Ok(result) => result,
                Err(error) => {
                    let diagnostic = agena_failure::diagnostic::format_error_chain_with_context(
                        "repository discovery worker task failed",
                        &error,
                    );
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error": diagnostic})),
                    )
                        .into_response();
                }
            };
            *cached = Some(RepoDiscoveryResult {
                repos,
                parent_repos,
                truncated,
                discovered_at: Instant::now(),
            });
        }
        cached
            .as_ref()
            .expect("repository discovery cache was filled")
            .clone()
    };
    let mut repos = result.repos.clone();
    let parent_repos = result.parent_repos.clone();
    let discovery_truncated = result.truncated;

    if !normalized_search.is_empty() {
        repos.retain(|repo| {
            repo.relative.to_lowercase().contains(&normalized_search)
                || repo.root.to_lowercase().contains(&normalized_search)
        });
    }

    let total = repos.len();
    let total_pages = if total == 0 {
        1
    } else {
        total.div_ceil(page_size)
    };
    let page = q.page.unwrap_or(1).clamp(1, total_pages);

    if page_requested {
        let start = (page - 1) * page_size;
        let end = (start + page_size).min(total);
        repos = if start < end {
            repos[start..end].to_vec()
        } else {
            Vec::new()
        };
    }

    Json(serde_json::json!({
        "repos": repos,
        "count": repos.len(),
        "base": path_slash(&base),
        "parentRepos": parent_repos,
        "parentCount": parent_repos.len(),
        "page": page,
        "pageSize": page_size,
        "total": total,
        "totalPages": total_pages,
        "hasMore": page < total_pages,
        "discoveryTruncated": discovery_truncated,
        "search": search,
    }))
    .into_response()
}

pub async fn git_safe_directory(Query(q): Query<DirectoryQuery>) -> Response {
    let dir = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let safe_path = path_slash(&dir);
    let (check_code, check_out, check_err) =
        match run_git(&dir, &["config", "--global", "--get-all", "safe.directory"]).await {
            Ok(result) => result,
            Err(error) => {
                return git_command_transport_error_response(
                    "read global Git safe.directory configuration",
                    &error,
                    Some("git_safe_directory_read_process_failed"),
                );
            }
        };

    if check_code == 0 {
        let exists = check_out
            .lines()
            .map(|line| line.trim())
            .any(|line| line == "*" || line == safe_path);
        if exists {
            return Json(serde_json::json!({
                "success": true,
                "path": safe_path,
                "alreadyPresent": true,
            }))
            .into_response();
        }
    } else if check_code != 1 {
        if let Some(response) = map_git_failure(check_code, &check_out, &check_err) {
            return response;
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": check_err.trim(),
                "code": "git_safe_directory_read_failed",
            })),
        )
            .into_response();
    }

    if let Err(resp) = run_git_checked(
        &dir,
        &["config", "--global", "--add", "safe.directory", &safe_path],
        Some("git_safe_directory_failed"),
    )
    .await
    {
        return resp;
    }

    Json(serde_json::json!({
        "success": true,
        "path": safe_path,
        "alreadyPresent": false,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Body of a git init request.
pub struct GitInitBody {
    pub path: Option<String>,
    pub default_branch: Option<String>,
}

pub async fn git_init(Query(q): Query<DirectoryQuery>, Json(body): Json<GitInitBody>) -> Response {
    let base = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let rel = body.path.as_deref().map(|s| s.trim()).unwrap_or(".");

    let target: PathBuf = if rel.is_empty() || rel == "." {
        base.clone()
    } else {
        if !is_safe_repo_rel_path(rel) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid path", "code": "invalid_path"})),
            )
                .into_response();
        }
        base.join(rel)
    };

    // Keep init contained within the requested project directory.
    if !target.starts_with(&base) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Path escapes project directory", "code": "invalid_path"}),
            ),
        )
            .into_response();
    }

    if let Err(err) = tokio::fs::create_dir_all(&target).await {
        return git_io_error_response(
            "create the target directory for Git initialization",
            &err,
            "mkdir_failed",
        );
    }

    let git_marker = target.join(".git");
    if tokio::fs::metadata(&git_marker).await.is_ok() {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::json!({"error": "Already a git repository", "code": "already_git_repo"}),
            ),
        )
            .into_response();
    }

    let default_branch = body
        .default_branch
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    if let Some(branch) = default_branch
        && branch.chars().any(|ch| ch.is_whitespace())
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Invalid default branch name",
                "code": "invalid_default_branch",
            })),
        )
            .into_response();
    }

    let mut args: Vec<&str> = vec!["init"];
    if let Some(branch) = default_branch {
        args.push("--initial-branch");
        args.push(branch);
    }

    if let Err(resp) = run_git_checked(&target, &args, Some("git_init_failed")).await {
        return resp;
    }

    Json(serde_json::json!({
        "success": true,
        "root": path_slash(&target),
        "relative": rel_path_slash(&base, &target),
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Body of a git clone request.
pub struct GitCloneBody {
    pub url: Option<String>,
    pub path: Option<String>,
    #[serde(default)]
    pub recursive: bool,
    pub r#ref: Option<String>,
    pub depth: Option<u32>,
}

fn infer_repo_dir(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let last = trimmed.rsplit(['/', ':']).next().unwrap_or(trimmed);
    let mut name = last.to_string();
    if let Some(stripped) = name.strip_suffix(".git") {
        name = stripped.to_string();
    }
    if name.is_empty() {
        return None;
    }
    Some(name)
}

pub async fn git_clone(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitCloneBody>,
) -> Response {
    let base = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let Some(url) = body
        .url
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "url is required", "code": "missing_url"})),
        )
            .into_response();
    };

    let clone_ref = body
        .r#ref
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    if let Some(rf) = clone_ref
        && rf.chars().any(|ch| ch.is_whitespace())
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid ref name", "code": "invalid_ref"})),
        )
            .into_response();
    }

    let clone_depth = body.depth;
    if matches!(clone_depth, Some(0)) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "depth must be greater than 0", "code": "invalid_depth"})),
        )
            .into_response();
    }

    let mut rel = body.path.as_deref().map(|s| s.trim()).unwrap_or("");
    let inferred;
    if rel.is_empty() {
        inferred = infer_repo_dir(url).unwrap_or_default();
        rel = inferred.as_str();
    }

    if rel.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "path is required", "code": "missing_path"})),
        )
            .into_response();
    }

    if !is_safe_repo_rel_path(rel) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid path", "code": "invalid_path"})),
        )
            .into_response();
    }

    let target = base.join(rel);
    if !target.starts_with(&base) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Path escapes project directory", "code": "invalid_path"}),
            ),
        )
            .into_response();
    }

    match tokio::fs::metadata(&target).await {
        Ok(meta) if meta.is_dir() => {
            let mut entries = match tokio::fs::read_dir(&target).await {
                Ok(entries) => entries,
                Err(error) => {
                    return git_io_error_response(
                        "open the Git clone target directory",
                        &error,
                        "clone_target_read_failed",
                    );
                }
            };
            match entries.next_entry().await {
                Ok(Some(_)) => {
                    return (
                        StatusCode::CONFLICT,
                        Json(
                            serde_json::json!({"error": "Target directory not empty", "code": "target_not_empty"}),
                        ),
                    )
                        .into_response();
                }
                Ok(None) => {}
                Err(error) => {
                    return git_io_error_response(
                        "inspect the Git clone target directory",
                        &error,
                        "clone_target_read_failed",
                    );
                }
            }
        }
        Ok(_) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "Target exists", "code": "target_exists"})),
            )
                .into_response();
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return git_io_error_response(
                "inspect the Git clone target",
                &error,
                "clone_target_metadata_failed",
            );
        }
    }

    if let Some(parent) = target.parent()
        && let Err(err) = tokio::fs::create_dir_all(parent).await
    {
        return git_io_error_response(
            "create the Git clone target parent directory",
            &err,
            "mkdir_failed",
        );
    }

    let target_str = target.to_string_lossy().to_string();
    let mut args: Vec<String> = vec!["clone".to_string()];
    if body.recursive {
        args.push("--recursive".to_string());
    }
    if let Some(rf) = clone_ref {
        args.push("--branch".to_string());
        args.push(rf.to_string());
    }
    if let Some(depth) = clone_depth {
        args.push("--depth".to_string());
        args.push(depth.to_string());
    }
    args.push(url.to_string());
    args.push(target_str);

    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    if let Err(resp) = run_git_checked(&base, &args_ref, Some("git_clone_failed")).await {
        return resp;
    }

    Json(serde_json::json!({
        "success": true,
        "root": path_slash(&target),
        "relative": rel_path_slash(&base, &target),
    }))
    .into_response()
}
