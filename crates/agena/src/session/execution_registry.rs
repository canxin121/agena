//! Per-session execution registry: exclusive ownership, cancel, and steer.
//!
//! Each in-flight execution registers an `ExecutionControl` with its session id; the
//! control owns a `CancellationToken` (so external callers can cancel the
//! execution) and a `mpsc::UnboundedSender<Vec<PartContent>>` (so external
//! callers can inject "steer" messages at the next inter-run boundary).
//!
//! This is intentionally an in-process structure — when the API server
//! lives on a different host this gets fronted by a remote-control RPC.

use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{
    message::PartContent,
    session::{ExecutionId, ExecutionLifecycle, ExecutionOutcome, ExecutionPhase},
};

/// Errors surfaced by cancellation, steering, and exclusive registration.
#[derive(Debug, thiserror::Error)]
pub enum ExecutionControlError {
    #[error("no active execution for session {0}")]
    NoActiveExecution(i64),
    #[error("session {0} already has an active execution")]
    AlreadyActive(i64),
    #[error("run no longer accepts steer input (channel closed)")]
    SteerClosed,
    #[error("invalid execution transition: {0}")]
    InvalidTransition(String),
}

#[derive(Debug)]
pub struct ExecutionControl {
    execution_id: ExecutionId,
    pub cancel: CancellationToken,
    pub steer_tx: mpsc::UnboundedSender<Vec<PartContent>>,
    lifecycle: Mutex<ExecutionLifecycle>,
    task_abort: Mutex<Option<tokio::task::AbortHandle>>,
}

const GRACEFUL_CANCELLATION_TIMEOUT: Duration = Duration::from_millis(500);

impl ExecutionControl {
    fn new(steer_tx: mpsc::UnboundedSender<Vec<PartContent>>) -> Self {
        let execution_id = ExecutionId::new();
        Self {
            execution_id,
            cancel: CancellationToken::new(),
            steer_tx,
            lifecycle: Mutex::new(ExecutionLifecycle::start(execution_id)),
            task_abort: Mutex::new(None),
        }
    }

    pub fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }

    pub async fn transition(&self, phase: ExecutionPhase) -> Result<(), ExecutionControlError> {
        self.lifecycle
            .lock()
            .await
            .transition(phase)
            .map_err(|error| ExecutionControlError::InvalidTransition(error.to_string()))
    }

    pub async fn finish(&self, outcome: ExecutionOutcome) -> Result<(), ExecutionControlError> {
        self.lifecycle
            .lock()
            .await
            .finish(outcome)
            .map_err(|error| ExecutionControlError::InvalidTransition(error.to_string()))
    }

    pub async fn lifecycle(&self) -> ExecutionLifecycle {
        self.lifecycle.lock().await.clone()
    }

    pub async fn attach_task_abort(&self, abort: tokio::task::AbortHandle) {
        *self.task_abort.lock().await = Some(abort);
    }

    pub async fn clear_task_abort(&self) {
        self.task_abort.lock().await.take();
    }

    async fn abort_task(&self) {
        if let Some(abort) = self.task_abort.lock().await.as_ref() {
            abort.abort();
        }
    }
}

#[derive(Debug, Default)]
pub struct ExecutionRegistry {
    inner: Mutex<HashMap<i64, Arc<ExecutionControl>>>,
}

impl ExecutionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire the session's single execution-writer slot.
    ///
    /// The returned receiver belongs exclusively to that execution. A second
    /// acquisition fails instead of implicitly cancelling or replacing the
    /// current owner.
    pub async fn register(
        &self,
        session_id: i64,
    ) -> Result<
        (
            Arc<ExecutionControl>,
            mpsc::UnboundedReceiver<Vec<PartContent>>,
        ),
        ExecutionControlError,
    > {
        let (tx, rx) = mpsc::unbounded_channel();
        let control = Arc::new(ExecutionControl::new(tx));
        let mut guard = self.inner.lock().await;
        if guard.contains_key(&session_id) {
            return Err(ExecutionControlError::AlreadyActive(session_id));
        }
        guard.insert(session_id, Arc::clone(&control));
        Ok((control, rx))
    }

    /// Remove the control for `session_id` if it still matches `expected`.
    /// Used on completion so stale cleanup can never remove a newer owner.
    pub async fn unregister_if_matches(&self, session_id: i64, expected: &Arc<ExecutionControl>) {
        let mut guard = self.inner.lock().await;
        if let Some(current) = guard.get(&session_id)
            && Arc::ptr_eq(current, expected)
        {
            guard.remove(&session_id);
        }
    }

    pub async fn cancel(&self, session_id: i64) -> Result<(), ExecutionControlError> {
        let control = self
            .inner
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .ok_or(ExecutionControlError::NoActiveExecution(session_id))?;
        control.transition(ExecutionPhase::Cancelling).await?;
        control.cancel.cancel();
        // Match the mature CLI pattern: give provider streams and tools a
        // short cooperative window to persist their terminal state, then
        // hard-abort the owning Tokio task so a buggy adapter or plugin can
        // never hold the UI hostage indefinitely.
        tokio::spawn(async move {
            tokio::time::sleep(GRACEFUL_CANCELLATION_TIMEOUT).await;
            control.abort_task().await;
        });
        Ok(())
    }

    pub async fn cancellation_token(&self, session_id: i64) -> Option<CancellationToken> {
        self.inner
            .lock()
            .await
            .get(&session_id)
            .map(|control| control.cancel.clone())
    }

    pub async fn steer(
        &self,
        session_id: i64,
        parts: Vec<PartContent>,
    ) -> Result<(), ExecutionControlError> {
        let guard = self.inner.lock().await;
        let control = guard
            .get(&session_id)
            .ok_or(ExecutionControlError::NoActiveExecution(session_id))?;
        control
            .steer_tx
            .send(parts)
            .map_err(|_| ExecutionControlError::SteerClosed)
    }

    pub async fn is_active(&self, session_id: i64) -> bool {
        self.inner.lock().await.contains_key(&session_id)
    }

    pub async fn execution(&self, session_id: i64) -> Option<ExecutionLifecycle> {
        let control = self.inner.lock().await.get(&session_id).cloned()?;
        Some(control.lifecycle().await)
    }

    pub async fn active_session_ids(&self) -> Vec<i64> {
        self.inner.lock().await.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_session_has_exactly_one_execution_writer() {
        let registry = ExecutionRegistry::new();
        let (first, _) = registry.register(7).await.expect("first execution");
        assert!(matches!(
            registry.register(7).await,
            Err(ExecutionControlError::AlreadyActive(7))
        ));

        registry.unregister_if_matches(7, &first).await;
        assert!(registry.register(7).await.is_ok());
    }

    #[tokio::test]
    async fn cancellation_moves_to_cancelling_before_signalling_worker() {
        let registry = ExecutionRegistry::new();
        let (control, _) = registry.register(9).await.expect("execution");
        registry.cancel(9).await.expect("cancel");

        assert!(control.cancel.is_cancelled());
        assert!(matches!(
            control.lifecycle().await,
            ExecutionLifecycle::Active {
                phase: ExecutionPhase::Cancelling,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn unregister_never_removes_a_different_execution() {
        let registry = ExecutionRegistry::new();
        let (control, _) = registry.register(11).await.expect("execution");
        let unrelated = Arc::new(ExecutionControl::new(mpsc::unbounded_channel().0));

        registry.unregister_if_matches(11, &unrelated).await;
        assert!(registry.is_active(11).await);
        registry.unregister_if_matches(11, &control).await;
        assert!(!registry.is_active(11).await);
    }

    #[tokio::test]
    async fn cancellation_hard_aborts_an_uncooperative_execution_after_grace() {
        let registry = ExecutionRegistry::new();
        let (control, _) = registry.register(13).await.expect("execution");
        let task = tokio::spawn(std::future::pending::<()>());
        control.attach_task_abort(task.abort_handle()).await;

        registry.cancel(13).await.expect("cancel");

        let join = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("watchdog should bound cancellation latency");
        assert!(join.expect_err("task should be aborted").is_cancelled());
    }
}
