use std::{future::Future, pin::Pin, sync::Arc};

use agena_application::Application;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) ui_auth: crate::server::auth::UiAuth,
    pub(crate) terminal: Arc<crate::server::terminal::manager::TerminalManager>,
    pub(crate) workspace_preview_registry:
        Arc<crate::server::preview::registry::WorkspacePreviewRegistry>,
    pub(crate) workspace_preview_runtime:
        Arc<crate::server::preview::runtime::WorkspacePreviewRuntime>,
    // Held for construction of the terminal/preview managers; handlers reach
    // the DB through the manager-held clones.
    #[allow(dead_code)]
    pub(crate) server_state_db: Arc<crate::server::persistence::db::ServerStateDb>,
    /// Application-owned diagnostics and workspace use cases retained for
    /// Server health and workspace-scoped presentation state. Runtime stays
    /// confined to bootstrap composition.
    pub(crate) application: Application,
}

fn parse_git_bool(value: Option<String>, default_value: bool) -> bool {
    match value {
        Some(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => true,
            "false" | "0" | "no" | "off" => false,
            _ => default_value,
        },
        None => default_value,
    }
}

fn git_env_bool(env_key: &'static str, default_value: bool) -> Pin<Box<dyn Future<Output = bool> + Send + 'static>> {
    let value = std::env::var(env_key).ok();
    Box::pin(async move { parse_git_bool(value, default_value) })
}

impl agena_git_http::GitHttpState for AppState {
    fn git_allow_force_push(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        git_env_bool("AGENA_GIT_ALLOW_FORCE_PUSH", false)
    }

    fn git_allow_no_verify_commit(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        git_env_bool("AGENA_GIT_ALLOW_NO_VERIFY_COMMIT", false)
    }

    fn git_enforce_branch_protection(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        git_env_bool("AGENA_GIT_ENFORCE_BRANCH_PROTECTION", false)
    }

    fn git_strict_patch_validation(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        git_env_bool("AGENA_GIT_STRICT_PATCH_VALIDATION", false)
    }

    fn git_branch_protection_prompt(
        &self,
        _branch: String,
    ) -> Pin<Box<dyn Future<Output = Option<String>> + Send + '_>> {
        // The git branch-protection prompt previously read from the stripped
        // server settings module; without it the git handlers degrade to
        // prompting behavior via the client. Keep it a no-op here.
        Box::pin(async { None })
    }
}
