use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::{
    DirectoryQuery, GitBranchProtectionPrompt, GitCommitSummary, git_allow_no_verify_commit,
    git_branch_protection_for_branch, git_command_transport_error_response, git_config_get,
    git_enforce_branch_protection, git_io_error_response, map_git_failure, redact_git_output,
    require_directory, require_locked_directory, run_git, run_git_checked_with_status,
    truncate_for_payload,
};

#[derive(Debug, Deserialize)]
/// Body of a git commit request.
pub struct GitCommitBody {
    pub message: Option<String>,
    #[serde(default, rename = "addAll")]
    pub add_all: bool,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(rename = "gpgPassphrase")]
    pub gpg_passphrase: Option<String>,

    // VS Code-like options.
    #[serde(default, rename = "noVerify")]
    pub no_verify: bool,
    #[serde(default)]
    pub signoff: bool,
    #[serde(default)]
    pub amend: bool,
    #[serde(default, rename = "allowEmpty")]
    pub allow_empty: bool,
    #[serde(default, rename = "noGpgSign")]
    pub no_gpg_sign: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// Result of a git commit.
pub struct GitCommitResult {
    pub success: bool,
    pub commit: String,
    pub branch: String,
    pub summary: GitCommitSummary,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Body of a git undo-commit request.
pub struct GitUndoCommitBody {
    // "soft" (default) | "mixed"
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Body of a git reset request.
pub struct GitResetCommitBody {
    pub commit: Option<String>,
    pub mode: Option<String>,
}

async fn git_path_exists(dir: &Path, name: &str) -> bool {
    let (code, out, _) = match run_git(dir, &["rev-parse", "--git-path", name]).await {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(
                diagnostic = %error,
                marker = name,
                "failed to resolve Git marker path"
            );
            return false;
        }
    };
    if code != 0 {
        return false;
    }
    let raw = out.trim();
    if raw.is_empty() {
        return false;
    }
    let p = PathBuf::from(raw);
    let full = if p.is_absolute() { p } else { dir.join(p) };
    tokio::fs::metadata(full).await.is_ok()
}

async fn git_sequencer_in_progress(dir: &Path) -> bool {
    for name in [
        "MERGE_HEAD",
        "rebase-apply",
        "rebase-merge",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
    ] {
        if git_path_exists(dir, name).await {
            return true;
        }
    }
    false
}

pub async fn git_undo_commit(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitUndoCommitBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    // Disallow undo while sequencer operations are in progress.
    if git_sequencer_in_progress(&dir).await {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Cannot undo commit while a merge/rebase/cherry-pick/revert is in progress",
                "code": "git_undo_not_allowed",
            })),
        )
            .into_response();
    }

    // Ensure there's a parent commit.
    if let Err(resp) = run_git_checked_with_status(
        &dir,
        &["rev-parse", "--verify", "HEAD~1"],
        StatusCode::BAD_REQUEST,
        Some("git_undo_not_possible"),
    )
    .await
    {
        if resp.status() == StatusCode::CONFLICT {
            return resp;
        }
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "No parent commit to undo",
                "code": "git_undo_not_possible",
            })),
        )
            .into_response();
    }

    let mode = body
        .mode
        .as_deref()
        .unwrap_or("soft")
        .trim()
        .to_ascii_lowercase();
    let flag = if mode == "mixed" {
        "--mixed"
    } else if mode == "soft" {
        "--soft"
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid mode", "code": "invalid_mode"})),
        )
            .into_response();
    };

    if let Err(resp) = run_git_checked_with_status(
        &dir,
        &["reset", flag, "HEAD~1"],
        StatusCode::INTERNAL_SERVER_ERROR,
        Some("git_reset_failed"),
    )
    .await
    {
        return resp;
    }

    Json(serde_json::json!({"success": true, "mode": mode})).into_response()
}

pub async fn git_reset_commit(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitResetCommitBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    let Some(commit) = body
        .commit
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "commit is required",
                "code": "missing_commit"
            })),
        )
            .into_response();
    };

    if git_sequencer_in_progress(&dir).await {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "Cannot reset while a merge/rebase/cherry-pick/revert is in progress",
                "code": "git_reset_not_allowed",
            })),
        )
            .into_response();
    }

    let mode = body
        .mode
        .as_deref()
        .unwrap_or("mixed")
        .trim()
        .to_ascii_lowercase();
    let flag = if mode == "hard" {
        "--hard"
    } else if mode == "soft" {
        "--soft"
    } else if mode == "mixed" {
        "--mixed"
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid mode", "code": "invalid_mode"})),
        )
            .into_response();
    };

    if let Err(resp) = run_git_checked_with_status(
        &dir,
        &["reset", flag, commit],
        StatusCode::INTERNAL_SERVER_ERROR,
        Some("git_reset_failed"),
    )
    .await
    {
        return resp;
    }

    Json(serde_json::json!({"success": true, "mode": mode, "commit": commit})).into_response()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// Response carrying the commit template.
pub struct GitCommitTemplateResponse {
    pub configured: bool,
    pub path: Option<String>,
    pub template: Option<String>,
}

pub async fn git_commit_template(Query(q): Query<DirectoryQuery>) -> Response {
    let dir = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let global = git_config_get(None, "--global", "commit.template").await;
    let raw = git_config_get(Some(&dir), "--local", "commit.template")
        .await
        .or(global);
    let Some(p) = raw else {
        return Json(GitCommitTemplateResponse {
            configured: false,
            path: None,
            template: None,
        })
        .into_response();
    };

    let mut path = PathBuf::from(p.trim());
    if !path.is_absolute() {
        path = dir.join(path);
    }

    let meta = match tokio::fs::metadata(&path).await {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Json(GitCommitTemplateResponse {
                configured: true,
                path: Some(path.to_string_lossy().into_owned()),
                template: None,
            })
            .into_response();
        }
        Err(error) => {
            return git_io_error_response(
                "failed to inspect configured Git commit template",
                &error,
                "commit_template_metadata_failed",
            );
        }
    };
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return Json(GitCommitTemplateResponse {
            configured: true,
            path: Some(path.to_string_lossy().into_owned()),
            template: None,
        })
        .into_response();
    }

    let template = match tokio::fs::read_to_string(&path).await {
        Ok(template) => template,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Json(GitCommitTemplateResponse {
                configured: true,
                path: Some(path.to_string_lossy().into_owned()),
                template: None,
            })
            .into_response();
        }
        Err(error) => {
            return git_io_error_response(
                "failed to read configured Git commit template",
                &error,
                "commit_template_read_failed",
            );
        }
    };
    let t = template.trim_end().to_string();
    Json(GitCommitTemplateResponse {
        configured: true,
        path: Some(path.to_string_lossy().into_owned()),
        template: Some(t),
    })
    .into_response()
}

fn parse_commit_hash(stdout: &str) -> String {
    // Capture last commit hash.
    stdout.trim().to_string()
}

fn post_commit_git_output(
    result: Result<(i32, String, String), String>,
    operation: &str,
) -> Option<String> {
    match result {
        Ok((0, output, _)) => Some(output),
        Ok((exit_code, output, error)) => {
            tracing::warn!(
                operation,
                exit_code,
                stdout = %redact_git_output(&truncate_for_payload(&output, 4_000)),
                stderr = %redact_git_output(&truncate_for_payload(&error, 4_000)),
                "post-commit Git metadata command failed"
            );
            None
        }
        Err(error) => {
            tracing::error!(
                operation,
                diagnostic = %error,
                "post-commit Git metadata process failed"
            );
            None
        }
    }
}

fn parse_shortstat(lines: &[&str]) -> (i32, i32, i32) {
    // " 2 files changed, 10 insertions(+), 1 deletion(-)"
    let mut files = 0;
    let mut ins = 0;
    let mut del = 0;
    for line in lines {
        if let Some(pos) = line.find("files changed") {
            let num = line[..pos].split_whitespace().last().unwrap_or("0");
            files = num.parse::<i32>().unwrap_or(0);
        }
        if let Some(pos) = line.find("insertions") {
            let num = line[..pos].split_whitespace().last().unwrap_or("0");
            ins = num.parse::<i32>().unwrap_or(0);
        }
        if let Some(pos) = line.find("deletions") {
            let num = line[..pos].split_whitespace().last().unwrap_or("0");
            del = num.parse::<i32>().unwrap_or(0);
        }
    }
    (files, ins, del)
}

pub async fn git_commit<S: crate::GitHttpState + 'static>(
    State(state): State<Arc<S>>,
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitCommitBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let Some(message) = body
        .message
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "message is required", "code": "missing_message"})),
        )
            .into_response();
    };

    if body.no_verify && !git_allow_no_verify_commit(&state).await {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Commits without verification are disabled by policy",
                "code": "git_no_verify_not_allowed",
                "hint": "Enable gitAllowNoVerifyCommit in settings if this is intentional.",
            })),
        )
            .into_response();
    }

    if git_enforce_branch_protection(&state).await
        && let Some(branch) = super::remote::git_current_branch(&dir).await
        && let Some(prompt_mode) = git_branch_protection_for_branch(&state, &branch).await
        && prompt_mode == GitBranchProtectionPrompt::CommitToNewBranch
    {
        return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": format!("Branch '{branch}' is protected; commit on a new branch instead."),
                    "code": "git_branch_protected",
                    "branch": branch,
                    "promptMode": prompt_mode.as_str(),
                    "category": "policy",
                    "hint": "Create a new branch and commit there, or change gitBranchProtectionPrompt in settings.",
                })),
            )
                .into_response();
    }

    // If the repo uses GPG signing and the key is passphrase protected, we can't prompt
    // in a server context. Instead, accept an optional passphrase from the UI and
    // preset it into gpg-agent so signing can proceed non-interactively.
    if let Some(pp) = body
        .gpg_passphrase
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        let signing_key = git_config_get(Some(&dir), "--local", "user.signingkey").await;
        // First query keys so we can return a more specific error if gpg is unavailable.
        let keys = match super::gpg::gpg_list_keys_for_signing().await {
            Ok(k) => k,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Failed to query GPG secret keys: {e}"),
                        "code": "gpg_keys_unavailable",
                        "hint": "Ensure gpg is installed and your secret key exists on this machine.",
                    })),
                )
                    .into_response();
            }
        };
        if keys.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "No GPG secret key with keygrip found",
                    "code": "gpg_no_secret_key",
                    "hint": "Import your secret key and/or set user.signingkey in this repository.",
                })),
            )
                .into_response();
        }
        if let Err(e) = super::gpg::gpg_preset_for_signing(signing_key.as_deref(), pp).await {
            let code = if e.to_ascii_lowercase().contains("no gpg secret key") {
                "gpg_no_secret_key"
            } else {
                "gpg_preset_failed"
            };
            let mut body = serde_json::json!({
                "error": format!("Failed to preset GPG passphrase: {e}"),
                "code": code,
            });
            if code == "gpg_preset_failed" {
                body["hint"] = serde_json::Value::String(
                    "Your gpg-agent may not allow presetting passphrases. You can enable allow-preset-passphrase from the UI and retry."
                        .to_string(),
                );
                body["canEnablePreset"] = serde_json::Value::Bool(true);
            }
            return (StatusCode::BAD_REQUEST, Json(body)).into_response();
        }
    }

    if body.add_all {
        let (c, o, e) = match run_git(&dir, &["add", "-A"]).await {
            Ok(result) => result,
            Err(error) => {
                return git_command_transport_error_response(
                    "stage all files before commit",
                    &error,
                    Some("git_add_process_failed"),
                );
            }
        };
        if c != 0 {
            if let Some(resp) = map_git_failure(c, &o, &e) {
                return resp;
            }
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": e.trim(),
                    "stdout": o,
                    "stderr": e,
                    "code": "git_add_failed"
                })),
            )
                .into_response();
        }
    } else if !body.files.is_empty() {
        let mut args: Vec<&str> = vec!["add", "--"];
        for f in &body.files {
            args.push(f);
        }
        let (c, o, e) = match run_git(&dir, &args).await {
            Ok(result) => result,
            Err(error) => {
                return git_command_transport_error_response(
                    "stage selected files before commit",
                    &error,
                    Some("git_add_process_failed"),
                );
            }
        };
        if c != 0 {
            if let Some(resp) = map_git_failure(c, &o, &e) {
                return resp;
            }
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.trim()})),
            )
                .into_response();
        }
    }

    // Commit.
    let mut commit_args: Vec<&str> = vec!["commit"];
    if body.no_verify {
        commit_args.push("--no-verify");
    }
    if body.signoff {
        commit_args.push("--signoff");
    }
    if body.amend {
        commit_args.push("--amend");
    }
    if body.allow_empty {
        commit_args.push("--allow-empty");
    }
    if body.no_gpg_sign {
        commit_args.push("--no-gpg-sign");
    }
    commit_args.extend(["-m", message]);
    if !body.add_all && !body.files.is_empty() {
        commit_args.push("--");
        for f in &body.files {
            commit_args.push(f);
        }
    }
    let (c, o, e) = match run_git(&dir, &commit_args).await {
        Ok(result) => result,
        Err(error) => {
            return git_command_transport_error_response(
                "create Git commit",
                &error,
                Some("git_commit_process_failed"),
            );
        }
    };
    if c != 0 {
        if let Some(resp) = map_git_failure(c, &o, &e) {
            return resp;
        }
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": truncate_for_payload(&redact_git_output(&e), 8_000),
                "code": "git_commit_failed",
                "stdout": redact_git_output(&truncate_for_payload(&o, 16_000)),
                "stderr": redact_git_output(&truncate_for_payload(&e, 16_000)),
                "exitCode": c,
            })),
        )
            .into_response();
    }

    // Return commit hash + branch.
    let commit = post_commit_git_output(
        run_git(&dir, &["rev-parse", "HEAD"]).await,
        "resolve resulting commit hash",
    )
    .map(|output| parse_commit_hash(&output))
    .unwrap_or_default();
    let branch = post_commit_git_output(
        run_git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]).await,
        "resolve resulting commit branch",
    )
    .map(|output| output.trim().to_string())
    .unwrap_or_default();

    // Best-effort summary from last commit.
    let (files_changed, insertions, deletions) = post_commit_git_output(
        run_git(&dir, &["show", "--shortstat", "--format=", "HEAD"]).await,
        "read resulting commit short-stat",
    )
    .map(|output| parse_shortstat(&output.lines().collect::<Vec<_>>()))
    .unwrap_or_default();

    Json(GitCommitResult {
        success: true,
        commit,
        branch,
        summary: GitCommitSummary {
            changes: files_changed,
            insertions,
            deletions,
        },
    })
    .into_response()
}
