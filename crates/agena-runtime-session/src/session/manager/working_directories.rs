use std::path::{Path, PathBuf};

use agena_domain::{
    AccessKind, RegisterSessionWorkingDirectory, SessionWorkingDirectoryRegistration,
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
            .spawn(async move { manager.register_directory(session, &request.path).await })
            .await
            .map_err(|error| {
                AppError::Internal(format!("directory registration task failed: {error}"))
            })?
    }

    async fn register_directory(
        &self,
        session: SessionId,
        path: &Path,
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

            let already_covered = std::iter::once(&root)
                .chain(context.additional_working_directories.iter())
                .any(|registered| directory.starts_with(registered));
            if !already_covered {
                context
                    .additional_working_directories
                    .push(directory.clone());
                let executor = state.executor.for_session_resource_context(&context);
                let read =
                    executor.path_permission_check(AccessKind::Read, &directory.to_string_lossy());
                self.authorize_session_scoped_checks(session.get(), &executor, vec![read])
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
                )
                .await
            {
                Ok(receipt) => {
                    if receipt.value.added {
                        self.dispatch_imported_directory_added(
                            session.get(),
                            &receipt.value.directory,
                        );
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
