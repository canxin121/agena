use std::sync::Arc;

use agena_application::Application;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) ui_auth: crate::server::auth::UiAuth,
    /// Application-owned diagnostics and workspace use cases retained for
    /// Server health and workspace-scoped presentation state. Runtime stays
    /// confined to bootstrap composition.
    pub(crate) application: Application,
}
