//! Isolated checkouts owned by a session, independent of the package format.
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use agena_domain::AccessKind;
use agena_plugin_host::sdk::{WorktreeHookInput, WorktreeHookOutput, WorktreeOperation};
use agena_storage_layout::{StorageLayout, secure_directory};

use super::*;

mod includes;

static WORKTREE_IO: agena_async::BlockingPool = agena_async::BlockingPool::new(4);

impl SessionManager {
    pub(super) async fn provision_worktree(
        &self,
        child: SessionId,
        state: &ExecutionState,
    ) -> Result<(), AppError> {
        let context = self.execution_context_with_state(child, state).await?;
        // The channel authority determines its workspace independently. Do
        // not rebase one of those grants onto another directory.
        let source = context
            .effective_workspace_root
            .as_ref()
            .filter(|_| context.channel_authority.is_none())
            .cloned()
            .ok_or_else(|| {
                AppError::Config("worktree isolation requires a project session".into())
            })?;
        let source = worktree_io(move || canonical_directory(&source)).await?;
        let executor = state.executor.for_session_resource_context(&context);
        self.authorize_session_scoped_checks(
            child.get(),
            &executor,
            vec![executor.path_permission_check(AccessKind::Read, &source.to_string_lossy())],
        )
        .await?;

        let name = format!("agena-{}", uuid::Uuid::new_v4().simple());
        let reply = state
            .executor
            .plugin_manager()
            .dispatch_worktree_hook(
                WorktreeHookInput {
                    session_id: child.get(),
                    cwd: source.clone(),
                    operation: WorktreeOperation::Create { name: name.clone() },
                },
                |plugin| executor.plugin_is_enabled(plugin),
            )
            .await
            .map_err(worktree_error)?;
        let worktree = match reply {
            Some(WorktreeHookOutput::Created { path }) => {
                let hook_source = source.clone();
                let path = worktree_io(move || resolve_hook_checkout(&hook_source, &path)).await?;
                let raw = path.to_string_lossy();
                self.authorize_session_scoped_checks(
                    child.get(),
                    &executor,
                    vec![
                        executor.path_permission_check(AccessKind::Read, &raw),
                        executor.path_permission_check(AccessKind::Write, &raw),
                    ],
                )
                .await?;
                SessionWorktree {
                    source,
                    path,
                    provider: SessionWorktreeProvider::Hook,
                }
            }
            None => {
                let discovery = source.clone();
                let repository = worktree_io(move || discover_git_root(&discovery)).await?;
                if repository != source {
                    self.authorize_session_scoped_checks(
                        child.get(),
                        &executor,
                        vec![executor.path_permission_check(
                            AccessKind::Read,
                            &repository.to_string_lossy(),
                        )],
                    )
                    .await?;
                }
                let included = self
                    .select_worktree_files(child, &executor, &repository)
                    .await?;
                worktree_io(move || create_git_worktree(source, repository, name, included)).await?
            }
            Some(WorktreeHookOutput::Removed) => unreachable!("validated hook operation"),
        };

        let command = RequestId::new();
        let change = SessionConfigurationChange {
            effective_workspace_root: Some(Some(worktree.path.clone())),
            worktree: Some(SessionWorktreeChange::Attach(worktree.clone())),
            ..Default::default()
        };
        if let Err(error) = persist(|| {
            self.kernel()
                .sessions
                .configure(command, child, change.clone())
        })
        .await
        {
            // A hook can return a directory already owned by another session.
            // Only a native checkout with our own new Git ref can be cleaned
            // here. Hook-owned checkouts are removed once ownership commits.
            let path = worktree.path.clone();
            if matches!(worktree.provider, SessionWorktreeProvider::Git { .. })
                && let Err(cleanup) = worktree_io(move || remove_git_worktree(&worktree)).await
            {
                tracing::warn!(%cleanup, "could not remove an unattached worktree");
            }
            return Err(AppError::Config(format!(
                "Could not attach worktree {} to session {}: {}",
                path.display(),
                child.get(),
                kernel_error(error),
            )));
        }
        Ok(())
    }

    async fn select_worktree_files(
        &self,
        session: SessionId,
        executor: &ToolExecutor,
        repository: &Path,
    ) -> Result<Vec<PathBuf>, AppError> {
        let manifest = repository.join(includes::MANIFEST_NAME);
        let candidate = manifest.clone();
        if !worktree_io(move || includes::is_regular_file(&candidate)).await? {
            return Ok(Vec::new());
        }
        self.authorize_session_scoped_checks(
            session.get(),
            executor,
            vec![executor.path_permission_check(AccessKind::Read, &manifest.to_string_lossy())],
        )
        .await?;
        let root = repository.to_owned();
        let paths = worktree_io(move || includes::select_files(&root)).await?;
        let checks = paths
            .iter()
            .map(|path| {
                executor.path_permission_check(
                    AccessKind::Read,
                    &repository.join(path).to_string_lossy(),
                )
            })
            .collect();
        self.authorize_session_scoped_checks(session.get(), executor, checks)
            .await?;
        Ok(paths)
    }

    pub(super) async fn release_worktree(&self, session: SessionId) -> Result<(), AppError> {
        let configuration = self
            .kernel()
            .reader
            .configuration(session)
            .await
            .map_err(kernel_error)?;
        let Some(worktree) = configuration.worktree.clone() else {
            return Ok(());
        };
        let path = worktree.path.clone();
        let exists = worktree_io(move || validate_checkout_if_present(&path)).await?;
        if exists && worktree.provider == SessionWorktreeProvider::Hook {
            let state = self.state_for_session(session).await?;
            let context = self.execution_context_with_state(session, &state).await?;
            let executor = state.executor.for_session_resource_context(&context);
            let result = state
                .executor
                .plugin_manager()
                .dispatch_worktree_hook(
                    WorktreeHookInput {
                        session_id: session.get(),
                        cwd: worktree.source.clone(),
                        operation: WorktreeOperation::Remove {
                            path: worktree.path.clone(),
                        },
                    },
                    |plugin| executor.plugin_is_enabled(plugin),
                )
                .await;
            match result {
                Ok(Some(WorktreeHookOutput::Removed)) => {}
                Ok(None) => worktree_io(move || remove_git_worktree(&worktree)).await?,
                // The handler may delete the checkout and report another
                // failure afterward. Keep ownership if the directory remains.
                Err(error) if worktree.path.try_exists().unwrap_or(true) => {
                    return Err(worktree_error(error));
                }
                Err(_) => {}
                Ok(Some(WorktreeHookOutput::Created { .. })) => {
                    unreachable!("validated hook operation")
                }
            }
        } else if matches!(worktree.provider, SessionWorktreeProvider::Git { .. }) {
            worktree_io(move || remove_git_worktree(&worktree)).await?;
        }

        let command = RequestId::new();
        persist(|| {
            self.kernel().sessions.configure(
                command,
                session,
                SessionConfigurationChange {
                    worktree: Some(SessionWorktreeChange::Release),
                    ..Default::default()
                },
            )
        })
        .await
        .map_err(kernel_error)?;
        Ok(())
    }
}

fn worktree_error(error: agena_plugin_host::sdk::PluginError) -> AppError {
    AppError::Config(error.diagnostic_message().to_owned())
}

async fn worktree_io<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, AppError> {
    WORKTREE_IO
        .run(work)
        .await
        .map_err(|_| AppError::Internal("worktree worker stopped".into()))?
        .map_err(AppError::Config)
}

pub(super) async fn validate_existing_worktree(path: PathBuf) -> Result<(), AppError> {
    worktree_io(move || {
        if validate_checkout_if_present(&path)? {
            Ok(())
        } else {
            Err(format!("Session worktree {} is missing", path.display()))
        }
    })
    .await
}

fn canonical_directory(path: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        format!(
            "Cannot resolve worktree directory {}: {error}",
            path.display()
        )
    })?;
    if !canonical.is_dir() {
        return Err(format!(
            "Worktree path {} is not a directory",
            path.display()
        ));
    }
    Ok(canonical)
}

fn validate_checkout_if_present(path: &Path) -> Result<bool, String> {
    // Check from the root down even if the final entry disappeared: a
    // replaced ancestor can redirect a cleanup to an unrelated checkout.
    let mut ancestors = path.ancestors().collect::<Vec<_>>();
    while let Some(ancestor) = ancestors.pop() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(format!(
                    "Worktree directory crosses a symlink or non-directory at {}",
                    ancestor.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "Cannot inspect worktree path {}: {error}",
                    ancestor.display()
                ));
            }
        }
    }
    Ok(true)
}

fn resolve_hook_checkout(source: &Path, output: &Path) -> Result<PathBuf, String> {
    let raw = output
        .to_str()
        .filter(|value| !value.is_empty() && !value.chars().any(char::is_control))
        .ok_or("Invalid worktree path from hook")?;
    let candidate = if output.is_absolute() {
        if raw
            .split(|ch| ch == '/' || (cfg!(windows) && ch == '\\'))
            .any(|part| matches!(part, "." | ".."))
        {
            return Err("An absolute worktree path must omit . and .. components".into());
        }
        output.to_path_buf()
    } else {
        let mut path = source.to_path_buf();
        for component in output.components() {
            match component {
                Component::Normal(name) => path.push(name),
                Component::CurDir => {}
                Component::ParentDir if path.pop() => {}
                _ => return Err("Invalid relative worktree path from hook".into()),
            }
        }
        path
    };
    if !validate_checkout_if_present(&candidate)? {
        return Err(format!(
            "Hook returned missing worktree directory {}",
            candidate.display()
        ));
    }
    let path = canonical_directory(&candidate)?;
    if source.starts_with(&path) {
        return Err("The hook returned the original workspace or one of its ancestors".into());
    }
    Ok(path)
}

fn discover_git_root(source: &Path) -> Result<PathBuf, String> {
    let repo = git2::Repository::discover(source).map_err(|error| {
        format!("Worktree isolation needs a Git repository or a WorktreeCreate hook: {error}")
    })?;
    canonical_directory(
        repo.workdir()
            .ok_or("Worktree isolation needs a Git working directory")?,
    )
}

fn create_git_worktree(
    source: PathBuf,
    repository: PathBuf,
    name: String,
    included: Vec<PathBuf>,
) -> Result<SessionWorktree, String> {
    let relative = source
        .strip_prefix(&repository)
        .map_err(|_| "The Git working directory does not contain the project")?;
    let repo = git2::Repository::open(&repository).map_err(|error| error.to_string())?;
    let root = repo
        .workdir()
        .ok_or("Worktree isolation needs a Git working directory")?;
    if canonical_directory(root)? != repository {
        return Err("The Git working directory changed before worktree creation".into());
    }
    let directory = StorageLayout::current().root().join("worktrees");
    secure_directory(&directory)
        .map_err(|error| format!("Cannot prepare the Agena worktree directory: {error}"))?;
    let directory = canonical_directory(&directory)?;
    let path = directory.join(&name);
    let start = repo
        .find_reference("refs/remotes/origin/HEAD")
        .and_then(|reference| reference.peel_to_commit())
        .or_else(|_| repo.head().and_then(|reference| reference.peel_to_commit()))
        .map_err(|error| format!("Cannot determine a starting commit for the worktree: {error}"))?;
    let branch = repo
        .branch(&name, &start, false)
        .map_err(|error| format!("Cannot reserve a worktree branch: {error}"))?;
    let workspace = if relative.as_os_str().is_empty() {
        path.clone()
    } else {
        path.join(relative)
    };
    let result = repo
        .worktree(
            &name,
            &path,
            Some(git2::WorktreeAddOptions::new().reference(Some(branch.get()))),
        )
        .map_err(|error| error.to_string())
        .and_then(|_| {
            if !validate_checkout_if_present(&workspace)? {
                // The current branch may contain a project directory absent from
                // the default branch. Provide the same cwd in the new checkout.
                fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
            }
            includes::copy_files(&repo, &start, &repository, &path, &included)
        });
    let included_files = match result {
        Ok(files) => files,
        Err(error) => {
            // Clean only the entries reserved for this unique name.
            let cleanup = || -> Result<(), String> {
                if let Ok(tree) = repo.find_worktree(&name) {
                    if tree.path() != path {
                        return Err("Git returned a different worktree path".into());
                    }
                    tree.prune(Some(
                        git2::WorktreePruneOptions::new()
                            .valid(true)
                            .working_tree(true),
                    ))
                    .map_err(|error| error.to_string())?;
                } else if validate_checkout_if_present(&path)? {
                    fs::remove_dir_all(&path).map_err(|error| error.to_string())?;
                }
                repo.find_branch(&name, git2::BranchType::Local)
                    .and_then(|mut branch| branch.delete())
                    .map_err(|error| error.to_string())
            };
            if let Err(cleanup) = cleanup() {
                tracing::warn!(%cleanup, "could not roll back a failed Git worktree checkout");
            }
            return Err(format!(
                "Cannot create Git worktree {}: {error}",
                path.display()
            ));
        }
    };

    let repository_paths = (repository != source).then_some(SessionGitWorktreePaths {
        source_root: repository,
        checkout_root: path,
    });
    Ok(SessionWorktree {
        source,
        path: workspace,
        provider: SessionWorktreeProvider::Git {
            name,
            start_commit: start.id().to_string(),
            repository_paths,
            included_files,
        },
    })
}

fn remove_git_worktree(worktree: &SessionWorktree) -> Result<(), String> {
    let (repository, path) = match &worktree.provider {
        SessionWorktreeProvider::Git {
            repository_paths: Some(paths),
            ..
        } => (&paths.source_root, &paths.checkout_root),
        SessionWorktreeProvider::Git { .. } => (&worktree.source, &worktree.path),
        SessionWorktreeProvider::Hook => (&worktree.path, &worktree.path),
    };
    let present = validate_checkout_if_present(path)?;
    let repo = git2::Repository::open(repository).map_err(|error| {
        format!(
            "Cannot open worktree repository for {}: {error}",
            path.display()
        )
    })?;
    let tree = match &worktree.provider {
        SessionWorktreeProvider::Git { name, .. } => match repo.find_worktree(name) {
            Ok(tree) => tree,
            Err(error) if !present && error.code() == git2::ErrorCode::NotFound => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "Cannot find Git worktree {}: {error}",
                    path.display()
                ));
            }
        },
        SessionWorktreeProvider::Hook => git2::Worktree::open_from_repository(&repo)
            .map_err(|_| format!("Worktree {} needs a WorktreeRemove hook", path.display()))?,
    };
    if tree.path() != path {
        return Err(format!(
            "Git worktree {} moved before removal",
            path.display()
        ));
    }
    if present {
        let checkout = git2::Repository::open(path).map_err(|error| error.to_string())?;
        let included_files = match &worktree.provider {
            SessionWorktreeProvider::Git { included_files, .. } => Some(included_files),
            SessionWorktreeProvider::Hook => None,
        };
        let mut options = git2::StatusOptions::new();
        options
            .include_untracked(true)
            .include_ignored(true)
            .recurse_untracked_dirs(true)
            .recurse_ignored_dirs(included_files.is_some_and(|files| !files.is_empty()))
            .exclude_submodules(false);
        let statuses = checkout
            .statuses(Some(&mut options))
            .map_err(|error| error.to_string())?;
        if statuses.iter().any(|status| {
            !included_files.is_some_and(|files| includes::is_unchanged(path, files, &status))
        }) {
            return Err(format!(
                "Worktree {} contains local files or changes",
                path.display()
            ));
        }
        if let SessionWorktreeProvider::Git { start_commit, .. } = &worktree.provider
            && checkout
                .head()
                .and_then(|head| head.peel_to_commit())
                .map_err(|error| error.to_string())?
                .id()
                .to_string()
                != *start_commit
        {
            return Err(format!(
                "Worktree {} has a different HEAD from its starting commit",
                path.display()
            ));
        }
    }
    tree.prune(Some(
        git2::WorktreePruneOptions::new()
            .valid(present)
            .working_tree(present),
    ))
    .map_err(|error| format!("Cannot remove Git worktree {}: {error}", path.display()))?;
    if let SessionWorktreeProvider::Git {
        name, start_commit, ..
    } = &worktree.provider
        && let Ok(mut branch) = repo.find_branch(name, git2::BranchType::Local)
        && let Some(tip) = branch.get().target()
        && tip.to_string() == *start_commit
    {
        branch
            .delete()
            .map_err(|error| format!("Cannot remove the worktree branch {name}: {error}"))?;
    }
    Ok(())
}
