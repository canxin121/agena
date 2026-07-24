//! Generic per-owner execution registry: exclusive ownership, cancel, and
//! steer. Concrete session code supplies its own steer payload type.

use std::{collections::HashMap, sync::Arc, time::Duration};

use agena_domain::{ExecutionId, ExecutionLifecycle, ExecutionOutcome, ExecutionPhase};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

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
pub struct ExecutionControl<T> {
    execution_id: ExecutionId,
    pub cancel: CancellationToken,
    pub steer_tx: mpsc::UnboundedSender<Vec<T>>,
    lifecycle: Mutex<ExecutionLifecycle>,
    operation_abort: Mutex<Option<tokio::task::AbortHandle>>,
}

const OPERATION_CANCELLATION_GRACE: Duration = Duration::from_millis(500);

impl<T> ExecutionControl<T> {
    fn new(steer_tx: mpsc::UnboundedSender<Vec<T>>) -> Self {
        let execution_id = ExecutionId::new();
        Self {
            execution_id,
            cancel: CancellationToken::new(),
            steer_tx,
            lifecycle: Mutex::new(ExecutionLifecycle::start(execution_id)),
            operation_abort: Mutex::new(None),
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

    pub async fn attach_operation_abort(&self, abort: tokio::task::AbortHandle) {
        *self.operation_abort.lock().await = Some(abort);
    }

    pub async fn clear_operation_abort(&self) {
        self.operation_abort.lock().await.take();
    }

    async fn abort_operation(&self) {
        if let Some(abort) = self.operation_abort.lock().await.as_ref() {
            abort.abort();
        }
    }
}

#[derive(Debug)]
pub struct ExecutionRegistry<T> {
    inner: Mutex<HashMap<i64, Arc<ExecutionControl<T>>>>,
}

impl<T> Default for ExecutionRegistry<T> {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl<T: Send + 'static> ExecutionRegistry<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(
        &self,
        session_id: i64,
    ) -> Result<(Arc<ExecutionControl<T>>, mpsc::UnboundedReceiver<Vec<T>>), ExecutionControlError>
    {
        let (tx, rx) = mpsc::unbounded_channel();
        let control = Arc::new(ExecutionControl::new(tx));
        let mut guard = self.inner.lock().await;
        if guard.contains_key(&session_id) {
            return Err(ExecutionControlError::AlreadyActive(session_id));
        }
        guard.insert(session_id, Arc::clone(&control));
        Ok((control, rx))
    }

    pub async fn unregister_if_matches(
        &self,
        session_id: i64,
        expected: &Arc<ExecutionControl<T>>,
    ) {
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
        tokio::spawn(async move {
            tokio::time::sleep(OPERATION_CANCELLATION_GRACE).await;
            control.abort_operation().await;
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

    pub async fn steer(&self, session_id: i64, parts: Vec<T>) -> Result<(), ExecutionControlError> {
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
    use std::sync::Arc;

    use agena_domain::{ExecutionLifecycle, ExecutionPhase};
    use tokio::sync::mpsc;

    use super::{ExecutionControl, ExecutionControlError, ExecutionRegistry};

    #[tokio::test]
    async fn a_owner_has_exactly_one_execution_writer() {
        let registry = ExecutionRegistry::<()>::new();
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
        let registry = ExecutionRegistry::<()>::new();
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
        let registry = ExecutionRegistry::<()>::new();
        let (control, _) = registry.register(11).await.expect("execution");
        let unrelated = Arc::new(ExecutionControl::new(mpsc::unbounded_channel().0));
        registry.unregister_if_matches(11, &unrelated).await;
        assert!(registry.is_active(11).await);
        registry.unregister_if_matches(11, &control).await;
        assert!(!registry.is_active(11).await);
    }
}
