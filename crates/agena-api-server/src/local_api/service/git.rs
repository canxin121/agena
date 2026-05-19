use super::*;

impl ApiService {
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
                worktree_active_sessions: 0,
                worktree_managed_dirs: 0,
            });
        };

        let executor = manager.tool_executor();
        let (worktree_active_sessions, worktree_managed_dirs) = match executor.worktree_registry() {
            Some(registry) => (
                agena::tool::worktree_list_active(registry).len() as u64,
                agena::tool::worktree_list_managed(&workspace_root, registry).len() as u64,
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
                worktree_active_sessions,
                worktree_managed_dirs,
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
                worktree_active_sessions,
                worktree_managed_dirs,
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
            worktree_active_sessions,
            worktree_managed_dirs,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ahead_behind_interprets_git_rev_list_counts() {
        assert_eq!(parse_ahead_behind(Some("0\t0")), (Some(0), Some(0)));
        assert_eq!(parse_ahead_behind(Some("2 5")), (Some(5), Some(2)));
        assert_eq!(parse_ahead_behind(None), (None, None));
    }

    #[test]
    fn summarize_git_status_counts_porcelain_entries() {
        let status = "M  staged.txt\n M unstaged.txt\nMM both.txt\n?? new.txt\n";
        assert_eq!(summarize_git_status(status), (2, 2, 1, 4));
    }
}
