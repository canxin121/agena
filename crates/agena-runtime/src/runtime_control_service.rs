//! Lifecycle and background-task controls for an already-composed runtime.

use std::{future::Future, path::Path, pin::Pin};

use async_trait::async_trait;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use agena_provider::ProviderClientVersions;
use agena_tool::SnapshotBackendCapabilities;

use crate::{
    RuntimeBackgroundTask, RuntimeBackgroundTaskControlError, RuntimeBackgroundTaskKind,
    RuntimeBackgroundTaskOrigin, RuntimeBackgroundTaskOutcome, RuntimeBackgroundTaskStart,
    RuntimeReloadCause, RuntimeReloadReport,
};

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("runtime control failed: {message}")]
/// Error from the runtime control service.
pub struct RuntimeControlServiceError {
    message: String,
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
        Self {
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
    async fn reload(&self) -> Result<RuntimeReloadReport, RuntimeControlServiceError>;

    /// Fetches provider client-version data through the Runtime-owned network
    /// effect without exposing its HTTP helper to an upper-layer use case.
    async fn fetch_provider_client_versions(
        &self,
    ) -> Result<ProviderClientVersions, RuntimeControlServiceError>;

    /// Reads the Runtime-owned process telemetry through the composed control
    /// capability rather than a global public helper.
    fn runtime_metrics(&self) -> crate::RuntimeMetricsSnapshot;

    /// Probes the Runtime-owned managed-snapshot backends without exposing the
    /// concrete process probe as a Runtime root free function to Application.
    fn snapshot_backend_capabilities(&self, workspace: &Path) -> SnapshotBackendCapabilities;

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
