use std::path::{Path, PathBuf};

use agena_domain::{
    AccessKind, RegisterSessionWorkingDirectory, RemoveSessionWorkingDirectory,
    SessionWorkingDirectory, SessionWorkingDirectoryAccess, SessionWorkingDirectoryRegistration,
    SessionWorkingDirectoryRemoval,
};

use super::*;

impl SessionManager {
    pub(super) async fn add_session_working_directory(
        &self,
        id: i64,
        request: RegisterSessionWorkingDirectory,
    ) -> Result<SessionWorkingDirectoryRegistration, AppError> {
        let session = session_id(id)?;
        // The commit and its observer belong to the runtime even if the HTTP
        // client disconnects after the writer has accepted the command.
        let manager = self.clone();
        self.control_tasks
            .spawn(async move {
                manager
                    .register_directory(session, &request.path, request.access)
                    .await
            })
            .await
            .map_err(|error| {
                AppError::Internal(format!("directory registration task failed: {error}"))
            })?
    }

    pub(super) async fn remove_session_working_directory(
        &self,
        id: i64,
        request: RemoveSessionWorkingDirectory,
    ) -> Result<SessionWorkingDirectoryRemoval, AppError> {
        let session = session_id(id)?;
        let manager = self.clone();
        self.control_tasks
            .spawn(async move { manager.unregister_directory(session, &request.path).await })
            .await
            .map_err(|error| {
                AppError::Internal(format!("directory removal task failed: {error}"))
            })?
    }

    async fn register_directory(
        &self,
        session: SessionId,
        path: &Path,
        access: SessionWorkingDirectoryAccess,
    ) -> Result<SessionWorkingDirectoryRegistration, AppError> {
        if path.as_os_str().is_empty() {
            return Err(AppError::Config(
                "A working directory path is required.".into(),
            ));
        }
        let mut requested_scope = None;
        let mut retries = 3;
        loop {
            if self.shutdown.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            let metadata = read_with_busy_retry(session, "directory_registration", || {
                self.kernel().reader.metadata(session)
            })
            .await?;
            let state = self.state.load_full();
            let mut context = self.execution_context_with_state(session, &state).await?;
            if metadata.workspace_id.is_none()
                || context.global
                || context.owner.is_some()
                || context.channel_authority.is_some()
                || context.relation_kind.is_subagent()
                || context
                    .conversation
                    .as_ref()
                    .is_some_and(|view| view.is_read_only())
            {
                return Err(AppError::Config(
                    "Additional working directories require a writable operator-owned project session.".into(),
                ));
            }

            let root = context.effective_workspace_root.as_deref().ok_or_else(|| {
                AppError::Config("The session has no project working directory.".into())
            })?;
            let root = canonical_directory(root).await?;
            let directory = canonical_directory(&root.join(path)).await?;
            let scope = (metadata.workspace_id, root.clone(), directory.clone());
            if requested_scope.get_or_insert_with(|| scope.clone()) != &scope {
                return Err(AppError::PersistenceConflict(
                    "The working directory or owning workspace changed during registration.".into(),
                ));
            }

            let existing = context
                .additional_working_directories
                .iter()
                .find(|entry| entry.path == directory)
                .cloned();
            let covered_same = directory == root
                || (access == SessionWorkingDirectoryAccess::Inherit && directory.starts_with(&root))
                || existing.as_ref().is_some_and(|entry| entry.access == access)
                || (existing.is_none()
                    && context
                        .additional_working_directories
                        .iter()
                        .filter(|entry| directory.starts_with(&entry.path))
                        .max_by_key(|entry| entry.path.components().count())
                        .is_some_and(|parent| parent.access == access));
            if !covered_same {
                if let Some(entry) = context
                    .additional_working_directories
                    .iter_mut()
                    .find(|entry| entry.path == directory)
                {
                    entry.access = access;
                } else {
                    context
                        .additional_working_directories
                        .push(SessionWorkingDirectory::new(directory.clone(), access));
                }
                let executor = state.executor.for_session_resource_context(&context);
                let target = directory.to_string_lossy();
                let mut checks = vec![executor.path_permission_check(AccessKind::Read, &target)];
                if access == SessionWorkingDirectoryAccess::ReadWrite {
                    checks.push(executor.path_permission_check(AccessKind::Write, &target));
                }
                self.authorize_session_scoped_checks(session.get(), &executor, checks)
                    .await?;
            }

            match self
                .kernel()
                .sessions
                .register_working_directory(
                    RequestId::new(),
                    session,
                    metadata.metadata_version,
                    root,
                    directory,
                    access,
                )
                .await
            {
                Ok(receipt) => {
                    if receipt.value.added {
                        self.dispatch_imported_directory_added(
                            session.get(),
                            &receipt.value.directory,
                        );
                    } else if receipt.value.updated {
                        self.refresh_working_directory_state(session.get());
                    }
                    return Ok(receipt.value);
                }
                Err(KernelError::Conflict(_)) if retries > 0 => retries -= 1,
                Err(KernelError::Conflict(reason)) => {
                    return Err(AppError::PersistenceConflict(reason));
                }
                Err(KernelError::Invalid(reason)) => return Err(AppError::Config(reason)),
                Err(error) => return Err(kernel_error(error)),
            }
        }
    }

    async fn unregister_directory(
        &self,
        session: SessionId,
        path: &Path,
    ) -> Result<SessionWorkingDirectoryRemoval, AppError> {
        if path.as_os_str().is_empty() {
            return Err(AppError::Config(
                "A working directory path is required.".into(),
            ));
        }
        let mut retries = 3;
        loop {
            if self.shutdown.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            let metadata = read_with_busy_retry(session, "directory_removal", || {
                self.kernel().reader.metadata(session)
            })
            .await?;
            let state = self.state.load_full();
            let context = self.execution_context_with_state(session, &state).await?;
            if metadata.workspace_id.is_none()
                || context.global
                || context.owner.is_some()
                || context.channel_authority.is_some()
                || context.relation_kind.is_subagent()
                || context
                    .conversation
                    .as_ref()
                    .is_some_and(|view| view.is_read_only())
            {
                return Err(AppError::Config(
                    "Additional working directories require a writable operator-owned project session.".into(),
                ));
            }

            let root = context.effective_workspace_root.as_deref().ok_or_else(|| {
                AppError::Config("The session has no project working directory.".into())
            })?;
            let root = tokio::fs::canonicalize(root)
                .await
                .unwrap_or_else(|_| root.to_path_buf());
            let candidate = root.join(path);
            let directory = match tokio::fs::canonicalize(&candidate).await {
                Ok(resolved) => resolved,
                Err(_) => {
                    let normalized = normalize_lexical_path(&candidate);
                    context
                        .additional_working_directories
                        .iter()
                        .find(|entry| entry.path == path || entry.path == normalized)
                        .map(|entry| entry.path.clone())
                        .unwrap_or(normalized)
                }
            };

            match self
                .kernel()
                .sessions
                .remove_working_directory(
                    RequestId::new(),
                    session,
                    metadata.metadata_version,
                    directory,
                )
                .await
            {
                Ok(receipt) => {
                    if receipt.value.removed {
                        self.refresh_working_directory_state(session.get());
                    }
                    return Ok(receipt.value);
                }
                Err(KernelError::Conflict(_)) if retries > 0 => retries -= 1,
                Err(KernelError::Conflict(reason)) => {
                    return Err(AppError::PersistenceConflict(reason));
                }
                Err(KernelError::Invalid(reason)) => return Err(AppError::Config(reason)),
                Err(error) => return Err(kernel_error(error)),
            }
        }
    }
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub(super) async fn canonical_directory(path: &Path) -> Result<PathBuf, AppError> {
    let resolved = tokio::fs::canonicalize(path).await.map_err(|error| {
        AppError::Config(format!(
            "Cannot resolve working directory {}: {error}",
            path.display()
        ))
    })?;
    if resolved.to_str().is_none()
        || !tokio::fs::metadata(&resolved)
            .await
            .is_ok_and(|entry| entry.is_dir())
    {
        return Err(AppError::Config(format!(
            "{} must be an existing directory with a UTF-8 path.",
            resolved.display()
        )));
    }
    Ok(resolved)
}
