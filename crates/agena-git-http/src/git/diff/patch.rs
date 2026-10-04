use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::sync::Arc;

use super::super::{
    DirectoryQuery, abs_path, git_command_transport_error_response, git_strict_patch_validation,
    is_safe_repo_rel_path, map_git_failure, require_locked_directory, run_git, run_git_with_input,
};
use super::unified::{parse_unified_diff_meta, patch_paths_are_safe, validate_unified_patch_hunks};

#[derive(Debug, Deserialize)]
/// Query for a git diff.
pub struct GitDiffQuery {
    pub directory: Option<String>,
    pub path: Option<String>,
    #[serde(rename = "oldPath")]
    pub old_path: Option<String>,
    pub staged: Option<String>,
    #[serde(rename = "contextLines")]
    pub context_lines: Option<String>,
    #[serde(rename = "includeMeta")]
    pub include_meta: Option<String>,
    /// Optional preview budget; full Git editors retain the existing behavior.
    #[serde(rename = "maxBytes")]
    pub max_bytes: Option<usize>,
}

pub async fn git_diff(Query(q): Query<GitDiffQuery>) -> Response {
    let Some(dir_raw) = q.directory.as_deref() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "directory parameter is required"})),
        )
            .into_response();
    };
    let dir = match abs_path(dir_raw) {
        Ok(dir) => dir,
        Err(response) => return *response,
    };
    let Some(path) = q
        .path
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "path parameter is required"})),
        )
            .into_response();
    };

    if !is_safe_repo_rel_path(path)
        || q.old_path
            .as_deref()
            .is_some_and(|p| !is_safe_repo_rel_path(p))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid path", "code": "invalid_path"})),
        )
            .into_response();
    }

    let staged = q.staged.as_deref().map(|v| v == "true").unwrap_or(false);
    let context = q
        .context_lines
        .as_deref()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(3)
        .clamp(0, 500) as u32;

    // Use the git CLI for patch output so the result is a valid unified diff.
    // (libgit2's line callbacks do not include +/-/space prefixes in `content()`.)
    let mut untracked = false;
    if !staged {
        let (c, o, error) = match run_git(
            &dir,
            &["ls-files", "--others", "--exclude-standard", "--", path],
        )
        .await
        {
            Ok(result) => result,
            Err(error) => {
                return git_command_transport_error_response(
                    "check whether Git diff target is untracked",
                    &error,
                    Some("git_diff_track_check_process_failed"),
                );
            }
        };
        if c == 0 {
            let listed = o
                .lines()
                .map(|l| l.trim())
                .any(|l| !l.is_empty() && l == path);
            untracked = listed;
        } else if let Some(response) = map_git_failure(c, &o, &error) {
            return response;
        }
    }

    let mut args: Vec<String> = Vec::new();
    args.push("diff".into());
    if staged {
        args.push("--cached".into());
    }
    args.push(format!("-U{}", context));
    if untracked {
        // `git diff` does not show content for untracked files; use --no-index to render
        // a patch against /dev/null.
        args.push("--no-index".into());
        args.push("--".into());
        args.push("/dev/null".into());
        args.push(path.to_string());
    } else {
        args.push("--".into());
        if let Some(old_path) = &q.old_path {
            args.push(old_path.clone());
        }
        args.push(path.to_string());
    }
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (code, out, err) = match run_git(&dir, &args_ref).await {
        Ok(result) => result,
        Err(error) => {
            return git_command_transport_error_response(
                "render Git file diff",
                &error,
                Some("git_diff_process_failed"),
            );
        }
    };

    // `git diff --no-index` returns 1 when differences exist. Treat that as success.
    let ok = if untracked {
        code == 0 || code == 1
    } else {
        code == 0
    };
    if !ok {
        if let Some(resp) = map_git_failure(code, &out, &err) {
            return resp;
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": err.trim(), "code": "git_diff_failed"})),
        )
            .into_response();
    }

    let include_meta = q
        .include_meta
        .as_deref()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);

    if include_meta {
        let meta = parse_unified_diff_meta(&out);
        return Json(serde_json::json!({"diff": out, "meta": meta})).into_response();
    }

    let total_bytes = out.len();
    let (preview, truncated) = diff_preview(&out, q.max_bytes);
    Json(serde_json::json!({"diff": preview, "truncated": truncated, "totalBytes": total_bytes}))
        .into_response()
}

fn diff_preview(diff: &str, max_bytes: Option<usize>) -> (&str, bool) {
    let Some(limit) = max_bytes else {
        return (diff, false);
    };
    let limit = limit.clamp(1024, 16 * 1024 * 1024);
    if diff.len() <= limit {
        return (diff, false);
    }
    let mut end = limit;
    while !diff.is_char_boundary(end) {
        end -= 1;
    }
    let preview = &diff[..end];
    (
        preview
            .rfind('\n')
            .map_or(preview, |newline| &preview[..newline + 1]),
        true,
    )
}

#[cfg(test)]
mod preview_tests {
    use super::diff_preview;
    #[test]
    fn preview_is_bounded_on_utf8_and_line_boundaries_and_can_expand() {
        let diff = format!(
            "--- a/文.rs\n+++ b/文.rs\n@@ -1,1 +1,1 @@\n-{}\n+new\n",
            "中文".repeat(700)
        );
        let (preview, truncated) = diff_preview(&diff, Some(1024));
        assert!(truncated);
        assert!(preview.len() <= 1024);
        assert!(preview.ends_with('\n'));
        assert_eq!(diff_preview(&diff, Some(8192)), (diff.as_str(), false));
        assert_eq!(diff_preview(&diff, None), (diff.as_str(), false));
    }

    #[tokio::test]
    async fn workspace_status_and_diff_handle_paging_renames_untracked_and_binary() {
        use super::{GitDiffQuery, Query, git_diff};
        use crate::git::status::{GitStatusQuery, git_status};
        async fn json(response: axum::response::Response) -> serde_json::Value {
            assert!(response.status().is_success(), "{}", response.status());
            serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap(),
            )
            .unwrap()
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let repo = git2::Repository::init(root).unwrap();
        std::fs::write(root.join("before 文.txt"), "first\nsecond\n").unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_path(std::path::Path::new("before 文.txt"))
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let author = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
        repo.commit(Some("HEAD"), &author, &author, "fixture", &tree, &[])
            .unwrap();
        std::fs::rename(root.join("before 文.txt"), root.join("after 文.txt")).unwrap();
        index
            .remove_path(std::path::Path::new("before 文.txt"))
            .unwrap();
        index
            .add_path(std::path::Path::new("after 文.txt"))
            .unwrap();
        index.write().unwrap();
        for i in 0..45 {
            std::fs::write(root.join(format!("new-{i:02}.txt")), "untracked text\n").unwrap();
        }
        std::fs::write(root.join("binary.bin"), b"\0binary\0").unwrap();
        let directory = root.to_string_lossy().to_string();
        let status = |offset, summary| GitStatusQuery {
            directory: Some(directory.clone()),
            offset: Some(offset),
            limit: Some(40),
            scope: None,
            summary,
            include_diff_stats: false,
        };
        let first = json(git_status(Query(status(0, false))).await).await;
        assert_eq!(first["files"].as_array().unwrap().len(), 40);
        assert_eq!(first["hasMore"], true);
        let renamed = first["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == "after 文.txt")
            .unwrap();
        assert_eq!(renamed["indexOldPath"], "before 文.txt");
        let second = json(git_status(Query(status(40, false))).await).await;
        assert_eq!(second["hasMore"], false);
        let summary = json(git_status(Query(status(0, true))).await).await;
        assert_eq!(summary["files"].as_array().unwrap().len(), 0);
        assert_eq!(summary["totalFiles"], first["totalFiles"]);
        let query = |path: &str, staged: bool, old_path: Option<&str>| GitDiffQuery {
            directory: Some(directory.clone()),
            path: Some(path.into()),
            old_path: old_path.map(str::to_owned),
            staged: Some(staged.to_string()),
            context_lines: Some("3".into()),
            include_meta: None,
            max_bytes: Some(262144),
        };
        let renamed =
            json(git_diff(Query(query("after 文.txt", true, Some("before 文.txt")))).await).await;
        assert!(renamed["diff"].as_str().unwrap().contains("rename from"));
        let added = json(git_diff(Query(query("new-00.txt", false, None))).await).await;
        assert!(added["diff"].as_str().unwrap().contains("+untracked text"));
        let binary = json(git_diff(Query(query("binary.bin", false, None))).await).await;
        assert!(binary["diff"].as_str().unwrap().contains("Binary files"));
        assert_eq!(
            git_diff(Query(query("after 文.txt", true, Some("../outside"))))
                .await
                .status(),
            axum::http::StatusCode::BAD_REQUEST
        );
    }
}

#[derive(Debug, Deserialize)]
/// Body of a patch application request.
pub struct GitApplyPatchBody {
    pub patch: Option<String>,
    pub mode: Option<String>,
    // Optional granularity hint from the client: "file" | "hunk" | "selected"
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatchTarget {
    File,
    Hunk,
    Selected,
}

#[derive(Debug, Clone, Copy)]
struct PatchMode {
    cached: bool,
    reverse: bool,
    three_way: bool,
    default_target: Option<PatchTarget>,
    strict_hunk_validation: bool,
}

fn parse_patch_target(value: Option<&str>) -> Result<Option<PatchTarget>, &'static str> {
    let Some(raw) = value.map(|v| v.trim()).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let t = raw.to_ascii_lowercase();
    let parsed = match t.as_str() {
        "file" => PatchTarget::File,
        "hunk" => PatchTarget::Hunk,
        "selected" | "range" | "selection" => PatchTarget::Selected,
        _ => return Err("invalid_patch_target"),
    };
    Ok(Some(parsed))
}

fn parse_patch_mode(mode: &str) -> Option<PatchMode> {
    let normalized = mode.trim().to_ascii_lowercase().replace('_', "-");
    match normalized.as_str() {
        "stage" => Some(PatchMode {
            cached: true,
            reverse: false,
            three_way: false,
            default_target: None,
            strict_hunk_validation: true,
        }),
        "stage-hunk" => Some(PatchMode {
            cached: true,
            reverse: false,
            three_way: false,
            default_target: Some(PatchTarget::Hunk),
            strict_hunk_validation: true,
        }),
        "stage-selected" | "stage-selection" | "stage-range" => Some(PatchMode {
            cached: true,
            reverse: false,
            three_way: false,
            default_target: Some(PatchTarget::Selected),
            strict_hunk_validation: true,
        }),
        "unstage" => Some(PatchMode {
            cached: true,
            reverse: true,
            three_way: false,
            default_target: None,
            strict_hunk_validation: true,
        }),
        "unstage-hunk" => Some(PatchMode {
            cached: true,
            reverse: true,
            three_way: false,
            default_target: Some(PatchTarget::Hunk),
            strict_hunk_validation: true,
        }),
        "unstage-selected" | "unstage-selection" | "unstage-range" => Some(PatchMode {
            cached: true,
            reverse: true,
            three_way: false,
            default_target: Some(PatchTarget::Selected),
            strict_hunk_validation: true,
        }),
        "discard" => Some(PatchMode {
            cached: false,
            reverse: true,
            three_way: false,
            default_target: None,
            strict_hunk_validation: true,
        }),
        "discard-hunk" => Some(PatchMode {
            cached: false,
            reverse: true,
            three_way: false,
            default_target: Some(PatchTarget::Hunk),
            strict_hunk_validation: true,
        }),
        "discard-selected" | "discard-selection" | "discard-range" => Some(PatchMode {
            cached: false,
            reverse: true,
            three_way: false,
            default_target: Some(PatchTarget::Selected),
            strict_hunk_validation: true,
        }),
        "apply" => Some(PatchMode {
            cached: false,
            reverse: false,
            three_way: false,
            default_target: None,
            strict_hunk_validation: false,
        }),
        "apply-3way" | "apply-three-way" | "apply3way" => Some(PatchMode {
            cached: false,
            reverse: false,
            three_way: true,
            default_target: None,
            strict_hunk_validation: false,
        }),
        _ => None,
    }
}

fn invalid_patch_response(
    code: &'static str,
    error: &'static str,
    hint: Option<&'static str>,
) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": error,
            "code": code,
            "hint": hint,
            "category": "validation",
        })),
    )
        .into_response()
}

pub async fn git_apply_patch<S: crate::GitHttpState + 'static>(
    State(state): State<Arc<S>>,
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitApplyPatchBody>,
) -> Response {
    let (dir, _guard) = match require_locked_directory(&q).await {
        Ok(value) => value,
        Err(resp) => return resp,
    };

    let Some(raw) = body
        .patch
        .as_deref()
        .map(|s| s.trim_end())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "patch is required", "code": "missing_patch"})),
        )
            .into_response();
    };

    const MAX_PATCH_BYTES: usize = 1024 * 1024;
    if raw.len() > MAX_PATCH_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "Patch too large", "code": "patch_too_large"})),
        )
            .into_response();
    }

    if !patch_paths_are_safe(raw) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid patch paths", "code": "invalid_patch"})),
        )
            .into_response();
    }

    let mode_raw = body.mode.as_deref().unwrap_or("stage").trim().to_string();
    let Some(mode) = parse_patch_mode(&mode_raw) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid mode", "code": "invalid_mode"})),
        )
            .into_response();
    };

    let explicit_target = match parse_patch_target(body.target.as_deref()) {
        Ok(v) => v,
        Err(error_code) => {
            return invalid_patch_response(
                error_code,
                "Invalid patch target",
                Some("Use one of: file, hunk, selected."),
            );
        }
    };
    let target = if let Some(default_target) = mode.default_target {
        if let Some(explicit) = explicit_target
            && explicit != default_target
        {
            return invalid_patch_response(
                "patch_mode_target_mismatch",
                "Patch mode and target are inconsistent",
                Some("Use a matching mode/target pair (for example stage-selected + selected)."),
            );
        }
        default_target
    } else {
        explicit_target.unwrap_or(PatchTarget::File)
    };

    if mode.strict_hunk_validation && git_strict_patch_validation(&state).await {
        let summary = match validate_unified_patch_hunks(raw) {
            Ok(s) => s,
            Err("invalid_patch_hunk_counts") => {
                return invalid_patch_response(
                    "invalid_patch_hunk_counts",
                    "Patch hunk headers do not match patch content",
                    Some("Refresh the diff and retry; stale patches can fail after file changes."),
                );
            }
            Err("invalid_patch_hunk_line") => {
                return invalid_patch_response(
                    "invalid_patch_hunk_line",
                    "Patch contains invalid hunk lines",
                    Some("Only unified diff lines (+, -, and context) are allowed inside hunks."),
                );
            }
            Err(code) => {
                return invalid_patch_response(
                    code,
                    "Patch is not a valid unified diff",
                    Some("Generate the patch from the current diff and retry."),
                );
            }
        };

        if summary.files != 1 {
            return invalid_patch_response(
                "patch_requires_single_file",
                "Patch must target exactly one file",
                Some("Split multi-file patches into one request per file."),
            );
        }
        if matches!(target, PatchTarget::Hunk | PatchTarget::Selected) && summary.hunks != 1 {
            return invalid_patch_response(
                "patch_requires_single_hunk",
                "Patch must target exactly one hunk",
                Some("Split multi-hunk patches into separate requests."),
            );
        }
    }

    let mut args: Vec<&str> = vec!["apply", "--whitespace=nowarn"];
    if mode.cached {
        args.push("--cached");
    }
    if mode.reverse {
        args.push("--reverse");
    }
    if mode.three_way {
        args.push("--3way");
    }

    let patch = if raw.ends_with('\n') {
        raw.to_string()
    } else {
        format!("{}\n", raw)
    };

    let (code, out, err) = match run_git_with_input(&dir, &args, &patch).await {
        Ok(result) => result,
        Err(error) => {
            return git_command_transport_error_response(
                "apply Git patch",
                &error,
                Some("git_apply_process_failed"),
            );
        }
    };
    if code != 0 {
        if let Some(resp) = map_git_failure(code, &out, &err) {
            return resp;
        }
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": err.trim(), "code": "git_apply_failed"})),
        )
            .into_response();
    }

    Json(serde_json::json!({"success": true})).into_response()
}
