use std::{future::Future, pin::Pin, sync::Arc};

use agena_application::Application;
use axum_extra::extract::cookie::SameSite;
use serde_json::Value;
use tokio::sync::RwLock;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) ui_auth: crate::server::auth::UiAuth,
    pub(crate) ui_cookie_same_site: SameSite,
    pub(crate) cors_allowed_origins: Vec<String>,
    pub(crate) cors_allow_all: bool,
    pub(crate) terminal: Arc<crate::server::terminal::manager::TerminalManager>,
    pub(crate) attachment_cache: Arc<crate::server::attachment::AttachmentCacheManager>,
    pub(crate) workspace_preview_registry:
        Arc<crate::server::preview::registry::WorkspacePreviewRegistry>,
    pub(crate) workspace_preview_runtime:
        Arc<crate::server::preview::runtime::WorkspacePreviewRuntime>,
    pub(crate) server_state_db: Arc<crate::server::persistence::db::ServerStateDb>,
    pub(crate) settings: Arc<RwLock<crate::server::settings::Settings>>,
    /// Application-owned diagnostics and workspace use cases retained for
    /// Server health and workspace-scoped presentation state. Runtime stays
    /// confined to bootstrap composition.
    pub(crate) application: Application,
}

fn parse_git_bool(value: Option<&Value>, default_value: bool) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value
            .as_i64()
            .map(|value| value != 0)
            .unwrap_or(default_value),
        Some(Value::String(value)) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => true,
            "false" | "0" | "no" | "off" => false,
            _ => default_value,
        },
        _ => default_value,
    }
}

fn git_bool_setting(
    settings: Arc<RwLock<crate::server::settings::Settings>>,
    env_key: &'static str,
    settings_key: &'static str,
    default_value: bool,
) -> Pin<Box<dyn Future<Output = bool> + Send + 'static>> {
    Box::pin(async move {
        if let Ok(value) = std::env::var(env_key) {
            return parse_git_bool(Some(&Value::String(value)), default_value);
        }
        let settings = settings.read().await;
        parse_git_bool(settings.extra.get(settings_key), default_value)
    })
}

impl agena_git_http::GitHttpState for AppState {
    fn git_allow_force_push(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        git_bool_setting(
            Arc::clone(&self.settings),
            "AGENA_GIT_ALLOW_FORCE_PUSH",
            "gitAllowForcePush",
            false,
        )
    }

    fn git_allow_no_verify_commit(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        git_bool_setting(
            Arc::clone(&self.settings),
            "AGENA_GIT_ALLOW_NO_VERIFY_COMMIT",
            "gitAllowNoVerifyCommit",
            false,
        )
    }

    fn git_enforce_branch_protection(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        git_bool_setting(
            Arc::clone(&self.settings),
            "AGENA_GIT_ENFORCE_BRANCH_PROTECTION",
            "gitEnforceBranchProtection",
            false,
        )
    }

    fn git_strict_patch_validation(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        git_bool_setting(
            Arc::clone(&self.settings),
            "AGENA_GIT_STRICT_PATCH_VALIDATION",
            "gitStrictPatchValidation",
            false,
        )
    }

    fn git_branch_protection_prompt(
        &self,
        branch: String,
    ) -> Pin<Box<dyn Future<Output = Option<String>> + Send + '_>> {
        let settings = Arc::clone(&self.settings);
        Box::pin(async move {
            let settings = settings.read().await;
            let protected = settings
                .extra
                .get("gitBranchProtection")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|rule| !rule.is_empty())
                .any(|rule| {
                    let pattern = regex::escape(rule).replace("\\*", ".*").replace("\\?", ".");
                    regex::Regex::new(&format!("^{pattern}$"))
                        .map(|regex| regex.is_match(branch.trim()))
                        .unwrap_or(false)
                });
            if !protected {
                return None;
            }
            Some(
                settings
                    .extra
                    .get("gitBranchProtectionPrompt")
                    .and_then(Value::as_str)
                    .filter(|value| {
                        value.eq_ignore_ascii_case("alwaysCommit")
                            || value.eq_ignore_ascii_case("alwaysCommitToNewBranch")
                    })
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "alwaysPrompt".to_string()),
            )
        })
    }
}
