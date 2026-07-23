use std::sync::Arc;

use agena_api::subscribe::SubscribeRequest;
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

    pub fn event_stream_service(
        &self,
    ) -> Result<Arc<dyn agena_runtime::RuntimeEventStreamService>, ServerError> {
        self.application
            .event_stream_service()
            .map_err(ServerError::from)
    }
}

impl std::ops::Deref for AppState {
    type Target = Application;

    fn deref(&self) -> &Self::Target {
        &self.application
    }
}

/// Maps the public wire subscription request onto the concrete runtime event
/// filter. This conversion belongs in the transport adapter, not in the API
/// contract crate.
pub(crate) fn event_filter_from_subscribe(request: SubscribeRequest) -> agena_domain::EventFilter {
    let scope = match request.scope {
        agena_api::Scope::Global => agena_domain::EventScope::Global,
        agena_api::Scope::Workspace { workspace_id } => {
            agena_domain::EventScope::Workspace { workspace_id }
        }
        agena_api::Scope::Session { session_id } => {
            agena_domain::EventScope::Session { session_id }
        }
    };
    agena_domain::EventFilter {
        scope,
        kinds: request.kinds,
        since_seq_global: request.since_seq_global,
    }
}
