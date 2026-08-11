use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::{DirectoryQuery, is_safe_repo_rel_path, require_directory, spawn_libgit2};

#[derive(Debug, Deserialize)]
/// Query for git blame.
pub struct GitBlameQuery {
    pub directory: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// One blame line.
pub struct GitBlameLine {
    pub line: usize,
    pub hash: String,
    pub author: String,
    pub author_email: String,
    pub author_time: i64,
    pub summary: String,
}

#[derive(Debug, Serialize)]
/// Response of a git blame query.
pub struct GitBlameResponse {
    pub lines: Vec<GitBlameLine>,
}

const NOT_COMMITTED_HASH: &str = "0000000000000000000000000000000000000000";
const NOT_COMMITTED_AUTHOR: &str = "Not Committed Yet";
const NOT_COMMITTED_SUMMARY: &str = "Uncommitted changes";

fn build_uncommitted_blame_lines_from_content(content: &str) -> Vec<GitBlameLine> {
    let line_count = content.lines().count();
    (1..=line_count)
        .map(|line| GitBlameLine {
            line,
            hash: NOT_COMMITTED_HASH.to_string(),
            author: NOT_COMMITTED_AUTHOR.to_string(),
            author_email: String::new(),
            author_time: 0,
            summary: NOT_COMMITTED_SUMMARY.to_string(),
        })
        .collect()
}

fn load_blame(directory: PathBuf, absolute_path: PathBuf) -> Result<Vec<GitBlameLine>, String> {
    let repository = git2::Repository::discover(&directory).map_err(|error| error.to_string())?;
    let workdir = repository
        .workdir()
        .ok_or_else(|| "git blame requires a non-bare worktree".to_owned())?;
    let canonical_workdir = workdir.canonicalize().map_err(|error| error.to_string())?;
    let canonical_path = absolute_path
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let relative_path = canonical_path
        .strip_prefix(&canonical_workdir)
        .map_err(|_| "path resolves outside the repository worktree".to_owned())?;
    let bytes = std::fs::read(&canonical_path).map_err(|error| error.to_string())?;
    let content =
        std::str::from_utf8(&bytes).map_err(|_| "git blame target is not UTF-8 text".to_owned())?;

    if repository.is_empty().map_err(|error| error.to_string())? {
        return Ok(build_uncommitted_blame_lines_from_content(content));
    }

    let committed = match repository.blame_file(relative_path, None) {
        Ok(blame) => blame,
        Err(error) if error.code() == git2::ErrorCode::NotFound => {
            return Ok(build_uncommitted_blame_lines_from_content(content));
        }
        Err(error) => return Err(error.to_string()),
    };
    let blame = committed
        .blame_buffer(&bytes)
        .map_err(|error| error.to_string())?;
    let mut lines = Vec::new();
    for hunk in blame.iter() {
        let oid = hunk.final_commit_id();
        let (author, author_email, author_time, summary) = if oid.is_zero() {
            (
                NOT_COMMITTED_AUTHOR.to_owned(),
                String::new(),
                0,
                NOT_COMMITTED_SUMMARY.to_owned(),
            )
        } else {
            let signature = hunk.final_signature();
            (
                signature
                    .as_ref()
                    .map(|value| String::from_utf8_lossy(value.name_bytes()).into_owned())
                    .unwrap_or_default(),
                signature
                    .as_ref()
                    .map(|value| String::from_utf8_lossy(value.email_bytes()).into_owned())
                    .unwrap_or_default(),
                signature.map(|value| value.when().seconds()).unwrap_or(0),
                hunk.summary().ok().flatten().unwrap_or_default().to_owned(),
            )
        };
        for line in
            hunk.final_start_line()..hunk.final_start_line().saturating_add(hunk.lines_in_hunk())
        {
            lines.push(GitBlameLine {
                line,
                hash: if oid.is_zero() {
                    NOT_COMMITTED_HASH.to_owned()
                } else {
                    oid.to_string()
                },
                author: author.clone(),
                author_email: author_email.clone(),
                author_time,
                summary: summary.clone(),
            });
        }
    }
    lines.sort_by_key(|line| line.line);
    Ok(lines)
}

pub async fn git_blame(Query(q): Query<GitBlameQuery>) -> Response {
    let dir = match require_directory(&DirectoryQuery {
        directory: q.directory.clone(),
    }) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    let Some(path) = q
        .path
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "path is required", "code": "missing_path"})),
        )
            .into_response();
    };

    if !is_safe_repo_rel_path(path) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid path", "code": "invalid_path"})),
        )
            .into_response();
    }

    let abs = dir.join(path);
    if !abs.starts_with(&dir) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid path", "code": "invalid_path"})),
        )
            .into_response();
    }

    match spawn_libgit2(move || load_blame(dir, abs)).await {
        Ok(Ok(lines)) => Json(GitBlameResponse { lines }).into_response(),
        Ok(Err(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error, "code": "git_blame_failed"})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("git blame worker failed: {error}"),
                "code": "git_blame_worker_failed"
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{IndexAddOption, Signature};
    use std::path::Path;

    #[test]
    fn blame_marks_worktree_changes_as_uncommitted() {
        let workspace = tempfile::tempdir().expect("create temporary repository");
        let repository = git2::Repository::init(workspace.path()).expect("initialize repository");
        let file_path = workspace.path().join("notes.txt");
        std::fs::write(&file_path, "first\nsecond\n").expect("write committed file");

        let mut index = repository.index().expect("open index");
        index
            .add_all([Path::new("notes.txt")], IndexAddOption::DEFAULT, None)
            .expect("add file to index");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repository.find_tree(tree_id).expect("find tree");
        let signature =
            Signature::now("Agena Test", "agena@example.invalid").expect("create commit signature");
        let commit_id = repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "initial content",
                &tree,
                &[],
            )
            .expect("commit file");
        drop(tree);
        drop(repository);

        std::fs::write(&file_path, "first\nchanged\nnew\n").expect("modify worktree file");
        let lines = load_blame(workspace.path().to_path_buf(), file_path).expect("load blame");

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].hash, commit_id.to_string());
        assert_eq!(lines[0].author, "Agena Test");
        assert_eq!(lines[1].hash, NOT_COMMITTED_HASH);
        assert_eq!(lines[1].summary, NOT_COMMITTED_SUMMARY);
        assert_eq!(lines[2].hash, NOT_COMMITTED_HASH);
    }

    #[test]
    fn blame_supports_new_untracked_files() {
        let workspace = tempfile::tempdir().expect("create temporary repository");
        git2::Repository::init(workspace.path()).expect("initialize repository");
        let file_path = workspace.path().join("new.txt");
        std::fs::write(&file_path, "one\ntwo\n").expect("write untracked file");

        let lines = load_blame(workspace.path().to_path_buf(), file_path).expect("load blame");

        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.hash == NOT_COMMITTED_HASH));
    }
}
