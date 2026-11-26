//! Shared application state held by the HTTP/WS/SSE/JSON-RPC transports.

use portable_atomic::AtomicI64;
#[cfg(feature = "http")]
use std::sync::Mutex;
use std::sync::{Arc, atomic::Ordering};

use agena_application::Application;

use crate::error::ServerError;

/// Shared state injected into every handler.
#[derive(Clone)]
pub struct AppState {
    application: Application,
    server: agena_api::resource::ServerIdentityResource,
    next_operator_call_id: Arc<AtomicI64>,
    #[cfg(feature = "http")]
    revisions: Arc<Mutex<Option<Arc<crate::revisions::ResourceRevisions>>>>,
    #[cfg(feature = "http")]
    acp: Arc<std::sync::OnceLock<AppState>>,
}

impl AppState {
    pub fn from_application(application: Application) -> Self {
        let server = application.server_identity().clone();
        Self {
            application,
            server,
            next_operator_call_id: Arc::new(AtomicI64::new(1)),
            #[cfg(feature = "http")]
            revisions: Arc::new(Mutex::new(None)),
            #[cfg(feature = "http")]
            acp: Default::default(),
        }
    }

    pub fn server(&self) -> &agena_api::resource::ServerIdentityResource {
        &self.server
    }

    #[cfg(feature = "http")]
    pub(crate) fn revisions(
        &self,
    ) -> Result<Arc<crate::revisions::ResourceRevisions>, ServerError> {
        let mut revisions = self.revisions.lock().unwrap_or_else(|e| e.into_inner());
        if revisions.is_none() {
            *revisions = Some(Arc::new(crate::revisions::ResourceRevisions::new(self)?));
        }
        Ok(revisions
            .as_ref()
            .expect("initialized revision index")
            .clone())
    }

    pub fn application(&self) -> &Application {
        &self.application
    }

    #[cfg(feature = "http")]
    pub(crate) fn acp_state(&self) -> Result<Self, ServerError> {
        if self.service().is_acp_scope() {
            return Ok(self.clone());
        }
        let application = self.application.acp_application()?;
        Ok(self
            .acp
            .get_or_init(|| Self::from_application(application.clone()))
            .clone())
    }

    #[cfg(feature = "http")]
    pub(crate) fn session_state(&self, id: i64) -> Result<Self, ServerError> {
        if agena_domain::is_acp_session_id(id) {
            return self.acp_state();
        }
        if self.service().is_acp_scope() {
            return Err(ServerError::not_found("The ACP view was not found."));
        }
        Ok(self.clone())
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

    pub fn session_kernel(&self) -> Result<&agena_storage::kernel::SessionKernel, ServerError> {
        self.application.session_kernel().map_err(ServerError::from)
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

/// Resolve a live ACP view on session-specific routes. Existing thin clients
/// can keep using their typed session commands with these process-local IDs.
#[cfg(feature = "http")]
pub struct SessionScope(pub AppState);

#[cfg(feature = "http")]
impl axum::extract::FromRequestParts<AppState> for SessionScope {
    type Rejection = ServerError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let axum::extract::Path(params) = axum::extract::Path::<
            std::collections::HashMap<String, String>,
        >::from_request_parts(parts, state)
        .await
        .map_err(|error| ServerError::bad_request_error(&error))?;
        let id = params.get("session_id").or_else(|| params.get("root_id"));
        let id = id
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| ServerError::bad_request("The session ID is invalid."))?;
        state.session_state(id).map(Self)
    }
}
