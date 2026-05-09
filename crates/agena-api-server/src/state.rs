use std::sync::Arc;

use agena::event::EventBus;
use agena::{event::EventKind, runtime::AgenaRuntime, session::SessionManager};
use sea_orm::DatabaseConnection;

use crate::local_api::ApiService;

use crate::error::ServerError;

/// Shared state injected into every handler.
#[derive(Clone)]
pub struct AppState {
    runtime: AgenaRuntime,
    service: ApiService,
    /// Optional override; tests bypass `AgenaRuntime` and inject a manager
    /// directly.
    manager_override: Option<Arc<SessionManager>>,
}

impl AppState {
    pub fn new(runtime: AgenaRuntime, db: Arc<DatabaseConnection>) -> Self {
        let workspace_root = runtime.workspace_root().display().to_string();
        let publisher = runtime
            .session_manager()
            .map(|manager| manager.event_publisher());
        Self {
            runtime,
            service: ApiService::new(db, workspace_root, publisher),
            manager_override: None,
        }
    }

    pub fn with_manager_override(mut self, manager: Arc<SessionManager>) -> Self {
        self.manager_override = Some(manager);
        self
    }

    pub fn runtime(&self) -> &AgenaRuntime {
        &self.runtime
    }

    pub fn service(&self) -> &ApiService {
        &self.service
    }

    pub fn session_manager(&self) -> Result<Arc<SessionManager>, ServerError> {
        self.manager_override
            .clone()
            .or_else(|| self.runtime.session_manager())
            .ok_or_else(|| {
                ServerError::ServiceUnavailable("session manager not initialised".into())
            })
    }

    pub fn event_bus(&self) -> Result<Arc<dyn EventBus<EventKind>>, ServerError> {
        Ok(self.session_manager()?.event_bus())
    }

    pub fn event_publisher(&self) -> Result<Arc<agena::event::EventPublisher>, ServerError> {
        Ok(self.session_manager()?.event_publisher())
    }
}
