use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};

use crate::git2_utils;

use super::{
    MAX_BLOB_BYTES, git_command_result_or_log, git_io_error_response, git_task_error_response,
    git2_open_error_response, require_directory_raw, run_git, run_git_checked, spawn_libgit2,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// A file in the git status.
pub struct GitStatusFile {
    pub path: String,
    pub index: String,
    pub working_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_old_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_old_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// Response of a git status query.
pub struct GitStatusResponse {
    pub current: String,
    pub tracking: Option<String>,
    pub ahead: i32,
    pub behind: i32,
    pub files: Vec<GitStatusFile>,
    pub is_clean: bool,
    pub total_files: usize,
    pub staged_count: usize,
    pub unstaged_count: usize,
    pub untracked_count: usize,
    pub merge_count: usize,
    pub offset: usize,
    pub limit: usize,
    pub has_more: bool,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_stats: Option<HashMap<String, DiffStat>>,
}

#[derive(Debug, Serialize, Clone, Copy)]
/// Diff statistics.
pub struct DiffStat {
    pub insertions: i32,
    pub deletions: i32,
}

fn parse_numstat(raw: &str, map: &mut HashMap<String, DiffStat>) -> Result<(), String> {
    for (line_index, line) in raw
        .lines()
        .map(|line| line.trim())
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
    {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            return Err(format!(
                "invalid Git numstat record at line {}: expected at least three tab-separated fields",
                line_index + 1
            ));
        }
        let ins_raw = parts[0];
        let del_raw = parts[1];
        let path = parts[2..].join("\t");
        if path.is_empty() {
            return Err(format!(
                "invalid Git numstat record at line {}: file path is empty",
                line_index + 1
            ));
        }
        let ins = if ins_raw == "-" {
            0
        } else {
            ins_raw.parse::<i32>().map_err(|error| {
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!(
                        "invalid insertion count in Git numstat record at line {}",
                        line_index + 1
                    ),
                    &error,
                )
            })?
        };
        let del = if del_raw == "-" {
            0
        } else {
            del_raw.parse::<i32>().map_err(|error| {
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!(
                        "invalid deletion count in Git numstat record at line {}",
                        line_index + 1
                    ),
                    &error,
                )
            })?
        };
        let entry = map.entry(path).or_insert(DiffStat {
            insertions: 0,
            deletions: 0,
        });
        entry.insertions += ins;
        entry.deletions += del;
    }
    Ok(())
}

async fn estimate_new_file_lines(repo: &Path, file_rel: &str) -> std::io::Result<Option<DiffStat>> {
    let full = repo.join(file_rel);
    let meta = match tokio::fs::metadata(&full).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !meta.is_file() || meta.len() > MAX_BLOB_BYTES as u64 {
        return Ok(None);
    }
    let data = match tokio::fs::read(&full).await {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if data.contains(&0) {
        return Ok(Some(DiffStat {
            insertions: 0,
            deletions: 0,
        }));
    }
    let s = String::from_utf8_lossy(&data).replace("\r\n", "\n");
    if s.is_empty() {
        return Ok(Some(DiffStat {
            insertions: 0,
            deletions: 0,
        }));
    }
    let mut lines = s.split('\n').count() as i32;
    if s.ends_with('\n') {
        lines -= 1;
    }
    Ok(Some(DiffStat {
        insertions: lines.max(0),
        deletions: 0,
    }))
}

fn git_status_diagnostic_response(context: &str, diagnostic: &str, code: &'static str) -> Response {
    tracing::error!(
        operation = context,
        error_code = code,
        diagnostic,
        "Git status failed"
    );
    let public = agena_failure::diagnostic::user_message_with_context(diagnostic, 400);
    let public = if public.is_empty() {
        "Git status could not be computed".to_owned()
    } else {
        public
    };
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": public, "code": code})),
    )
        .into_response()
}

fn serialize_git_watch_event(value: serde_json::Value, event_kind: &'static str) -> String {
    match serde_json::to_string(&value) {
        Ok(json) => json,
        Err(error) => {
            tracing::error!(
                event_kind,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to serialize a Git watch SSE event",
                    &error,
                ),
                "Git watch event serialization failed"
            );
            r#"{"type":"git.watch.error","message":"Git watch event serialization failed"}"#
                .to_owned()
        }
    }
}

fn git2_other(context: &str, error: &git2::Error) -> git2_utils::Git2OpenError {
    git2_utils::Git2OpenError::Other(git2_utils::git2_error_diagnostic(context, error))
}

fn read_branch_state(
    repo: &git2::Repository,
) -> Result<(String, Option<String>, i32, i32), git2_utils::Git2OpenError> {
    use git2::{BranchType, ErrorCode};

    let head = match repo.head() {
        Ok(head) => head,
        Err(error) if matches!(error.code(), ErrorCode::NotFound | ErrorCode::UnbornBranch) => {
            return Ok((String::new(), None, 0, 0));
        }
        Err(error) => return Err(git2_other("failed to resolve Git HEAD", &error)),
    };
    if !head.is_branch() {
        return Ok(("HEAD".to_owned(), None, 0, 0));
    }

    let current = head
        .shorthand()
        .map_err(|error| git2_other("Git HEAD branch name is not valid UTF-8", &error))?
        .to_owned();
    let branch = repo
        .find_branch(&current, BranchType::Local)
        .map_err(|error| git2_other("failed to resolve the current local Git branch", &error))?;
    let upstream = match branch.upstream() {
        Ok(upstream) => upstream,
        Err(error) if error.code() == ErrorCode::NotFound => {
            return Ok((current, None, 0, 0));
        }
        Err(error) => {
            return Err(git2_other(
                "failed to resolve the upstream Git branch",
                &error,
            ));
        }
    };
    let tracking = upstream
        .get()
        .shorthand()
        .map_err(|error| git2_other("upstream Git branch name is not valid UTF-8", &error))?
        .to_owned();

    let (ahead, behind) = match (head.target(), upstream.get().target()) {
        (Some(head_id), Some(upstream_id)) => repo
            .graph_ahead_behind(head_id, upstream_id)
            .map_err(|error| {
                git2_other(
                    "failed to compute ahead/behind counts for the current Git branch",
                    &error,
                )
            })?,
        _ => (0, 0),
    };
    Ok((current, Some(tracking), ahead as i32, behind as i32))
}

async fn select_base_ref_for_unpublished(dir: &Path) -> Option<String> {
    let candidates = {
        let mut out = Vec::new();
        if let Some((code, stdout, stderr)) = git_command_result_or_log(
            run_git(dir, &["symbolic-ref", "-q", "refs/remotes/origin/HEAD"]).await,
            "resolve the default origin branch for unpublished-commit estimation",
        ) {
            if code == 0 {
                let branch = stdout.trim();
                if !branch.is_empty() {
                    out.push(branch.replace("refs/remotes/", ""));
                }
            } else {
                tracing::debug!(
                    git_exit_code = code,
                    git_stderr = %super::redact_git_output(&super::truncate_for_payload(&stderr, 4_000)),
                    "default origin branch is unavailable for unpublished-commit estimation"
                );
            }
        }
        out.extend(
            ["origin/main", "origin/master", "main", "master"]
                .into_iter()
                .map(|s| s.to_string()),
        );
        out
    };

    for r in candidates {
        let Some((code, stdout, stderr)) = git_command_result_or_log(
            run_git(dir, &["rev-parse", "--verify", &r]).await,
            "validate a Git base reference for unpublished-commit estimation",
        ) else {
            continue;
        };
        if code == 0 && !stdout.trim().is_empty() {
            return Some(r);
        }
        if code != 0 {
            tracing::debug!(
                git_exit_code = code,
                git_stderr = %super::redact_git_output(&super::truncate_for_payload(&stderr, 4_000)),
                "candidate Git base reference is unavailable"
            );
        }
    }
    None
}

#[derive(Debug, Deserialize)]
/// Query for git status.
pub struct GitStatusQuery {
    pub directory: Option<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    // "all" (default) | "staged" | "unstaged" | "merge" | "untracked"
    pub scope: Option<String>,
    // If true, return counts/branch info only (no file list).
    #[serde(default)]
    pub summary: bool,
    // If true, include per-file diff stats (can be expensive for large repos).
    #[serde(default, rename = "includeDiffStats")]
    pub include_diff_stats: bool,
}

pub async fn git_status(Query(q): Query<GitStatusQuery>) -> Response {
    let dir = match require_directory_raw(q.directory.as_deref()) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    // Use libgit2 for stable, structured status.
    // Keep output compatible with our current UI (porcelain-like index + working_dir codes).
    let snapshot = spawn_libgit2({
        let dir = dir.clone();
        move || {
            use git2::{Status, StatusOptions};

            let repo = match git2_utils::open_repo_discover(&dir) {
                Ok(r) => r,
                Err(e) => return Err(e),
            };

            let (current, tracking, ahead, behind) = read_branch_state(&repo)?;

            let mut opts = StatusOptions::new();
            opts.include_untracked(true)
                .recurse_untracked_dirs(true)
                .include_ignored(false)
                .include_unmodified(false)
                .renames_head_to_index(true);

            let statuses = repo.statuses(Some(&mut opts)).map_err(|error| {
                git2_utils::Git2OpenError::Other(git2_utils::git2_error_diagnostic(
                    "failed to inspect Git status",
                    &error,
                ))
            })?;

            fn idx_code(st: Status) -> &'static str {
                if st.is_conflicted() {
                    return "U";
                }
                if st.contains(Status::INDEX_NEW) {
                    return "A";
                }
                if st.contains(Status::INDEX_MODIFIED) {
                    return "M";
                }
                if st.contains(Status::INDEX_DELETED) {
                    return "D";
                }
                if st.contains(Status::INDEX_RENAMED) {
                    return "R";
                }
                if st.contains(Status::INDEX_TYPECHANGE) {
                    return "T";
                }
                ""
            }
            fn wt_code(st: Status) -> &'static str {
                if st.is_conflicted() {
                    return "U";
                }
                if st.contains(Status::WT_NEW) {
                    return "?";
                }
                if st.contains(Status::WT_MODIFIED) {
                    return "M";
                }
                if st.contains(Status::WT_DELETED) {
                    return "D";
                }
                if st.contains(Status::WT_RENAMED) {
                    return "R";
                }
                if st.contains(Status::WT_TYPECHANGE) {
                    return "T";
                }
                ""
            }

            let mut files: Vec<GitStatusFile> = Vec::new();
            for entry in statuses.iter() {
                let path = entry
                    .path()
                    .map_err(|error| git2_other("Git status path is not valid UTF-8", &error))?;
                let head_delta = entry.head_to_index();
                let work_delta = entry.index_to_workdir();
                let new_path = work_delta
                    .as_ref()
                    .and_then(|d| d.new_file().path())
                    .or_else(|| head_delta.as_ref().and_then(|d| d.new_file().path()));
                let path = new_path.and_then(|p| p.to_str()).unwrap_or(path);
                let old_path = |delta: Option<&git2::DiffDelta<'_>>| {
                    delta
                        .filter(|d| d.status() == git2::Delta::Renamed)
                        .and_then(|d| d.old_file().path())
                        .and_then(|p| p.to_str())
                        .map(str::to_owned)
                };
                let st = entry.status();
                let x = idx_code(st).to_string();
                let y = wt_code(st).to_string();
                if x.is_empty() && y.is_empty() {
                    continue;
                }
                // libgit2 uses WT_NEW for untracked. Match porcelain "??".
                let (x, y) = if y == "?" {
                    ("?".to_string(), "?".to_string())
                } else {
                    (x, y)
                };
                files.push(GitStatusFile {
                    path: path.to_string(),
                    index: x,
                    working_dir: y,
                    index_old_path: old_path(head_delta.as_ref()),
                    working_old_path: old_path(work_delta.as_ref()),
                });
            }
            files.sort_by(|a, b| a.path.cmp(&b.path));

            Ok((current, tracking, ahead, behind, files))
        }
    })
    .await;

    let (current, tracking, mut ahead, mut behind, files) = match snapshot {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return git2_open_error_response(e),
        Err(error) => {
            let diagnostic = agena_failure::diagnostic::format_error_chain_with_context(
                "Git status worker task failed",
                &error,
            );
            return git_status_diagnostic_response(
                "join Git status worker task",
                &diagnostic,
                "git2_task_failed",
            );
        }
    };

    let is_merge = |f: &GitStatusFile| f.index.trim() == "U" || f.working_dir.trim() == "U";
    let is_untracked = |f: &GitStatusFile| f.index.trim() == "?" && f.working_dir.trim() == "?";
    // Match VS Code Git view grouping semantics:
    // - Merge changes are separate.
    // - Untracked is separate.
    // - Staged vs unstaged can overlap for the same path (e.g. "MM").
    let is_staged = |f: &GitStatusFile| {
        if is_merge(f) {
            return false;
        }
        let x = f.index.trim();
        !x.is_empty() && x != "?"
    };
    let is_unstaged = |f: &GitStatusFile| {
        if is_merge(f) || is_untracked(f) {
            return false;
        }
        let y = f.working_dir.trim();
        !y.is_empty()
    };

    let total_files = files.len();
    let staged_count = files.iter().filter(|f| is_staged(f)).count();
    let unstaged_count = files.iter().filter(|f| is_unstaged(f)).count();
    let untracked_count = files.iter().filter(|f| is_untracked(f)).count();
    let merge_count = files.iter().filter(|f| is_merge(f)).count();

    let summary = q.summary;
    let scope = q
        .scope
        .as_deref()
        .unwrap_or("all")
        .trim()
        .to_ascii_lowercase();

    let mut scoped: Vec<GitStatusFile> = match scope.as_str() {
        "staged" => files.into_iter().filter(|f| is_staged(f)).collect(),
        "unstaged" => files.into_iter().filter(|f| is_unstaged(f)).collect(),
        "merge" => files.into_iter().filter(|f| is_merge(f)).collect(),
        "untracked" => files.into_iter().filter(|f| is_untracked(f)).collect(),
        _ => files,
    };

    let scope_total = scoped.len();
    let offset = if summary { 0 } else { q.offset.unwrap_or(0) };
    // Default to a bounded page size; callers can page via offset/limit.
    let mut limit = if summary { 0 } else { q.limit.unwrap_or(200) };
    // Guardrails for request size.
    limit = limit.min(500);

    let end = offset.saturating_add(limit).min(scope_total);
    let has_more = end < scope_total;
    let page_files = if limit == 0 || offset >= scope_total {
        Vec::new()
    } else {
        scoped.drain(offset..end).collect::<Vec<_>>()
    };

    let mut diff_stats: Option<HashMap<String, DiffStat>> = None;
    let include_diff_stats = q.include_diff_stats;

    if include_diff_stats && !summary && !page_files.is_empty() {
        let mut map: HashMap<String, DiffStat> = HashMap::new();
        let mut paths: Vec<String> = page_files
            .iter()
            .map(|f| f.path.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        paths.sort();
        paths.dedup();

        if !paths.is_empty() {
            let mut staged_args: Vec<String> = vec![
                "diff".into(),
                "--cached".into(),
                "--numstat".into(),
                "--".into(),
            ];
            staged_args.extend(paths.iter().cloned());
            let staged_refs: Vec<&str> = staged_args.iter().map(|s| s.as_str()).collect();
            let (staged, _) =
                match run_git_checked(&dir, &staged_refs, Some("git_status_staged_numstat_failed"))
                    .await
                {
                    Ok(output) => output,
                    Err(response) => return response,
                };
            if let Err(diagnostic) = parse_numstat(&staged, &mut map) {
                return git_status_diagnostic_response(
                    "parse staged Git diff statistics",
                    &diagnostic,
                    "git_status_numstat_invalid",
                );
            }

            let mut working_args: Vec<String> =
                vec!["diff".into(), "--numstat".into(), "--".into()];
            working_args.extend(paths.iter().cloned());
            let working_refs: Vec<&str> = working_args.iter().map(|s| s.as_str()).collect();
            let (working, _) = match run_git_checked(
                &dir,
                &working_refs,
                Some("git_status_worktree_numstat_failed"),
            )
            .await
            {
                Ok(output) => output,
                Err(response) => return response,
            };
            if let Err(diagnostic) = parse_numstat(&working, &mut map) {
                return git_status_diagnostic_response(
                    "parse worktree Git diff statistics",
                    &diagnostic,
                    "git_status_numstat_invalid",
                );
            }
        }

        // Estimate new file insertions for untracked/added where numstat didn't include content.
        // Limit this to the returned page so paging stays cheap.
        for f in &page_files {
            let status_code = if f.working_dir.trim().is_empty() {
                &f.index
            } else {
                &f.working_dir
            };
            if status_code != "?" && status_code != "A" {
                continue;
            }
            if let Some(existing) = map.get(&f.path)
                && existing.insertions > 0
            {
                continue;
            }
            match estimate_new_file_lines(&dir, &f.path).await {
                Ok(Some(stat)) => {
                    map.insert(f.path.clone(), stat);
                }
                Ok(None) => {}
                Err(error) => {
                    return git_io_error_response(
                        "estimate diff statistics for a new worktree file",
                        &error,
                        "git_status_file_read_failed",
                    );
                }
            }
        }

        // Only return stats for paths in this page to keep the response bounded.
        let allowed: HashSet<String> = page_files.iter().map(|f| f.path.clone()).collect();
        map.retain(|k, _| allowed.contains(k));
        diff_stats = Some(map);
    }

    // If no upstream tracking but we know current branch, estimate unpublished commits.
    if tracking.is_none()
        && !current.is_empty()
        && let Some(base) = select_base_ref_for_unpublished(&dir).await
        && let Some((code, stdout, stderr)) = git_command_result_or_log(
            run_git(&dir, &["rev-list", "--count", &format!("{base}..HEAD")]).await,
            "estimate unpublished Git commit count",
        )
    {
        if code == 0 {
            match stdout.trim().parse::<i32>() {
                Ok(count) => {
                    ahead = count;
                    behind = 0;
                }
                Err(error) => tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "Git returned an invalid unpublished commit count",
                        &error,
                    ),
                    "unpublished Git commit estimation was unavailable"
                ),
            }
        } else {
            tracing::debug!(
                git_exit_code = code,
                git_stderr = %super::redact_git_output(&super::truncate_for_payload(&stderr, 4_000)),
                "Git could not estimate unpublished commits"
            );
        }
    }

    let is_clean = total_files == 0;

    Json(GitStatusResponse {
        current,
        tracking,
        ahead,
        behind,
        files: page_files,
        is_clean,
        total_files,
        staged_count,
        unstaged_count,
        untracked_count,
        merge_count,
        offset,
        limit,
        has_more,
        scope,
        diff_stats,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
/// Query for git watch.
pub struct GitWatchQuery {
    pub directory: Option<String>,
    pub path: Option<String>,
    #[serde(rename = "intervalMs")]
    pub interval_ms: Option<u64>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
struct GitWatchStatusPayload {
    current: String,
    tracking: Option<String>,
    ahead: i32,
    behind: i32,
    staged_count: usize,
    unstaged_count: usize,
    untracked_count: usize,
    merge_count: usize,
    total_files: usize,
    is_clean: bool,
    worktree_signature: String,
    updated_at_ms: i64,
    #[serde(skip)]
    path_signatures: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_path_signature: Option<String>,
    scope_signatures: HashMap<String, String>,
}

#[derive(Debug, PartialEq, Eq)]
struct GitFileFingerprint {
    length: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    changed: (i64, i64),
    #[cfg(unix)]
    mode: u32,
    symlink: bool,
}

struct GitFileDigest {
    fingerprint: GitFileFingerprint,
    oid: git2::Oid,
}

// Stat is only a cheap gate. The revision includes the blob's content hash,
// so touching a file without changing its contents emits no refresh.
fn git_watch_file_oid(
    path: &Path,
    cache: &mut HashMap<PathBuf, GitFileDigest>,
    hash_budget: &mut u64,
) -> Result<Option<git2::Oid>, git2_utils::Git2OpenError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            cache.remove(path);
            return Ok(None);
        }
        Err(error) => {
            return Err(git2_utils::Git2OpenError::Other(
                agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to inspect a watched Git file",
                    &error,
                ),
            ));
        }
    };
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    let fingerprint = GitFileFingerprint {
        length: metadata.len(),
        modified: metadata.modified().ok(),
        symlink: metadata.file_type().is_symlink(),
        #[cfg(unix)]
        changed: (metadata.ctime(), metadata.ctime_nsec()),
        #[cfg(unix)]
        mode: metadata.mode(),
    };
    // A watch must not hash gigabytes of untracked/binary content merely to
    // decide whether a reader should refresh. Bound both file and scan I/O.
    const MAX_FILE_HASH_BYTES: u64 = 512 * 1024;
    let content_hash = fingerprint.symlink
        || (metadata.is_file()
            && metadata.len() <= MAX_FILE_HASH_BYTES
            && *hash_budget > metadata.len());
    if let Some(previous) = cache.get(path)
        && previous.fingerprint == fingerprint
    {
        return Ok(Some(previous.oid));
    }
    let oid = if fingerprint.symlink {
        let target = std::fs::read_link(path).map_err(|error| {
            git2_utils::Git2OpenError::Other(
                agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to read a watched Git symlink",
                    &error,
                ),
            )
        })?;
        git2::Oid::hash_object(
            git2::ObjectType::Blob,
            target.as_os_str().as_encoded_bytes(),
        )
        .map_err(|error| git2_other("failed to hash a watched Git symlink", &error))?
    } else if metadata.is_file() {
        if content_hash {
            use std::io::Read;
            let limit = MAX_FILE_HASH_BYTES.min(*hash_budget).saturating_add(1);
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .and_then(|file| file.take(limit).read_to_end(&mut bytes))
                .map_err(|error| {
                    git2_utils::Git2OpenError::Other(
                        agena_failure::diagnostic::format_error_chain_with_context(
                            "failed to read a watched Git file",
                            &error,
                        ),
                    )
                })?;
            *hash_budget = hash_budget.saturating_sub(bytes.len() as u64);
            if bytes.len() as u64 >= limit {
                git2::Oid::hash_object(
                    git2::ObjectType::Blob,
                    format!("{fingerprint:?}").as_bytes(),
                )
                .map_err(|error| git2_other("failed to hash a watched file revision", &error))?
            } else {
                git2::Oid::hash_object(git2::ObjectType::Blob, &bytes)
                    .map_err(|error| git2_other("failed to hash a watched Git file", &error))?
            }
        } else {
            // Large files use a conservative metadata revision. A pure touch
            // may wake a consumer, while unchanged files require no body I/O.
            git2::Oid::hash_object(
                git2::ObjectType::Blob,
                format!("{fingerprint:?}").as_bytes(),
            )
            .map_err(|error| git2_other("failed to hash a watched file revision", &error))?
        }
    } else {
        // A submodule directory is represented by its checked-out commit.
        return Ok(git2::Repository::open(path)
            .ok()
            .and_then(|repo| repo.head().ok().and_then(|head| head.target())));
    };
    cache.insert(path.to_path_buf(), GitFileDigest { fingerprint, oid });
    Ok(Some(oid))
}

struct GitWatchCache {
    revision: u64,
    checked_at: tokio::time::Instant,
    payload: GitWatchStatusPayload,
}

/// One OS watcher and one serialized status cache per repository, shared by
/// every browser/pane. The last stream drops the watcher and cached snapshot.
struct GitWatchHub {
    _watcher: Option<notify::RecommendedWatcher>,
    changes: tokio::sync::watch::Sender<u64>,
    snapshot: tokio::sync::Mutex<Option<GitWatchCache>>,
    file_digests: Arc<Mutex<HashMap<PathBuf, GitFileDigest>>>,
}

fn git_watch_hub(root: PathBuf, git_dir: PathBuf, common_dir: PathBuf) -> Arc<GitWatchHub> {
    static HUBS: OnceLock<Mutex<HashMap<PathBuf, Weak<GitWatchHub>>>> = OnceLock::new();
    let mut hubs = HUBS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    hubs.retain(|_, hub| hub.strong_count() > 0);
    if let Some(hub) = hubs.get(&root).and_then(Weak::upgrade) {
        return hub;
    }
    let (changes, _) = tokio::sync::watch::channel(0_u64);
    let wake = changes.clone();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.as_ref().is_ok_and(|event| matches!(event.kind, notify::EventKind::Access(_))) { return; }
        wake.send_modify(|revision| *revision = revision.wrapping_add(1));
    }).and_then(|mut watcher| {
        watcher.watch(&root, RecursiveMode::Recursive)?;
        // Linked worktrees keep their index/refs outside the worktree root.
        if !git_dir.starts_with(&root) { watcher.watch(&git_dir, RecursiveMode::Recursive)?; }
        if !common_dir.starts_with(&root) && common_dir != git_dir {
            watcher.watch(&common_dir, RecursiveMode::Recursive)?;
        }
        Ok(watcher)
    }).map_err(|error| {
        tracing::warn!(%error, "Git filesystem watch unavailable; using shared 60 second reconciliation");
    }).ok();
    let hub = Arc::new(GitWatchHub {
        _watcher: watcher,
        changes,
        snapshot: tokio::sync::Mutex::new(None),
        file_digests: Arc::new(Mutex::new(HashMap::new())),
    });
    hubs.insert(root, Arc::downgrade(&hub));
    hub
}

pub async fn git_watch(Query(q): Query<GitWatchQuery>) -> Response {
    let dir = match require_directory_raw(q.directory.as_deref()) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };
    // This is an identity lookup into the shared status cache, never an
    // additional filesystem read for each subscriber.
    let selected_path = q
        .path
        .filter(|path| !path.is_empty())
        .map(|path| path.replace('\\', "/"));

    // Validate repository early so the client gets a normal JSON response.
    let probe = spawn_libgit2({
        let dir = dir.clone();
        move || {
            let repo = git2_utils::open_repo_discover(&dir)?;
            let root = repo.workdir().unwrap_or(repo.path()).to_path_buf();
            let root = std::fs::canonicalize(&root).unwrap_or(root);
            Ok::<_, git2_utils::Git2OpenError>(git_watch_hub(
                root,
                repo.path().to_path_buf(),
                repo.commondir().to_path_buf(),
            ))
        }
    })
    .await;
    let hub = match probe {
        Ok(Ok(hub)) => hub,
        Ok(Err(e)) => return git2_open_error_response(e),
        Err(error) => return git_task_error_response("validate the Git watch repository", &error),
    };

    let interval_ms = q.interval_ms.unwrap_or(1500).clamp(1000, 10_000);
    let stream = async_stream::stream! {
        let mut last: Option<GitWatchStatusPayload> = None;
        let mut changes = hub.changes.subscribe();

        loop {
            let mut cache = hub.snapshot.lock().await;
            let mut revision = *changes.borrow_and_update();
            // Check the shared cache before doing any libgit2 work. Idle
            // streams only wake for a 60s safety reconciliation, not 1.5s scans.
            let cached = cache.as_ref().filter(|entry| entry.revision == revision && entry.checked_at.elapsed() < Duration::from_secs(60));
            let cache_hit = cached.is_some();
            let snapshot = if let Some(entry) = cached {
                Ok(Ok(entry.payload.clone()))
            } else {
            if let Some(entry) = cache.as_ref() {
                let cooldown = Duration::from_millis(interval_ms).saturating_sub(entry.checked_at.elapsed());
                tokio::time::sleep(cooldown).await;
            }
            revision = *changes.borrow_and_update();
            spawn_libgit2({
                let dir = dir.clone();
                let file_digests = hub.file_digests.clone();
                move || -> Result<GitWatchStatusPayload, git2_utils::Git2OpenError> {
                    use git2::{Status, StatusOptions};

                    let repo = git2_utils::open_repo_discover(&dir)?;

                    let (current, tracking, ahead, behind) = read_branch_state(&repo)?;

                    let mut opts = StatusOptions::new();
                    opts.include_untracked(true)
                        .recurse_untracked_dirs(true)
                        .include_ignored(false)
                        .include_unmodified(false)
                .renames_head_to_index(true);

                    let statuses = repo.statuses(Some(&mut opts)).map_err(|error| {
                        git2_other("failed to inspect Git status for the watch stream", &error)
                    })?;

                    fn idx_code(st: Status) -> &'static str {
                        if st.is_conflicted() {
                            return "U";
                        }
                        if st.contains(Status::INDEX_NEW) {
                            return "A";
                        }
                        if st.contains(Status::INDEX_MODIFIED) {
                            return "M";
                        }
                        if st.contains(Status::INDEX_DELETED) {
                            return "D";
                        }
                        if st.contains(Status::INDEX_RENAMED) {
                            return "R";
                        }
                        if st.contains(Status::INDEX_TYPECHANGE) {
                            return "T";
                        }
                        ""
                    }
                    fn wt_code(st: Status) -> &'static str {
                        if st.is_conflicted() {
                            return "U";
                        }
                        if st.contains(Status::WT_NEW) {
                            return "?";
                        }
                        if st.contains(Status::WT_MODIFIED) {
                            return "M";
                        }
                        if st.contains(Status::WT_DELETED) {
                            return "D";
                        }
                        if st.contains(Status::WT_RENAMED) {
                            return "R";
                        }
                        if st.contains(Status::WT_TYPECHANGE) {
                            return "T";
                        }
                        ""
                    }

                    fn fnv1a64_update(hash: &mut u64, bytes: &[u8]) {
                        const FNV_PRIME: u64 = 1099511628211;
                        for byte in bytes {
                            *hash ^= u64::from(*byte);
                            *hash = hash.wrapping_mul(FNV_PRIME);
                        }
                    }

                    let mut staged_count: usize = 0;
                    let mut unstaged_count: usize = 0;
                    let mut untracked_count: usize = 0;
                    let mut merge_count: usize = 0;
                    let mut total_files: usize = 0;
                    let mut worktree_signature: u64 = 1469598103934665603;
                    if let Ok(head) = repo.head() && let Some(oid) = head.target() {
                        fnv1a64_update(&mut worktree_signature, oid.as_bytes());
                    }
                    if let Ok(branch) = repo.find_branch(&current, git2::BranchType::Local)
                        && let Ok(upstream) = branch.upstream() && let Some(oid) = upstream.get().target() {
                        fnv1a64_update(&mut worktree_signature, oid.as_bytes());
                    }
                    let mut digests = file_digests.lock().unwrap_or_else(|error| error.into_inner());
                    let mut hash_budget = 2 * 1024 * 1024;
                    let mut retained = HashSet::new();
                    let mut path_signatures = HashMap::new();
                    let mut scope_signatures = HashMap::<String, u64>::new();

                    for entry in statuses.iter() {
                        let reported_path = entry.path().map_err(|error| {
                            git2_other("Git watch status path is not valid UTF-8", &error)
                        })?;
                        let head_delta = entry.head_to_index();
                        let work_delta = entry.index_to_workdir();
                        let path = work_delta.as_ref().and_then(|delta| delta.new_file().path())
                            .or_else(|| head_delta.as_ref().and_then(|delta| delta.new_file().path()))
                            .and_then(|path| path.to_str()).unwrap_or(reported_path);
                        let st = entry.status();
                        let mut x = idx_code(st);
                        let mut y = wt_code(st);

                        if x.is_empty() && y.is_empty() {
                            continue;
                        }

                        // libgit2 uses WT_NEW for untracked. Match porcelain "??".
                        if y == "?" {
                            x = "?";
                            y = "?";
                        }

                        let mut file_signature = 1469598103934665603;
                        fnv1a64_update(&mut file_signature, x.as_bytes());
                        fnv1a64_update(&mut file_signature, b"|");
                        fnv1a64_update(&mut file_signature, y.as_bytes());
                        fnv1a64_update(&mut file_signature, b"|");
                        fnv1a64_update(&mut file_signature, path.as_bytes());
                        if let Some(delta) = entry.head_to_index() {
                            fnv1a64_update(&mut file_signature, delta.old_file().id().as_bytes());
                            fnv1a64_update(&mut file_signature, delta.new_file().id().as_bytes());
                            fnv1a64_update(&mut file_signature, format!("{:?}", delta.new_file().mode()).as_bytes());
                        }
                        if let Some(delta) = entry.index_to_workdir() {
                            // Working diffs depend on the index baseline too.
                            // A checkout/stage can change it while the worktree
                            // tail and porcelain status codes stay identical.
                            fnv1a64_update(&mut file_signature, delta.old_file().id().as_bytes());
                            fnv1a64_update(&mut file_signature, delta.new_file().id().as_bytes());
                            fnv1a64_update(&mut file_signature, format!("{:?}", delta.new_file().mode()).as_bytes());
                        }
                        if let Some(root) = repo.workdir() {
                            let file = root.join(path);
                            retained.insert(file.clone());
                            if let Some(oid) = git_watch_file_oid(&file, &mut digests, &mut hash_budget)? { fnv1a64_update(&mut file_signature, oid.as_bytes()); }
                        }
                        let signature = format!("{file_signature:016x}");
                        path_signatures.insert(path.to_owned(), signature.clone());
                        for delta in [entry.head_to_index(), entry.index_to_workdir()].into_iter().flatten() {
                            if let Some(old) = delta.old_file().path().and_then(|path| path.to_str()) {
                                path_signatures.entry(old.to_owned()).or_insert_with(|| signature.clone());
                            }
                        }
                        fnv1a64_update(&mut worktree_signature, path.as_bytes());
                        fnv1a64_update(&mut worktree_signature, &file_signature.to_le_bytes());
                        fnv1a64_update(&mut worktree_signature, b"\n");

                        total_files += 1;

                        let is_merge = x == "U" || y == "U";
                        let is_untracked = x == "?" && y == "?";
                        let is_staged = !is_merge && !x.is_empty() && x != "?";
                        let is_unstaged = !is_merge && !is_untracked && !y.is_empty();
                        for (scope, included) in [("merge", is_merge), ("staged", is_staged), ("unstaged", is_unstaged), ("untracked", is_untracked)] {
                            if included {
                                fnv1a64_update(scope_signatures.entry(scope.to_owned()).or_insert(1469598103934665603), &file_signature.to_le_bytes());
                            }
                        }

                        if is_merge {
                            merge_count += 1;
                        }
                        if is_staged {
                            staged_count += 1;
                        }
                        if is_unstaged {
                            unstaged_count += 1;
                        }
                        if is_untracked {
                            untracked_count += 1;
                        }
                    }
                    digests.retain(|path, _| retained.contains(path));

                    Ok(GitWatchStatusPayload {
                        current,
                        tracking,
                        ahead,
                        behind,
                        staged_count,
                        unstaged_count,
                        untracked_count,
                        merge_count,
                        total_files,
                        is_clean: total_files == 0,
                        worktree_signature: format!("{worktree_signature:016x}"),
                        updated_at_ms: 0,
                        path_signatures,
                        selected_path_signature: None,
                        scope_signatures: scope_signatures.into_iter().map(|(scope, hash)| (scope, format!("{hash:016x}"))).collect(),
                    })
                }
            })
            .await
            };

            let mut payload = match snapshot {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => {
                    let diagnostic = e.message();
                    tracing::error!(
                        diagnostic,
                        "Git watch snapshot failed"
                    );
                    let message = agena_failure::diagnostic::user_message_with_context(
                        &diagnostic,
                        400,
                    );
                    let message = if message.is_empty() {
                        "Git watch snapshot failed".to_owned()
                    } else {
                        message
                    };
                    let payload = serialize_git_watch_event(serde_json::json!({
                        "type": "git.watch.error",
                        "message": message,
                    }), "error");
                    yield Ok::<Event, Infallible>(Event::default().event("error").data(payload));
                    break;
                }
                Err(error) => {
                    let diagnostic = agena_failure::diagnostic::format_error_chain_with_context(
                        "Git watch worker task failed",
                        &error,
                    );
                    tracing::error!(diagnostic, "Git watch worker task could not be joined");
                    let message = agena_failure::diagnostic::user_message_with_context(
                        &diagnostic,
                        400,
                    );
                    let message = if message.is_empty() {
                        "Git watch worker task failed".to_owned()
                    } else {
                        message
                    };
                    let payload = serialize_git_watch_event(serde_json::json!({
                        "type": "git.watch.error",
                        "message": message,
                    }), "error");
                    yield Ok::<Event, Infallible>(Event::default().event("error").data(payload));
                    break;
                }
            };

            if !cache_hit {
                let previous_time = cache.as_ref().map_or(0, |entry| entry.payload.updated_at_ms);
                payload.updated_at_ms = previous_time;
                if cache.as_ref().is_none_or(|entry| entry.payload != payload) {
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |duration| duration.as_millis() as i64);
                    payload.updated_at_ms = now.max(previous_time.saturating_add(1));
                }
                *cache = Some(GitWatchCache { revision, checked_at: tokio::time::Instant::now(), payload: payload.clone() });
            }
            drop(cache);
            payload.selected_path_signature = selected_path.as_ref().map(|path|
                payload.path_signatures.get(path).cloned().unwrap_or_else(|| "clean".to_owned()));
            if last.as_ref() != Some(&payload) {
              last = Some(payload.clone());
            let json = serialize_git_watch_event(serde_json::json!({
                "type": "git.watch.status",
                "properties": payload,
            }), "status");
            yield Ok::<Event, Infallible>(Event::default().event("status").data(json));
            }
            tokio::select! {
                result = changes.changed() => { if result.is_err() { break; } }
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
            }
        }
    };

    let keep = KeepAlive::new()
        .interval(Duration::from_secs(15))
        .text("ping");
    Sse::new(stream).keep_alive(keep).into_response()
}

#[cfg(test)]
mod watch_tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn watch_streams_share_idle_snapshots_and_detect_content_changes_in_already_dirty_files()
    {
        let directory = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(directory.path()).unwrap();
        std::fs::write(directory.path().join("file.txt"), "before").unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let hub = git_watch_hub(
            root.clone(),
            repo.path().to_path_buf(),
            repo.commondir().to_path_buf(),
        );
        let same = git_watch_hub(
            root,
            repo.path().to_path_buf(),
            repo.commondir().to_path_buf(),
        );
        assert!(
            Arc::ptr_eq(&hub, &same),
            "one repository must own one watcher/cache"
        );
        drop(same);
        let query = || {
            Query(GitWatchQuery {
                directory: Some(directory.path().to_string_lossy().into_owned()),
                path: None,
                interval_ms: Some(1000),
            })
        };
        let mut first = git_watch(query()).await.into_body().into_data_stream();
        let mut second = git_watch(query()).await.into_body().into_data_stream();
        let a = tokio::time::timeout(Duration::from_secs(5), first.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let checked = hub.snapshot.lock().await.as_ref().unwrap().checked_at;
        let b = tokio::time::timeout(Duration::from_secs(5), second.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(
            checked,
            hub.snapshot.lock().await.as_ref().unwrap().checked_at,
            "the second consumer must reuse the computed snapshot"
        );
        let idle = tokio::time::timeout(Duration::from_millis(2200), first.next()).await;
        assert!(idle.is_err(), "an idle stream sends no status payloads");
        assert_eq!(
            checked,
            hub.snapshot.lock().await.as_ref().unwrap().checked_at,
            "idle watching must not run periodic 1.5s status scans"
        );
        std::fs::write(
            directory.path().join("file.txt"),
            "after-with-different-content-and-size",
        )
        .unwrap();
        // Restricted macOS runners can construct FSEvents watches without
        // receiving callbacks. The shared 60s reconciliation must still find
        // the change, while native delivery normally completes immediately.
        let updated = tokio::time::timeout(Duration::from_secs(65), async {
            loop {
                let bytes = first
                    .next()
                    .await
                    .expect("the watch stream stays open")
                    .unwrap();
                // Axum sends SSE keep-alive comments independently of status.
                if std::str::from_utf8(&bytes)
                    .unwrap()
                    .lines()
                    .any(|line| line.starts_with("data:"))
                {
                    break bytes;
                }
            }
        })
        .await
        .unwrap_or_else(|error| {
            panic!(
                "Git change was not observed: {error}; native watcher: {}; revision: {}",
                hub._watcher.is_some(),
                *hub.changes.borrow()
            )
        });
        let parse = |bytes: &[u8]| {
            let text = std::str::from_utf8(bytes).unwrap();
            let data = text
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap();
            serde_json::from_str::<serde_json::Value>(data).unwrap()
        };
        let old = parse(&a);
        let new = parse(&updated);
        assert_eq!(new["properties"]["totalFiles"], 1);
        assert_ne!(
            old["properties"]["worktreeSignature"],
            new["properties"]["worktreeSignature"]
        );
    }
}
