//! Shared application state held by the HTTP/WS/SSE/JSON-RPC transports.

use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use agena_application::Application;

use crate::error::ServerError;

/// Shared state injected into every handler.
#[derive(Clone)]
pub struct AppState {
    application: Application,
    server: agena_api::resource::ServerIdentityResource,
    next_operator_call_id: Arc<AtomicI64>,
}

impl AppState {
    pub fn from_application(application: Application) -> Self {
        Self {
            application,
            server: agena_api::resource::ServerIdentityResource {
                id: uuid::Uuid::new_v4(),
                pid: std::process::id(),
                started_at: chrono::Utc::now(),
                protocol_version: agena_api::PROTOCOL_VERSION,
            },
            next_operator_call_id: Arc::new(AtomicI64::new(1)),
        }
    }

    pub fn server(&self) -> &agena_api::resource::ServerIdentityResource {
        &self.server
    }

    pub fn application(&self) -> &Application {
        &self.application
    }

    pub fn next_operator_call_id(&self) -> i64 {
        self.next_operator_call_id.fetch_add(1, Ordering::SeqCst)
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
