impl ApiService {
    pub fn snapshot_status(
        &self,
        runtime: &agena::runtime::AgenaRuntime,
    ) -> SnapshotStatusResource {
        let workspace_root = runtime.workspace_root().to_path_buf();
        let capabilities = agena::tool::snapshot_backend_capabilities(&workspace_root);
        let Some(manager) = runtime.session_manager() else {
            return SnapshotStatusResource {
                workspace_root: workspace_root.display().to_string(),
                session_runtime_available: false,
                registry_available: false,
                preferred_backend: capabilities
                    .preferred_backend
                    .map(|backend| backend.to_string()),
                git: snapshot_backend_support_resource(capabilities.git),
                rift: snapshot_backend_support_resource(capabilities.rift),
                active: Vec::new(),
                managed: Vec::new(),
            };
        };
        let executor = manager.tool_executor();
        let Some(registry) = executor.snapshot_registry() else {
            return SnapshotStatusResource {
                workspace_root: workspace_root.display().to_string(),
                session_runtime_available: true,
                registry_available: false,
                preferred_backend: capabilities
                    .preferred_backend
                    .map(|backend| backend.to_string()),
                git: snapshot_backend_support_resource(capabilities.git),
                rift: snapshot_backend_support_resource(capabilities.rift),
                active: Vec::new(),
                managed: Vec::new(),
            };
        };

        let active = agena::tool::snapshot_list_active(registry)
            .into_iter()
            .map(|entry| ActiveSnapshotResource {
                session_id: entry.session_id,
                path: entry.path.display().to_string(),
                branch: entry.branch,
                backend: entry.backend.to_string(),
                created_here: entry.created_here,
            })
            .collect();
        let managed = agena::tool::snapshot_list_managed(&workspace_root, registry)
            .into_iter()
            .map(|entry| ManagedSnapshotResource {
                stale: entry.is_stale(),
                path: entry.path.display().to_string(),
                session_id: entry.session_id,
                branch: entry.branch,
                backend: entry.backend.map(|backend| backend.to_string()),
                registered_with_git: entry.registered_with_git,
                registered_with_rift: entry.registered_with_rift,
            })
            .collect();

        SnapshotStatusResource {
            workspace_root: workspace_root.display().to_string(),
            session_runtime_available: true,
            registry_available: true,
            preferred_backend: capabilities
                .preferred_backend
                .map(|backend| backend.to_string()),
            git: snapshot_backend_support_resource(capabilities.git),
            rift: snapshot_backend_support_resource(capabilities.rift),
            active,
            managed,
        }
    }

    pub async fn git_status(
        &self,
        runtime: &agena::runtime::AgenaRuntime,
    ) -> ApiResult<GitStatusResource> {
        let workspace_root = runtime.workspace_root().to_path_buf();
        let git_available = command_available("git");
        let gh_available = command_available("gh");

        let Some(manager) = runtime.session_manager() else {
            return Ok(GitStatusResource {
                workspace_root: workspace_root.display().to_string(),
                git_available,
                repo: false,
                gh_available,
                branch: None,
                upstream: None,
                ahead: None,
                behind: None,
                staged_files: 0,
                unstaged_files: 0,
                untracked_files: 0,
                changed_files: 0,
                clean: true,
                snapshot_active_sessions: 0,
                snapshot_managed_dirs: 0,
            });
        };

        let executor = manager.tool_executor();
        let (snapshot_active_sessions, snapshot_managed_dirs) = match executor.snapshot_registry() {
            Some(registry) => (
                agena::tool::snapshot_list_active(registry).len() as u64,
                agena::tool::snapshot_list_managed(&workspace_root, registry).len() as u64,
            ),
            None => (0, 0),
        };

        if !git_available {
            return Ok(GitStatusResource {
                workspace_root: workspace_root.display().to_string(),
                git_available,
                repo: false,
                gh_available,
                branch: None,
                upstream: None,
                ahead: None,
                behind: None,
                staged_files: 0,
                unstaged_files: 0,
                untracked_files: 0,
                changed_files: 0,
                clean: true,
                snapshot_active_sessions,
                snapshot_managed_dirs,
            });
        }

        let repo = git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]);
        if !repo {
            return Ok(GitStatusResource {
                workspace_root: workspace_root.display().to_string(),
                git_available,
                repo,
                gh_available,
                branch: None,
                upstream: None,
                ahead: None,
                behind: None,
                staged_files: 0,
                unstaged_files: 0,
                untracked_files: 0,
                changed_files: 0,
                clean: true,
                snapshot_active_sessions,
                snapshot_managed_dirs,
            });
        }

        let branch = git_output(&workspace_root, ["branch", "--show-current"])?;
        let upstream = git_output(
            &workspace_root,
            [
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
        )
        .ok()
        .and_then(|value| non_empty(Some(value.as_str())).map(ToOwned::to_owned));
        let ahead_behind = upstream.as_ref().and_then(|_| {
            git_output(
                &workspace_root,
                ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
            )
            .ok()
        });
        let (ahead, behind) = parse_ahead_behind(ahead_behind.as_deref());
        let status = git_output(&workspace_root, ["status", "--porcelain"])?;
        let (staged_files, unstaged_files, untracked_files, changed_files) =
            summarize_git_status(status.as_str());

        Ok(GitStatusResource {
            workspace_root: workspace_root.display().to_string(),
            git_available,
            repo,
            gh_available,
            branch: non_empty(Some(branch.as_str())).map(ToOwned::to_owned),
            upstream,
            ahead,
            behind,
            staged_files,
            unstaged_files,
            untracked_files,
            changed_files,
            clean: changed_files == 0,
            snapshot_active_sessions,
            snapshot_managed_dirs,
        })
    }

    pub async fn git_init(
        &self,
        runtime: &agena::runtime::AgenaRuntime,
    ) -> ApiResult<GitStatusResource> {
        let workspace_root = runtime.workspace_root().to_path_buf();
        if !command_available("git") {
            return Err(ApiError::bad_request(
                "git is not available on PATH; cannot initialize a repository",
            ));
        }

        if !git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]) {
            let output = Command::new("git")
                .args(["init"])
                .current_dir(&workspace_root)
                .output()
                .map_err(|error| {
                    ApiError::internal(format!("failed to execute git init: {error}"))
                })?;
            if !output.status.success() {
                return Err(ApiError::internal(format!(
                    "git init failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
        }

        self.git_status(runtime).await
    }

    pub async fn vcs_diff_raw(&self, runtime: &agena::runtime::AgenaRuntime) -> ApiResult<String> {
        let workspace_root = runtime.workspace_root().to_path_buf();
        if !command_available("git") {
            return Ok(String::new());
        }
        if !git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]) {
            return Ok(String::new());
        }

        let mut chunks = Vec::<String>::new();
        if git_success(&workspace_root, ["rev-parse", "--verify", "HEAD"]) {
            let tracked = git_output_with_status(
                &workspace_root,
                ["diff", "--no-ext-diff", "--binary", "HEAD", "--"],
                &[0],
            )?;
            if !tracked.trim().is_empty() {
                chunks.push(tracked);
            }
        } else {
            let staged = git_output_with_status(
                &workspace_root,
                ["diff", "--no-ext-diff", "--binary", "--cached", "--"],
                &[0],
            )?;
            if !staged.trim().is_empty() {
                chunks.push(staged);
            }
        }

        let status = git_output(&workspace_root, ["status", "--porcelain"])?;
        for file in untracked_files_from_status(status.as_str()) {
            let patch = git_untracked_patch(&workspace_root, file.as_str())?;
            if !patch.trim().is_empty() {
                chunks.push(patch);
            }
        }

        Ok(chunks.join("\n"))
    }

    pub async fn git_stage(
        &self,
        runtime: &agena::runtime::AgenaRuntime,
        request: GitStageRequest,
    ) -> ApiResult<GitStatusResource> {
        let workspace_root = runtime.workspace_root().to_path_buf();
        let status = self.git_status(runtime).await?;
        if !status.git_available || !status.repo {
            return Err(ApiError::bad_request(
                "the runtime workspace is not a git repository",
            ));
        }

        let mut command = Command::new("git");
        command.arg("add");
        if request.paths.is_empty() {
            command.arg("--all");
        } else {
            command.arg("--");
            for path in request.paths {
                let normalized = validate_git_stage_path(path.as_str())?;
                command.arg(normalized);
            }
        }
        let output = command
            .current_dir(&workspace_root)
            .output()
            .map_err(|error| ApiError::internal(format!("failed to execute git add: {error}")))?;
        if !output.status.success() {
            return Err(ApiError::bad_request(format!(
                "git add failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        self.git_status(runtime).await
    }

    pub async fn git_commit(
        &self,
        runtime: &agena::runtime::AgenaRuntime,
        request: GitCommitRequest,
    ) -> ApiResult<GitCommitResource> {
        let workspace_root = runtime.workspace_root().to_path_buf();
        let status = self.git_status(runtime).await?;
        if !status.git_available || !status.repo {
            return Err(ApiError::bad_request(
                "the runtime workspace is not a git repository",
            ));
        }
        if status.staged_files == 0 {
            return Err(ApiError::bad_request("no staged changes to commit"));
        }
        let message = request.message.trim();
        if message.is_empty() {
            return Err(ApiError::bad_request("commit message is required"));
        }

        let output = Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&workspace_root)
            .output()
            .map_err(|error| {
                ApiError::internal(format!("failed to execute git commit: {error}"))
            })?;
        if !output.status.success() {
            return Err(ApiError::bad_request(format!(
                "git commit failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        Ok(GitCommitResource {
            commit: git_output(&workspace_root, ["rev-parse", "HEAD"])?,
            summary: git_output(&workspace_root, ["log", "-1", "--pretty=%s"])?,
            status: self.git_status(runtime).await?,
        })
    }

    pub async fn git_create_pull_request(
        &self,
        runtime: &agena::runtime::AgenaRuntime,
        request: GitPullRequestCreateRequest,
    ) -> ApiResult<GitPullRequestResource> {
        let workspace_root = runtime.workspace_root().to_path_buf();
        let status = self.git_status(runtime).await?;
        if !status.git_available || !status.repo {
            return Err(ApiError::bad_request(
                "the runtime workspace is not a git repository",
            ));
        }
        if !status.gh_available {
            return Err(ApiError::bad_request("gh is not available on PATH"));
        }
        let title = request.title.trim();
        if title.is_empty() {
            return Err(ApiError::bad_request("pull request title is required"));
        }
        let head = request
            .head
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or(status.branch.as_deref())
            .ok_or_else(|| {
                ApiError::bad_request("could not determine the pull request head branch")
            })?;

        let mut command = Command::new("gh");
        command
            .args(["pr", "create", "--title", title, "--body"])
            .arg(request.body.as_deref().unwrap_or_default())
            .args(["--head", head]);
        if let Some(base) = request
            .base
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            command.args(["--base", base]);
        }
        let output = command
            .current_dir(&workspace_root)
            .output()
            .map_err(|error| {
                ApiError::internal(format!("failed to execute gh pr create: {error}"))
            })?;
        if !output.status.success() {
            return Err(ApiError::bad_request(format!(
                "gh pr create failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if url.is_empty() {
            return Err(ApiError::internal("gh pr create returned an empty URL"));
        }
        Ok(GitPullRequestResource { url })
    }
}

fn snapshot_backend_support_resource(
    support: agena::tool::SnapshotBackendSupport,
) -> SnapshotBackendSupportResource {
    SnapshotBackendSupportResource {
        backend: support.backend.to_string(),
        available: support.available,
        detail: support.detail,
    }
}

fn validate_git_stage_path(path: &str) -> ApiResult<&str> {
    let normalized = path.trim();
    if normalized.is_empty() || Path::new(normalized).is_absolute() {
        return Err(ApiError::bad_request(
            "git stage paths must be non-empty workspace-relative paths",
        ));
    }
    if Path::new(normalized).components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        return Err(ApiError::bad_request(
            "git stage paths cannot contain parent or root components",
        ));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::validate_git_stage_path;

    #[test]
    fn validates_workspace_relative_git_stage_paths() {
        assert_eq!(
            validate_git_stage_path("src/main.rs").unwrap(),
            "src/main.rs"
        );
        assert!(validate_git_stage_path("../outside").is_err());
        assert!(validate_git_stage_path("src/../../outside").is_err());
        assert!(validate_git_stage_path("/absolute").is_err());
        assert!(validate_git_stage_path(" ").is_err());
    }
}

fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn git_success<const N: usize>(workspace_root: &Path, args: [&str; N]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn git_output<const N: usize>(workspace_root: &Path, args: [&str; N]) -> ApiResult<String> {
    Ok(git_output_with_status(workspace_root, args, &[0])?
        .trim()
        .to_string())
}

fn git_output_with_status<const N: usize>(
    workspace_root: &Path,
    args: [&str; N],
    ok_statuses: &[i32],
) -> ApiResult<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .map_err(|error| {
            ApiError::internal(format!("failed to execute git {:?}: {}", args, error))
        })?;
    let code = output.status.code().unwrap_or_default();
    if !ok_statuses.contains(&code) {
        return Err(ApiError::internal(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn untracked_files_from_status(status: &str) -> Vec<String> {
    status
        .lines()
        .filter_map(|line| line.strip_prefix("?? ").map(str::trim))
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn git_untracked_patch(workspace_root: &Path, file: &str) -> ApiResult<String> {
    #[cfg(windows)]
    let null_path = "NUL";
    #[cfg(not(windows))]
    let null_path = "/dev/null";

    git_output_with_status(
        workspace_root,
        [
            "diff",
            "--no-index",
            "--binary",
            "--no-ext-diff",
            "--",
            null_path,
            file,
        ],
        &[0, 1],
    )
}

fn parse_ahead_behind(value: Option<&str>) -> (Option<u64>, Option<u64>) {
    let Some(value) = value else {
        return (None, None);
    };
    let mut parts = value.split_whitespace();
    let behind = parts.next().and_then(|part| part.parse::<u64>().ok());
    let ahead = parts.next().and_then(|part| part.parse::<u64>().ok());
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
use super::{
    ActiveSnapshotResource, ApiError, ApiResult, ApiService, Command, GitCommitRequest,
    GitCommitResource, GitPullRequestCreateRequest, GitPullRequestResource, GitStageRequest,
    GitStatusResource, ManagedSnapshotResource, Path, SnapshotBackendSupportResource,
    SnapshotStatusResource, non_empty,
};
