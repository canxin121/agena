//! Optional server-listener control shared by HTTP and local plugin operations.
use crate::{Application, ApplicationError};
use agena_api::server_control::{
    McpServerControlResponse, ServerCommand, ServerQuery, UpdateMcpServerControlRequest,
};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock},
};

/// Implemented by a running listener. It owns persistence and listener state.
pub trait ServerControlPort: Send + Sync {
    fn mcp_status(&self) -> Result<McpServerControlResponse, ApplicationError>;
    fn update_mcp(
        &self,
        request: UpdateMcpServerControlRequest,
    ) -> Pin<Box<dyn Future<Output = Result<McpServerControlResponse, ApplicationError>> + Send + '_>>;
}

/// All clones of an application observe the listener installed after bootstrap.
#[derive(Default)]
pub struct ServerControlSlot {
    port: RwLock<Option<Arc<dyn ServerControlPort>>>,
}

impl ServerControlSlot {
    fn get(&self) -> Result<Arc<dyn ServerControlPort>, ApplicationError> {
        self.port
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or_else(|| {
                ApplicationError::service_unavailable("The MCP server listener is not running.")
            })
    }

    fn install(&self, port: Arc<dyn ServerControlPort>) {
        *self.port.write().unwrap_or_else(|error| error.into_inner()) = Some(port);
    }
}

impl Application {
    pub fn install_server_control(&self, port: Arc<dyn ServerControlPort>) {
        self.server_control.install(port);
    }

    pub fn mcp_server_control(&self) -> Result<McpServerControlResponse, ApplicationError> {
        self.server_control.get()?.mcp_status()
    }

    pub async fn update_mcp_server_control(
        &self,
        request: UpdateMcpServerControlRequest,
    ) -> Result<McpServerControlResponse, ApplicationError> {
        self.server_control.get()?.update_mcp(request).await
    }

    pub(crate) async fn dispatch_server_query(
        &self,
        query: ServerQuery,
    ) -> Result<serde_json::Value, ApplicationError> {
        let response = match query {
            ServerQuery::Mcp => self.mcp_server_control()?,
        };
        serde_json::to_value(response).map_err(|error| ApplicationError::internal_error(&error))
    }

    pub(crate) async fn dispatch_server_command(
        &self,
        command: ServerCommand,
    ) -> Result<serde_json::Value, ApplicationError> {
        let response = match command {
            ServerCommand::UpdateMcp(request) => self.update_mcp_server_control(request).await?,
        };
        serde_json::to_value(response).map_err(|error| ApplicationError::internal_error(&error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_listener_is_an_explicit_service_state() {
        let error = match ServerControlSlot::default().get() {
            Err(error) => error,
            Ok(_) => panic!("an uninstalled listener must be unavailable"),
        };
        assert_eq!(
            error.failure.category,
            agena_failure::FailureCategory::DependencyUnavailable
        );
    }
}
