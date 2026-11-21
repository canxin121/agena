//! Lifecycle and background-task controls for an already-composed runtime.

use std::{future::Future, pin::Pin};

use async_trait::async_trait;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use agena_provider::ProviderClientVersions;

use crate::{
    RuntimeBackgroundTask, RuntimeBackgroundTaskControlError, RuntimeBackgroundTaskKind,
    RuntimeBackgroundTaskOrigin, RuntimeBackgroundTaskOutcome, RuntimeBackgroundTaskStart,
    RuntimeReloadCause, RuntimeReloadReport,
};

#[derive(Debug, Clone, Error, PartialEq, Eq)]
/// Error from the runtime control service.
pub enum RuntimeControlServiceError {
    #[error("runtime is shut down")]
    Shutdown,
    #[error("runtime control failed: {message}")]
    Operation { message: String },
}

/// Object-safe work item for a runtime-managed background task.  The concrete
/// runtime owns task registration, cancellation, deduplication, and outcome
/// tracking; callers supply only the isolated work itself.
pub type RuntimeBackgroundTaskWork = Box<
    dyn FnOnce(
            CancellationToken,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = Result<RuntimeBackgroundTaskOutcome, RuntimeControlServiceError>,
                    > + Send,
            >,
        > + Send,
>;

impl RuntimeControlServiceError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Operation {
            message: message.into(),
        }
    }

    pub fn from_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::new(agena_failure::diagnostic::format_error_chain(error))
    }
}

#[async_trait]
/// Service for controlling the runtime.
pub trait RuntimeControlService: Send + Sync {
    async fn browser_control(
        &self,
        _session_id: i64,
        _command: agena_plugin_sdk::browser::BrowserControlCommand,
    ) -> agena_plugin_sdk::error::Result<agena_plugin_sdk::browser::BrowserControlResult> {
        Err(agena_plugin_sdk::PluginError::from_kind(
            agena_plugin_sdk::PluginErrorKind::HostUnavailable,
            "Browser control is unavailable",
        ))
    }

    fn session_processes(
        &self,
        _session_id: i64,
        _websocket: bool,
    ) -> Vec<agena_domain::ProcessSummary> {
        Vec::new()
    }

    fn session_process(
        &self,
        _session_id: i64,
        _id: &str,
        _websocket: bool,
    ) -> Option<agena_domain::ProcessSummary> {
        None
    }

    async fn stop_session_process(
        &self,
        _session_id: i64,
        _id: &str,
        _websocket: bool,
    ) -> Result<Option<agena_domain::ProcessSummary>, RuntimeControlServiceError> {
        Ok(None)
    }

    /// The active client bridge, absent when the built-in MCP plugin is disabled.
    fn mcp_connection_manager(
        &self,
    ) -> Option<std::sync::Arc<agena_mcp_client::McpConnectionManager>> {
        None
    }

    /// Finish runtime activation after the application backend has been attached.
    async fn application_ready(&self) -> Result<(), RuntimeControlServiceError> {
        Ok(())
    }

    async fn reload(&self) -> Result<RuntimeReloadReport, RuntimeControlServiceError>;

    /// Fetches provider client-version data through the Runtime-owned network
    /// effect without exposing its HTTP helper to an upper-layer use case.
    async fn fetch_provider_client_versions(
        &self,
    ) -> Result<ProviderClientVersions, RuntimeControlServiceError>;

    /// Reads the Runtime-owned process telemetry through the composed control
    /// capability rather than a global public helper.
    fn runtime_metrics(&self) -> crate::RuntimeMetricsSnapshot;

    fn start_runtime_reload_task(
        &self,
        cause: RuntimeReloadCause,
        origin: RuntimeBackgroundTaskOrigin,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError>;

    fn start_background_task(
        &self,
        kind: RuntimeBackgroundTaskKind,
        origin: RuntimeBackgroundTaskOrigin,
        title: String,
        dedupe_key: Option<String>,
        cancellable: bool,
        work: RuntimeBackgroundTaskWork,
    ) -> Result<RuntimeBackgroundTaskStart, RuntimeBackgroundTaskControlError>;

    fn background_tasks(&self) -> Vec<RuntimeBackgroundTask>;

    fn cancel_background_task(
        &self,
        task_id: &str,
    ) -> Result<RuntimeBackgroundTask, RuntimeBackgroundTaskControlError>;
}
