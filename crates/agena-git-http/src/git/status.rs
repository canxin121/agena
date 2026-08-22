use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::path::Path;
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
                .include_unmodified(false);

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
    {
        if let Some((code, stdout, stderr)) = git_command_result_or_log(
            run_git(&dir, &["rev-list", "--count", &format!("{base}..HEAD")]).await,
            "estimate unpublished Git commit count",
        ) {
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
    is_clean: bool,
    worktree_signature: String,
}

pub async fn git_watch(Query(q): Query<GitWatchQuery>) -> Response {
    let dir = match require_directory_raw(q.directory.as_deref()) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    // Validate repository early so the client gets a normal JSON response.
    let probe = spawn_libgit2({
        let dir = dir.clone();
        move || git2_utils::open_repo_discover(&dir).map(|_| ())
    })
    .await;
    match probe {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return git2_open_error_response(e),
        Err(error) => return git_task_error_response("validate the Git watch repository", &error),
    }

    let interval_ms = q.interval_ms.unwrap_or(1500).clamp(500, 10_000);
    let stream = async_stream::stream! {
        let mut last: Option<GitWatchStatusPayload> = None;
        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));

        loop {
            ticker.tick().await;

            let snapshot = spawn_libgit2({
                let dir = dir.clone();
                move || -> Result<GitWatchStatusPayload, git2_utils::Git2OpenError> {
                    use git2::{Status, StatusOptions};

                    let repo = git2_utils::open_repo_discover(&dir)?;

                    let (current, tracking, ahead, behind) = read_branch_state(&repo)?;

                    let mut opts = StatusOptions::new();
                    opts.include_untracked(true)
                        .recurse_untracked_dirs(true)
                        .include_ignored(false)
                        .include_unmodified(false);

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

                    for entry in statuses.iter() {
                        let path = entry.path().map_err(|error| {
                            git2_other("Git watch status path is not valid UTF-8", &error)
                        })?;
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

                        fnv1a64_update(&mut worktree_signature, x.as_bytes());
                        fnv1a64_update(&mut worktree_signature, b"|");
                        fnv1a64_update(&mut worktree_signature, y.as_bytes());
                        fnv1a64_update(&mut worktree_signature, b"|");
                        fnv1a64_update(&mut worktree_signature, path.as_bytes());
                        fnv1a64_update(&mut worktree_signature, b"\n");

                        total_files += 1;

                        let is_merge = x == "U" || y == "U";
                        let is_untracked = x == "?" && y == "?";
                        let is_staged = !is_merge && !x.is_empty() && x != "?";
                        let is_unstaged = !is_merge && !is_untracked && !y.is_empty();

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

                    Ok(GitWatchStatusPayload {
                        current,
                        tracking,
                        ahead,
                        behind,
                        staged_count,
                        unstaged_count,
                        untracked_count,
                        merge_count,
                        is_clean: total_files == 0,
                        worktree_signature: format!("{worktree_signature:016x}"),
                    })
                }
            })
            .await;

            let payload = match snapshot {
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

            if last.as_ref().is_some_and(|prev| prev == &payload) {
                continue;
            }
            last = Some(payload.clone());

            let json = serialize_git_watch_event(serde_json::json!({
                "type": "git.watch.status",
                "properties": payload,
            }), "status");
            yield Ok::<Event, Infallible>(Event::default().event("status").data(json));
        }
    };

    let keep = KeepAlive::new()
        .interval(Duration::from_secs(15))
        .text("ping");
    Sse::new(stream).keep_alive(keep).into_response()
}
