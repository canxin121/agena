use std::path::{Path, PathBuf};

use agena_plugin_host::sdk::{
    HookSubscription, ImportedConfigChangeInput, ImportedConfigChangeSource,
};

use super::*;

impl SessionManager {
    pub(super) fn track_imported_config_session(&self, session: SessionId) {
        // A reload can enable ConfigChange hooks while this session is idle.
        self.config_hook_sessions.lock().insert(session);
    }

    /// Run against the previous runtime and its permission snapshot before an
    /// automatic reload can publish a changed file.
    pub async fn validate_watched_claude_config(
        &self,
        workspace: &Path,
        changes: &[(ImportedConfigChangeSource, PathBuf)],
    ) -> Result<(), AppError> {
        if !self
            .tool_executor()
            .plugin_manager()
            .has_subscription(HookSubscription::IMPORTED_CONFIG_CHANGE)
        {
            return Ok(());
        }
        let sessions = self
            .config_hook_sessions
            .lock()
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for session in sessions {
            let Ok(state) = self.state_for_session(session).await else {
                continue;
            };
            let Ok(context) = self.execution_context_with_state(session, &state).await else {
                continue;
            };
            if context.global
                || context.owner.is_some()
                || context.channel_authority.is_some()
                || context.relation_kind.is_subagent()
                || context
                    .conversation
                    .as_ref()
                    .is_some_and(|view| view.is_read_only())
            {
                continue;
            }
            let executor = state.executor.for_session_resource_context(&context);
            let plugins = executor.plugin_manager();
            for (source, path) in changes {
                let project_change = matches!(
                    source,
                    ImportedConfigChangeSource::ProjectSettings
                        | ImportedConfigChangeSource::LocalSettings
                        | ImportedConfigChangeSource::Skills
                );
                if project_change && executor.workspace_root() != workspace {
                    continue;
                }
                if !self.config_hook_sessions.lock().contains(&session) {
                    break;
                }
                let blocked = plugins
                    .dispatch_imported_config_change(
                        ImportedConfigChangeInput {
                            session_id: session.get(),
                            source: *source,
                            file_path: path.clone(),
                        },
                        |key| executor.plugin_is_enabled(key),
                        &self.shutdown,
                    )
                    .await
                    .is_some();
                if project_change && blocked {
                    return Err(AppError::Config(format!(
                        "A ConfigChange hook blocked the reload of {}",
                        path.display()
                    )));
                }
            }
        }
        Ok(())
    }
}
