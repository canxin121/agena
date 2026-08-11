use std::{path::PathBuf, process::Stdio, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

const COMMAND_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const MUTATING_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_COMMAND_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

async fn snapshot_counts(
    control: Option<Arc<dyn agena_runtime::SessionExecutionControl>>,
    workspace_root: PathBuf,
) -> ApplicationResult<(u64, u64)> {
    let Some(control) = control else {
        return Ok((0, 0));
    };
    let permit = SNAPSHOT_WORKERS
        .acquire()
        .await
        .map_err(|_| ApplicationError::internal("snapshot worker pool is unavailable"))?;
    let status = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        control.snapshot_status(&workspace_root)
    })
    .await
    .map_err(|error| {
        ApplicationError::internal(format!("snapshot status worker failed: {error}"))
    })?;
    Ok(status
        .map(|status| (status.active.len() as u64, status.managed.len() as u64))
        .unwrap_or((0, 0)))
}

impl ApplicationService {
    pub fn snapshot_status(
        &self,
        control: Option<&dyn agena_runtime::SessionExecutionControl>,
        capabilities: agena_tool::SnapshotBackendCapabilities,
    ) -> SnapshotStatusResource {
        let workspace_root = PathBuf::from(&self.workspace_root);
        let Some(control) = control else {
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
        let Some(snapshot_status) = control.snapshot_status(&workspace_root) else {
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

        let active = snapshot_status
            .active
            .into_iter()
            .map(|entry| ActiveSnapshotResource {
                session_id: entry.session_id,
                path: entry.path,
                branch: entry.branch,
                backend: entry.backend.to_string(),
                created_here: entry.created_here,
            })
            .collect();
        let managed = snapshot_status
            .managed
            .into_iter()
            .map(|entry| ManagedSnapshotResource {
                stale: entry.stale,
                path: entry.path,
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
        control: Option<Arc<dyn agena_runtime::SessionExecutionControl>>,
    ) -> ApplicationResult<GitStatusResource> {
        let workspace_root = PathBuf::from(&self.workspace_root);
        let (git_available, gh_available) =
            tokio::join!(command_available("git"), command_available("gh"));

        let (snapshot_active_sessions, snapshot_managed_dirs) =
            snapshot_counts(control, workspace_root.clone()).await?;

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

        let repo = git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]).await;
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

        let branch = git_output(&workspace_root, ["branch", "--show-current"]).await?;
        let upstream = git_output(
            &workspace_root,
            [
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
        )
        .await
        .ok()
        .and_then(|value| non_empty(Some(value.as_str())).map(ToOwned::to_owned));
        let ahead_behind = if upstream.is_some() {
            git_output(
                &workspace_root,
                ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
            )
            .await
            .ok()
        } else {
            None
        };
        let (ahead, behind) = parse_ahead_behind(ahead_behind.as_deref());
        let status = git_output(&workspace_root, ["status", "--porcelain"]).await?;
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
        control: Option<Arc<dyn agena_runtime::SessionExecutionControl>>,
    ) -> ApplicationResult<GitStatusResource> {
        let workspace_root = PathBuf::from(&self.workspace_root);
        if !command_available("git").await {
            return Err(ApplicationError::bad_request(
                "git is not available on PATH; cannot initialize a repository",
            ));
        }

        if !git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]).await {
            let mut command = Command::new("git");
            command.args(["init"]).current_dir(&workspace_root);
            let output = run_command_output(command, MUTATING_COMMAND_TIMEOUT, "git init").await?;
            if !output.status.success() {
                return Err(ApplicationError::internal(format!(
                    "git init failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
        }

        self.git_status(control).await
    }

    pub async fn vcs_diff_raw(&self) -> ApplicationResult<String> {
        let workspace_root = PathBuf::from(&self.workspace_root);
        if !command_available("git").await {
            return Ok(String::new());
        }
        if !git_success(&workspace_root, ["rev-parse", "--is-inside-work-tree"]).await {
            return Ok(String::new());
        }

        let mut chunks = Vec::<String>::new();
        if git_success(&workspace_root, ["rev-parse", "--verify", "HEAD"]).await {
            let tracked = git_output_with_status(
                &workspace_root,
                ["diff", "--no-ext-diff", "--binary", "HEAD", "--"],
                &[0],
            )
            .await?;
            if !tracked.trim().is_empty() {
                chunks.push(tracked);
            }
        } else {
            let staged = git_output_with_status(
                &workspace_root,
                ["diff", "--no-ext-diff", "--binary", "--cached", "--"],
                &[0],
            )
            .await?;
            if !staged.trim().is_empty() {
                chunks.push(staged);
            }
        }

        let status = git_output(&workspace_root, ["status", "--porcelain"]).await?;
        for file in untracked_files_from_status(status.as_str()) {
            let patch = git_untracked_patch(&workspace_root, file.as_str()).await?;
            if !patch.trim().is_empty() {
                chunks.push(patch);
            }
        }

        Ok(chunks.join("\n"))
    }

    pub async fn git_stage(
        &self,
        control: Option<Arc<dyn agena_runtime::SessionExecutionControl>>,
        request: GitStageRequest,
    ) -> ApplicationResult<GitStatusResource> {
        let workspace_root = PathBuf::from(&self.workspace_root);
        let status = self.git_status(control.clone()).await?;
        if !status.git_available || !status.repo {
            return Err(ApplicationError::bad_request(
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
        command.current_dir(&workspace_root);
        let output = run_command_output(command, MUTATING_COMMAND_TIMEOUT, "git add").await?;
        if !output.status.success() {
            return Err(ApplicationError::bad_request_with_diagnostic(
                "Git could not stage the selected files.",
                format!(
                    "git add failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
        self.git_status(control).await
    }

    pub async fn git_commit(
        &self,
        control: Option<Arc<dyn agena_runtime::SessionExecutionControl>>,
        request: GitCommitRequest,
    ) -> ApplicationResult<GitCommitResource> {
        let workspace_root = PathBuf::from(&self.workspace_root);
        let status = self.git_status(control.clone()).await?;
        if !status.git_available || !status.repo {
            return Err(ApplicationError::bad_request(
                "the runtime workspace is not a git repository",
            ));
        }
        if status.staged_files == 0 {
            return Err(ApplicationError::bad_request("no staged changes to commit"));
        }
        let message = request.message.trim();
        if message.is_empty() {
            return Err(ApplicationError::bad_request("commit message is required"));
        }

        let mut command = Command::new("git");
        command
            .args(["commit", "-m", message])
            .current_dir(&workspace_root);
        let output = run_command_output(command, MUTATING_COMMAND_TIMEOUT, "git commit").await?;
        if !output.status.success() {
            return Err(ApplicationError::bad_request_with_diagnostic(
                "Git could not create the commit.",
                format!(
                    "git commit failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }

        Ok(GitCommitResource {
            commit: git_output(&workspace_root, ["rev-parse", "HEAD"]).await?,
            summary: git_output(&workspace_root, ["log", "-1", "--pretty=%s"]).await?,
            status: self.git_status(control).await?,
        })
    }

    pub async fn git_create_pull_request(
        &self,
        control: Option<Arc<dyn agena_runtime::SessionExecutionControl>>,
        request: GitPullRequestCreateRequest,
    ) -> ApplicationResult<GitPullRequestResource> {
        let workspace_root = PathBuf::from(&self.workspace_root);
        let status = self.git_status(control).await?;
        if !status.git_available || !status.repo {
            return Err(ApplicationError::bad_request(
                "the runtime workspace is not a git repository",
            ));
        }
        if !status.gh_available {
            return Err(ApplicationError::bad_request("gh is not available on PATH"));
        }
        let title = request.title.trim();
        if title.is_empty() {
            return Err(ApplicationError::bad_request(
                "pull request title is required",
            ));
        }
        let head = request
            .head
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or(status.branch.as_deref())
            .ok_or_else(|| {
                ApplicationError::bad_request("could not determine the pull request head branch")
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
        command.current_dir(&workspace_root);
        let output = run_command_output(command, MUTATING_COMMAND_TIMEOUT, "gh pr create").await?;
        if !output.status.success() {
            return Err(ApplicationError::bad_request_with_diagnostic(
                "GitHub could not create the pull request.",
                format!(
                    "gh pr create failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if url.is_empty() {
            return Err(ApplicationError::internal(
                "gh pr create returned an empty URL",
            ));
        }
        Ok(GitPullRequestResource { url })
    }
}

fn snapshot_backend_support_resource(
    support: agena_tool::SnapshotBackendSupport,
) -> SnapshotBackendSupportResource {
    SnapshotBackendSupportResource {
        backend: support.backend.to_string(),
        available: support.available,
        detail: support.detail,
    }
}

fn validate_git_stage_path(path: &str) -> ApplicationResult<&str> {
    let normalized = path.trim();
    if normalized.is_empty() || Path::new(normalized).is_absolute() {
        return Err(ApplicationError::bad_request(
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
        return Err(ApplicationError::bad_request(
            "git stage paths cannot contain parent or root components",
        ));
    }
    Ok(normalized)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{Duration, run_command_output_with_limit, validate_git_stage_path};
    use tokio::process::Command;

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

    #[cfg(unix)]
    #[tokio::test]
    async fn subprocess_timeout_kills_a_silent_command_instead_of_blocking_the_runtime() {
        let mut command = Command::new("sh");
        command.args(["-c", "exec sleep 30"]);
        let error = run_command_output_with_limit(
            command,
            Duration::from_millis(20),
            "silent test command",
            1024,
        )
        .await
        .expect_err("silent command must time out");
        assert!(error.to_string().contains("timed out"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn subprocess_output_is_bounded() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf 12345"]);
        let error =
            run_command_output_with_limit(command, Duration::from_secs(1), "noisy test command", 4)
                .await
                .expect_err("oversized output must fail");
        assert!(error.to_string().contains("output limit"));
    }
}

async fn run_command_output(
    command: Command,
    timeout: Duration,
    description: &str,
) -> ApplicationResult<std::process::Output> {
    run_command_output_with_limit(command, timeout, description, MAX_COMMAND_OUTPUT_BYTES).await
}

async fn read_bounded_output<R>(
    mut reader: R,
    output_limit: usize,
) -> std::io::Result<(Vec<u8>, bool)>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut exceeded = false;
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        let remaining = output_limit.saturating_sub(output.len());
        let captured = remaining.min(read);
        output.extend_from_slice(&chunk[..captured]);
        exceeded |= captured < read;
    }
    Ok((output, exceeded))
}

async fn run_command_output_with_limit(
    mut command: Command,
    timeout: Duration,
    description: &str,
    output_limit: usize,
) -> ApplicationResult<std::process::Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PROMPT_DISABLED", "1");
    let mut child = agena_process::spawn(command).map_err(|error| {
        ApplicationError::internal(format!("failed to execute {description}: {error}"))
    })?;
    let stdout = child.stdout().take().ok_or_else(|| {
        ApplicationError::internal(format!("failed to capture {description} stdout"))
    })?;
    let stderr = child.stderr().take().ok_or_else(|| {
        ApplicationError::internal(format!("failed to capture {description} stderr"))
    })?;
    tokio::time::timeout(timeout, async move {
        let (stdout_result, stderr_result, status_result) = tokio::join!(
            read_bounded_output(stdout, output_limit),
            read_bounded_output(stderr, output_limit),
            child.wait(),
        );
        let (stdout_bytes, stdout_exceeded) = stdout_result.map_err(|error| {
            ApplicationError::internal(format!("failed to read {description} stdout: {error}"))
        })?;
        let (stderr_bytes, stderr_exceeded) = stderr_result.map_err(|error| {
            ApplicationError::internal(format!("failed to read {description} stderr: {error}"))
        })?;
        let status = status_result.map_err(|error| {
            ApplicationError::internal(format!("failed to wait for {description}: {error}"))
        })?;
        if stdout_exceeded || stderr_exceeded {
            return Err(ApplicationError::internal(format!(
                "{description} exceeded the {} byte output limit",
                output_limit
            )));
        }
        Ok(std::process::Output {
            status,
            stdout: stdout_bytes,
            stderr: stderr_bytes,
        })
    })
    .await
    .map_err(|_| {
        ApplicationError::internal(format!(
            "{description} timed out after {} seconds",
            timeout.as_secs_f64()
        ))
    })?
}

async fn command_available(command_name: &str) -> bool {
    let mut command = Command::new(command_name);
    command.arg("--version");
    run_command_output(command, COMMAND_PROBE_TIMEOUT, command_name)
        .await
        .is_ok_and(|output| output.status.success())
}

async fn git_success<const N: usize>(workspace_root: &Path, args: [&str; N]) -> bool {
    let description = format!("git {args:?}");
    let mut command = Command::new("git");
    command.args(args).current_dir(workspace_root);
    run_command_output(command, GIT_COMMAND_TIMEOUT, description.as_str())
        .await
        .is_ok_and(|output| output.status.success())
}

async fn git_output<const N: usize>(
    workspace_root: &Path,
    args: [&str; N],
) -> ApplicationResult<String> {
    Ok(git_output_with_status(workspace_root, args, &[0])
        .await?
        .trim()
        .to_string())
}

async fn git_output_with_status<const N: usize>(
    workspace_root: &Path,
    args: [&str; N],
    ok_statuses: &[i32],
) -> ApplicationResult<String> {
    let description = format!("git {args:?}");
    let mut command = Command::new("git");
    command.args(args).current_dir(workspace_root);
    let output = run_command_output(command, GIT_COMMAND_TIMEOUT, description.as_str()).await?;
    let code = output.status.code().unwrap_or_default();
    if !ok_statuses.contains(&code) {
        return Err(ApplicationError::internal(format!(
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

async fn git_untracked_patch(workspace_root: &Path, file: &str) -> ApplicationResult<String> {
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
    .await
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
    ActiveSnapshotResource, ApplicationError, ApplicationResult, ApplicationService, Arc,
    GitCommitRequest, GitCommitResource, GitPullRequestCreateRequest, GitPullRequestResource,
    GitStageRequest, GitStatusResource, ManagedSnapshotResource, Path, SNAPSHOT_WORKERS,
    SnapshotBackendSupportResource, SnapshotStatusResource, non_empty,
};
