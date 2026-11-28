use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use agena_domain::{AccessKind, PermissionDecision};
use agena_plugin_sdk::{
    PluginError, SessionFileChange, SessionFileChangedInput,
    host_api::{ImportedPluginFileEvents, ImportedPluginWatchPath},
};
use async_trait::async_trait;

use super::{AgenaRuntime, host_unavailable, plugin_error_from_app};
use crate::runtime::builder::WeakAgenaRuntime;
use crate::session::SessionManager;

static PATH_CHECKS: agena_async::BlockingPool = agena_async::BlockingPool::new(4);

struct FileEvents {
    runtime: WeakAgenaRuntime,
    session: i64,
}

pub(super) fn bind(runtime: &AgenaRuntime, session: i64) -> Arc<dyn ImportedPluginFileEvents> {
    Arc::new(FileEvents {
        runtime: runtime.downgrade(),
        session,
    })
}

impl FileEvents {
    fn runtime(&self) -> Result<AgenaRuntime, PluginError> {
        self.runtime
            .upgrade()
            .ok_or_else(|| host_unavailable("the file hook runtime has stopped"))
    }

    async fn authorize_paths(
        &self,
        manager: &SessionManager,
        filenames: Vec<PathBuf>,
        absolute: Vec<PathBuf>,
    ) -> Result<Vec<ImportedPluginWatchPath>, PluginError> {
        let executor = manager
            .host_resource_callback_executor(Some(self.session), None)
            .await
            .map_err(plugin_error_from_app)?;
        if executor.is_global() {
            return Ok(Vec::new());
        }
        PATH_CHECKS
            .run(move || {
                let policy = executor.sandbox_permission_snapshot();
                let cwd = executor
                    .imported_shell_cwd()
                    .unwrap_or_else(|| executor.workspace_root().to_path_buf());
                let permitted = |path: &Path| {
                    matches!(
                        policy.authorize_path_access(AccessKind::Read, path),
                        PermissionDecision::Allow
                    )
                };
                absolute
                    .into_iter()
                    .filter(|path| path.is_absolute())
                    .chain(
                        filenames
                            .into_iter()
                            .filter(|path| {
                                path.components()
                                    .all(|component| matches!(component, Component::Normal(_)))
                            })
                            .map(|path| cwd.join(path)),
                    )
                    .filter(|path| {
                        path.file_name().is_some()
                            && !path
                                .components()
                                .any(|component| component == Component::ParentDir)
                    })
                    .map(|path| path.components().collect::<PathBuf>())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .filter_map(|requested| {
                        let path = requested.to_str()?;
                        if !permitted(&requested) {
                            return None;
                        }
                        let resolved = executor.resolve_target_path(path);
                        permitted(&resolved).then_some(ImportedPluginWatchPath {
                            requested,
                            resolved,
                        })
                    })
                    .collect()
            })
            .await
            .map_err(|_| PluginError::internal("file hook path check worker stopped"))
    }
}

#[async_trait]
impl ImportedPluginFileEvents for FileEvents {
    async fn resolve_paths(
        &self,
        absolute: Vec<PathBuf>,
    ) -> Result<Vec<ImportedPluginWatchPath>, PluginError> {
        let runtime = self.runtime()?;
        let snapshot = runtime.current_snapshot();
        let host = snapshot.plugin_manager();
        if !host.has_subscription(agena_plugin_sdk::HookSubscription::SESSION_FILE_CHANGED) {
            return Ok(Vec::new());
        }
        let manager = snapshot
            .session_manager()
            .ok_or_else(|| host_unavailable("session manager is unavailable"))?;
        let filenames = host.imported_file_watch_filenames().cloned().collect();
        self.authorize_paths(&manager, filenames, absolute).await
    }

    async fn changed(
        &self,
        path: ImportedPluginWatchPath,
        change: SessionFileChange,
    ) -> Result<(), PluginError> {
        let runtime = self.runtime()?;
        let snapshot = runtime.current_snapshot();
        let manager = snapshot
            .session_manager()
            .ok_or_else(|| host_unavailable("session manager is unavailable"))?;
        let paths = self
            .authorize_paths(&manager, Vec::new(), vec![path.requested.clone()])
            .await?;
        if !paths
            .iter()
            .any(|candidate| candidate.resolved == path.resolved)
        {
            return Ok(());
        }
        let paths = runtime
            .current_snapshot()
            .plugin_manager()
            .dispatch_imported_session_file_changed(SessionFileChangedInput {
                session_id: self.session,
                path: path.requested,
                change,
            })
            .await?;
        if let Some(paths) = paths {
            manager
                .imported_file_watches()
                .replace_paths(self.session, paths);
        }
        Ok(())
    }
}
