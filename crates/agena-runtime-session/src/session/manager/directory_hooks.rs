use std::path::Path;

use agena_plugin_host::sdk::{HookSubscription, ImportedDirectoryAddedInput};

use super::*;

impl SessionManager {
    /// Queue the observer after an additional working directory is committed.
    pub fn dispatch_imported_directory_added(&self, session: i64, directory: &Path) {
        let Ok(id) = session_id(session) else {
            return;
        };
        if self.shutdown.is_cancelled() {
            return;
        }
        let Some(cancel) = self.imported_shells.session_lifetime(session) else {
            return;
        };
        let manager = self.clone();
        let directory = directory.to_path_buf();
        let shutdown = self.shutdown.clone();
        self.control_tasks.spawn(async move {
            let observe = async {
                manager
                    .imported_file_watches
                    .refresh_session_and_wait(session)
                    .await;
                let Ok(state) = manager.state_for_session(id).await else {
                    return;
                };
                let Ok(context) = manager.execution_context_with_state(id, &state).await else {
                    return;
                };
                if context.global
                    || context.owner.is_some()
                    || context.channel_authority.is_some()
                    || context.relation_kind.is_subagent()
                    || !context.additional_working_directories.contains(&directory)
                    || context
                        .conversation
                        .as_ref()
                        .is_some_and(|view| view.is_read_only())
                {
                    return;
                }
                let executor = state.executor.for_session_resource_context(&context);
                let plugins = executor.plugin_manager();
                if !plugins.has_subscription(HookSubscription::IMPORTED_DIRECTORY_ADDED) {
                    return;
                }
                let cwd = executor
                    .imported_shell_cwd()
                    .unwrap_or_else(|| executor.workspace_root().to_path_buf());
                plugins
                    .dispatch_imported_directory_added(
                        ImportedDirectoryAddedInput {
                            session_id: session,
                            directory,
                            cwd,
                        },
                        |key| executor.plugin_is_enabled(key),
                        &cancel,
                    )
                    .await;
            };
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => {}
                _ = cancel.cancelled() => {}
                _ = observe => {}
            }
        });
    }
}
