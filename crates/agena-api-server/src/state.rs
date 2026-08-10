//! Shared application state held by the HTTP/WS/SSE/JSON-RPC transports.

use std::sync::Arc;

use agena_application::Application;

use crate::error::ServerError;

/// Shared state injected into every handler.
#[derive(Clone)]
pub struct AppState {
    application: Application,
}

impl AppState {
    pub fn from_application(application: Application) -> Self {
        Self { application }
    }

    pub fn application(&self) -> &Application {
        &self.application
    }

    pub fn service(&self) -> &agena_application::service::ApplicationService {
        self.application.service()
    }

    pub fn plugin_runtime(&self) -> &Arc<dyn agena_runtime::PluginRuntimeService> {
        self.application.plugin_runtime()
    }

    pub fn runtime_control(&self) -> &Arc<dyn agena_runtime::RuntimeControlService> {
        self.application.runtime_control()
    }

    pub fn session_store(
        &self,
    ) -> Result<Arc<dyn agena_storage::store::SessionStore>, ServerError> {
        self.application
            .session_store_facade()
            .map_err(ServerError::from)
    }

    pub fn live_signals(
        &self,
    ) -> Result<Arc<dyn agena_runtime::RuntimeLiveSignalService>, ServerError> {
        self.application
            .live_signal_service()
            .map_err(ServerError::from)
    }
}

impl std::ops::Deref for AppState {
    type Target = Application;

    fn deref(&self) -> &Self::Target {
        &self.application
    }
}
