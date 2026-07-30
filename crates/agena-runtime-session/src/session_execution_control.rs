//! Runtime-facing control operations for an already-composed session service.
//!
//! This deliberately exposes only ID-based lifecycle control. Message,
//! transcript, event, and persistence projections are concrete adapter work
//! and must not leak through this contract while they are still core-owned.

use std::path::Path;

use async_trait::async_trait;
use thiserror::Error;

use agena_domain::{
    CancellationResult, ExecutionId, ExecutionLifecycle, ModelRef, SessionCacheStats,
};
use agena_tool::SnapshotBackend;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("session execution control failed: {message}")]
pub struct SessionExecutionControlError {
    message: String,
}

impl SessionExecutionControlError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Stable active-snapshot projection for callers that inspect execution state.
/// The Runtime retains the mutable registry and snapshot-tool composition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeActiveSnapshot {
    pub session_id: i64,
    pub path: String,
    pub branch: String,
    pub backend: SnapshotBackend,
    pub created_here: bool,
}

/// Stable managed-snapshot projection for callers that inspect workspace state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeManagedSnapshot {
    pub stale: bool,
    pub path: String,
    pub session_id: Option<i64>,
    pub branch: Option<String>,
    pub backend: Option<SnapshotBackend>,
    pub registered_with_git: bool,
    pub registered_with_rift: bool,
}

/// Snapshot state available through the composed execution-control service.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeSnapshotStatus {
    pub active: Vec<RuntimeActiveSnapshot>,
    pub managed: Vec<RuntimeManagedSnapshot>,
}

/// Narrow control port for a composed session execution service.
#[async_trait]
pub trait SessionExecutionControl: Send + Sync {
    /// Returns the current registry-backed lifecycle, if an execution exists.
    async fn active_execution(&self, session_id: i64) -> Option<ExecutionLifecycle>;

    /// Requests cancellation for the active execution identified by the
    /// session. Implementations may cancel descendants as part of their
    /// concrete orchestration policy.
    async fn cancel_execution(
        &self,
        session_id: i64,
        execution_id: ExecutionId,
    ) -> Result<CancellationResult, SessionExecutionControlError>;

    /// Lists scheduler-owned automation jobs visible to the composed session
    /// service. The job contract is independent of core transcript state.
    async fn list_scheduled_jobs(&self) -> Vec<agena_scheduler::ScheduledJob>;

    /// Whether the composed service currently has scheduler support.
    fn scheduler_available(&self) -> bool;

    /// Resolves the persisted model selection without exposing the concrete
    /// session/message projection that stores it.
    async fn selected_model(
        &self,
        session_id: i64,
    ) -> Result<Option<ModelRef>, SessionExecutionControlError>;

    /// Returns cache telemetry without exposing the concrete session cache.
    fn cache_stats(&self) -> SessionCacheStats;

    /// Projects managed snapshot state when the composed service has snapshot
    /// support. The Runtime retains the registry and concrete snapshot tools.
    fn snapshot_status(&self, workspace_root: &Path) -> Option<RuntimeSnapshotStatus>;
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Mutex};

    use agena_domain::{CancellationResult, ExecutionId, ExecutionLifecycle, ExecutionPhase};
    use uuid::Uuid;

    use super::{RuntimeSnapshotStatus, SessionExecutionControl, SessionExecutionControlError};

    struct FakeControl {
        cancelled: Mutex<Vec<i64>>,
    }

    #[async_trait::async_trait]
    impl SessionExecutionControl for FakeControl {
        async fn active_execution(&self, session_id: i64) -> Option<ExecutionLifecycle> {
            (session_id == 7).then_some(ExecutionLifecycle::Active {
                execution_id: ExecutionId(Uuid::nil()),
                phase: ExecutionPhase::StreamingModel,
            })
        }

        async fn cancel_execution(
            &self,
            session_id: i64,
            _execution_id: ExecutionId,
        ) -> Result<CancellationResult, SessionExecutionControlError> {
            self.cancelled
                .lock()
                .expect("lock cancelled")
                .push(session_id);
            Ok(CancellationResult::CancellationRequested)
        }

        async fn list_scheduled_jobs(&self) -> Vec<agena_scheduler::ScheduledJob> {
            Vec::new()
        }

        fn scheduler_available(&self) -> bool {
            false
        }

        async fn selected_model(
            &self,
            _session_id: i64,
        ) -> Result<Option<agena_domain::ModelRef>, SessionExecutionControlError> {
            Ok(None)
        }

        fn cache_stats(&self) -> agena_domain::SessionCacheStats {
            agena_domain::SessionCacheStats::default()
        }

        fn snapshot_status(&self, _workspace_root: &Path) -> Option<RuntimeSnapshotStatus> {
            None
        }
    }

    #[tokio::test]
    async fn trait_object_keeps_lifecycle_values_outside_the_adapter() {
        let control: &dyn SessionExecutionControl = &FakeControl {
            cancelled: Mutex::new(Vec::new()),
        };
        assert!(matches!(
            control.active_execution(7).await,
            Some(ExecutionLifecycle::Active { .. })
        ));
        control
            .cancel_execution(7, ExecutionId(Uuid::nil()))
            .await
            .expect("cancel through port");
    }
}
