use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use path_clean::PathClean;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::CheckpointError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitSnapshotCheckpoint {
    pub repo_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_prefix: Option<String>,
    pub commit_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preexisting_untracked: Vec<String>,
}

pub(super) struct GitSnapshotBackend;

impl GitSnapshotBackend {
    pub fn capture(
        workspace_root: &Path,
    ) -> Result<Option<GitSnapshotCheckpoint>, CheckpointError> {
        let Some(repo_root) = resolve_repo_root(workspace_root)? else {
            return Ok(None);
        };
        let scope_prefix = scope_prefix(repo_root.as_path(), workspace_root)?;
        let preexisting_untracked = list_untracked(repo_root.as_path(), scope_prefix.as_deref())?;
        let temp_index_path = std::env::temp_dir()
            .join(format!("agena-git-index-{}", Uuid::new_v4()))
            .clean();
        let base_env = [
            ("GIT_INDEX_FILE", temp_index_path.as_os_str()),
            ("GIT_AUTHOR_NAME", OsStr::new("agena")),
            ("GIT_AUTHOR_EMAIL", OsStr::new("agena@example.invalid")),
            ("GIT_COMMITTER_NAME", OsStr::new("agena")),
            ("GIT_COMMITTER_EMAIL", OsStr::new("agena@example.invalid")),
        ];

        if let Some(head) = verify_head(repo_root.as_path())? {
            run_git_status(
                repo_root.as_path(),
                &[OsString::from("read-tree"), OsString::from(head)],
                &base_env,
            )?;
        }

        let scope_arg = scope_prefix.clone().unwrap_or_else(|| ".".to_string());
        run_git_status(
            repo_root.as_path(),
            &[
                OsString::from("add"),
                OsString::from("--all"),
                OsString::from("--"),
                OsString::from(scope_arg.clone()),
            ],
            &base_env,
        )?;

        let tree_id = run_git_stdout(
            repo_root.as_path(),
            &[OsString::from("write-tree")],
            &base_env,
        )?;
        let mut commit_args = vec![OsString::from("commit-tree"), OsString::from(tree_id)];
        if let Some(head) = verify_head(repo_root.as_path())? {
            commit_args.push(OsString::from("-p"));
            commit_args.push(OsString::from(head));
        }
        commit_args.push(OsString::from("-m"));
        commit_args.push(OsString::from("agena snapshot"));
        let commit_id = run_git_stdout(repo_root.as_path(), commit_args.as_slice(), &base_env)?;
        let _ = fs::remove_file(temp_index_path);

        Ok(Some(GitSnapshotCheckpoint {
            repo_root: normalize_path_text(repo_root.as_path()),
            scope_prefix,
            commit_id,
            preexisting_untracked,
        }))
    }

    pub fn restore(snapshot: &GitSnapshotCheckpoint) -> Result<(), CheckpointError> {
        let repo_root = PathBuf::from(snapshot.repo_root.as_str());
        let scope_arg = snapshot
            .scope_prefix
            .clone()
            .unwrap_or_else(|| ".".to_string());
        run_git_status(
            repo_root.as_path(),
            &[
                OsString::from("restore"),
                OsString::from("--source"),
                OsString::from(snapshot.commit_id.clone()),
                OsString::from("--worktree"),
                OsString::from("--"),
                OsString::from(scope_arg),
            ],
            &[],
        )?;

        let current_untracked =
            list_untracked(repo_root.as_path(), snapshot.scope_prefix.as_deref())?;
        let preexisting = snapshot
            .preexisting_untracked
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        for path in current_untracked {
            if preexisting.contains(path.as_str()) {
                continue;
            }

            let absolute = repo_root.join(path.as_str()).clean();
            match fs::metadata(absolute.as_path()) {
                Ok(metadata) if metadata.is_file() => {
                    fs::remove_file(absolute.as_path())?;
                }
                Ok(metadata) if metadata.is_dir() => {
                    fs::remove_dir_all(absolute.as_path())?;
                }
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(CheckpointError::Io(err)),
            }
        }

        Ok(())
    }
}

fn resolve_repo_root(path: &Path) -> Result<Option<PathBuf>, CheckpointError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(stdout).clean()))
}

fn verify_head(repo_root: &Path) -> Result<Option<String>, CheckpointError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("rev-parse")
        .arg("--verify")
        .arg("HEAD")
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

fn list_untracked(
    repo_root: &Path,
    scope_prefix: Option<&str>,
) -> Result<Vec<String>, CheckpointError> {
    let mut args = vec![
        OsString::from("ls-files"),
        OsString::from("--others"),
        OsString::from("--exclude-standard"),
    ];
    if let Some(scope_prefix) = scope_prefix {
        args.push(OsString::from("--"));
        args.push(OsString::from(scope_prefix));
    }
    let stdout = run_git_stdout(repo_root, args.as_slice(), &[])?;
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn scope_prefix(
    repo_root: &Path,
    workspace_root: &Path,
) -> Result<Option<String>, CheckpointError> {
    let cleaned_workspace = workspace_root.clean();
    if cleaned_workspace == repo_root {
        return Ok(None);
    }
    let relative = cleaned_workspace.strip_prefix(repo_root).map_err(|_| {
        CheckpointError::Git(format!(
            "workspace {} is not inside repo {}",
            cleaned_workspace.display(),
            repo_root.display()
        ))
    })?;
    let text = normalize_path_text(relative);
    if text.is_empty() || text == "." {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

fn run_git_stdout(
    repo_root: &Path,
    args: &[OsString],
    envs: &[(&str, &OsStr)],
) -> Result<String, CheckpointError> {
    let output = run_git(repo_root, args, envs)?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_git_status(
    repo_root: &Path,
    args: &[OsString],
    envs: &[(&str, &OsStr)],
) -> Result<(), CheckpointError> {
    let _ = run_git(repo_root, args, envs)?;
    Ok(())
}

fn run_git(
    repo_root: &Path,
    args: &[OsString],
    envs: &[(&str, &OsStr)],
) -> Result<std::process::Output, CheckpointError> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo_root);
    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(CheckpointError::Git(format!(
            "{}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output)
}

fn normalize_path_text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
