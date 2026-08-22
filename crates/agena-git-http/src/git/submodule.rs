use std::path::Path;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::git2_utils::{self, Git2OpenError};

use super::{
    DirectoryQuery, git_success_response, git2_open_error_response, require_directory,
    run_locked_git_checked, run_locked_git_env_checked, spawn_libgit2,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// Information about a git submodule.
pub struct GitSubmoduleInfo {
    pub path: String,
    pub url: String,
    pub branch: Option<String>,
}

#[derive(Debug, Serialize)]
/// Response listing git submodules.
pub struct GitSubmoduleListResponse {
    pub submodules: Vec<GitSubmoduleInfo>,
}

fn list_submodules(dir: &Path) -> Result<Vec<GitSubmoduleInfo>, Git2OpenError> {
    let repo = git2_utils::open_repo_discover(dir)?;
    let discovered = repo.submodules().map_err(|error| {
        Git2OpenError::Other(crate::git2_utils::git2_error_diagnostic(
            "failed to enumerate Git submodules",
            &error,
        ))
    })?;

    let mut submodules = Vec::with_capacity(discovered.len());
    for submodule in discovered {
        let Some(url) = submodule.url().map_err(|error| {
            Git2OpenError::Other(crate::git2_utils::git2_error_diagnostic(
                "failed to resolve a Git submodule work directory",
                &error,
            ))
        })?
        else {
            continue;
        };
        let branch = submodule
            .branch()
            .map_err(|error| {
                Git2OpenError::Other(crate::git2_utils::git2_error_diagnostic(
                    "failed to open a Git submodule repository",
                    &error,
                ))
            })?
            .map(str::to_string);
        submodules.push(GitSubmoduleInfo {
            path: submodule.path().to_string_lossy().into_owned(),
            url: url.to_string(),
            branch,
        });
    }
    submodules.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(submodules)
}

pub async fn git_submodules(Query(q): Query<DirectoryQuery>) -> Response {
    let dir = match require_directory(&q) {
        Ok(d) => d,
        Err(resp) => return *resp,
    };

    match spawn_libgit2(move || list_submodules(&dir)).await {
        Ok(Ok(submodules)) => Json(GitSubmoduleListResponse { submodules }).into_response(),
        Ok(Err(error)) => git2_open_error_response(error),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("submodule worker failed: {error}"),
                "code": "submodule_worker_failed",
            })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Body of a submodule add request.
pub struct GitSubmoduleAddBody {
    pub url: Option<String>,
    pub path: Option<String>,
    pub branch: Option<String>,
}

pub async fn git_submodule_add(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitSubmoduleAddBody>,
) -> Response {
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
    let Some(path) = body
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

    let mut args: Vec<&str> = vec!["submodule", "add"];
    if let Some(branch) = body
        .branch
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        args.push("-b");
        args.push(branch);
    }
    args.push(url);
    args.push(path);

    if let Err(resp) = run_locked_git_checked(&q, &args, Some("git_submodule_add_failed")).await {
        return resp;
    }

    git_success_response()
}

#[derive(Debug, Deserialize)]
/// Body of a submodule path request.
pub struct GitSubmodulePathBody {
    pub path: Option<String>,
}

pub async fn git_submodule_init(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitSubmodulePathBody>,
) -> Response {
    let Some(path) = body
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

    if let Err(resp) = run_locked_git_checked(
        &q,
        &["submodule", "init", "--", path],
        Some("git_submodule_init_failed"),
    )
    .await
    {
        return resp;
    }

    git_success_response()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Body of a submodule update request.
pub struct GitSubmoduleUpdateBody {
    pub path: Option<String>,
    #[serde(default)]
    pub init: bool,
    #[serde(default)]
    pub recursive: bool,
}

pub async fn git_submodule_update(
    Query(q): Query<DirectoryQuery>,
    Json(body): Json<GitSubmoduleUpdateBody>,
) -> Response {
    let mut args: Vec<&str> = vec!["submodule", "update"];
    if body.init {
        args.push("--init");
    }
    if body.recursive {
        args.push("--recursive");
    }
    if let Some(path) = body
        .path
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        args.push("--");
        args.push(path);
    }

    if let Err(resp) =
        run_locked_git_env_checked(&q, &args, &[], Some("git_submodule_update_failed")).await
    {
        return resp;
    }

    git_success_response()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::list_submodules;

    #[test]
    fn libgit2_lists_submodules_with_git_config_quoting() {
        let temp = tempfile::tempdir().expect("temp directory");
        git2::Repository::init(temp.path()).expect("repository");
        fs::write(
            temp.path().join(".gitmodules"),
            concat!(
                "[submodule \"quoted name\"]\n",
                "\tpath = vendor/quoted path\n",
                "\turl = ssh://git@example.com/team/repository.git\n",
                "\tbranch = release/v2\n",
            ),
        )
        .expect("gitmodules");

        let listed = list_submodules(temp.path()).expect("submodule list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, "vendor/quoted path");
        assert_eq!(listed[0].url, "ssh://git@example.com/team/repository.git");
        assert_eq!(listed[0].branch.as_deref(), Some("release/v2"));
    }
}
