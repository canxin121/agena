//! Generic per-owner execution registry: exclusive ownership, cancel, and
//! steer. Concrete session code supplies its own steer payload type.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use agena_domain::{
    CancellationResult, ExecutionId, ExecutionLifecycle, ExecutionOutcome, ExecutionPhase,
};
use tokio::sync::{Mutex, Notify, mpsc};
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
    turn_id: agena_domain::TurnId,
    reply_id: agena_domain::AssistantReplyId,
    pub cancel: CancellationToken,
    pub steer_tx: mpsc::UnboundedSender<Vec<T>>,
    lifecycle: Mutex<ExecutionLifecycle>,
    operation_abort: Mutex<Option<tokio::task::AbortHandle>>,
    interaction_epoch: AtomicU64,
    interaction_notify: Notify,
}

const OPERATION_CANCELLATION_GRACE: Duration = Duration::from_millis(500);

impl<T> ExecutionControl<T> {
    fn new(
        turn_id: agena_domain::TurnId,
        reply_id: agena_domain::AssistantReplyId,
        steer_tx: mpsc::UnboundedSender<Vec<T>>,
    ) -> Self {
        let execution_id = ExecutionId::new();
        Self {
            execution_id,
            turn_id,
            reply_id,
            cancel: CancellationToken::new(),
            steer_tx,
            lifecycle: Mutex::new(ExecutionLifecycle::start(execution_id)),
            operation_abort: Mutex::new(None),
            interaction_epoch: AtomicU64::new(0),
            interaction_notify: Notify::new(),
        }
    }

    pub fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }

    pub fn turn_id(&self) -> agena_domain::TurnId {
        self.turn_id
    }

    pub fn reply_id(&self) -> agena_domain::AssistantReplyId {
        self.reply_id
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

    /// Observe the durable-interaction signal generation before checking the
    /// session projection. Waiting with this generation is race-free: a reply
    /// persisted between the projection check and the await cannot be lost.
    pub fn interaction_epoch(&self) -> u64 {
        self.interaction_epoch.load(Ordering::Acquire)
    }

    pub fn signal_interaction(&self) {
        self.interaction_epoch.fetch_add(1, Ordering::AcqRel);
        self.interaction_notify.notify_waiters();
    }

    pub async fn wait_for_interaction_after(&self, observed_epoch: u64) {
        loop {
            let notified = self.interaction_notify.notified();
            if self.interaction_epoch() != observed_epoch {
                return;
            }
            notified.await;
        }
    }

    async fn abort_operation(&self) {
        if let Some(abort) = self.operation_abort.lock().await.as_ref() {
            abort.abort();
        }
    }
}

/// How often an executing process refreshes its session lease.
pub const LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Lease binding for an execution registry: the shared database and the
/// per-process owner id that identifies this process's leases.
#[derive(Debug, Clone)]
pub struct LeaseConfig {
    pub db: Arc<sea_orm::DatabaseConnection>,
    pub owner_id: String,
}

/// Per-session lease bookkeeping: the cancel token for the heartbeat task.
#[derive(Debug)]
struct LeaseHandle {
    stop: CancellationToken,
}

#[derive(Debug)]
pub struct ExecutionRegistry<T> {
    inner: Mutex<HashMap<i64, Arc<ExecutionControl<T>>>>,
    lease: Option<LeaseConfig>,
    lease_handles: Mutex<HashMap<i64, LeaseHandle>>,
}

impl<T: Send + 'static> ExecutionRegistry<T> {
    /// A registry without a database binding: single-process semantics only
    /// (no cross-process lease). Used by tests.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            lease: None,
            lease_handles: Mutex::new(HashMap::new()),
        }
    }

    /// A registry bound to a shared database: `register` also acquires the
    /// cross-process execution lease, so two processes cannot run the same
    /// session at once.
    pub fn with_lease(db: Arc<sea_orm::DatabaseConnection>, owner_id: String) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            lease: Some(LeaseConfig { db, owner_id }),
            lease_handles: Mutex::new(HashMap::new()),
        }
    }

    /// The owner id this registry uses for cross-process leases, if bound.
    pub fn owner_id(&self) -> String {
        self.lease
            .as_ref()
            .map(|config| config.owner_id.clone())
            .unwrap_or_default()
    }

    pub async fn register(
        &self,
        session_id: i64,
        turn_id: agena_domain::TurnId,
        reply_id: agena_domain::AssistantReplyId,
    ) -> Result<(Arc<ExecutionControl<T>>, mpsc::UnboundedReceiver<Vec<T>>), ExecutionControlError>
    {
        // Local exclusivity first: this process must not run the session twice.
        {
            let guard = self.inner.lock().await;
            if guard.contains_key(&session_id) {
                return Err(ExecutionControlError::AlreadyActive(session_id));
            }
        }

        // Cross-process exclusivity: take the database lease before any
        // ExecutionStarted event is emitted, so reconcile never mistakes an
        // actively-executing session for an interrupted one.
        if let Some(LeaseConfig { db, owner_id }) = &self.lease {
            let now = agena_runtime_session_core::db::leases::lease_now_ms();
            match agena_runtime_session_core::db::leases::try_acquire_lease(
                db.as_ref(),
                session_id,
                owner_id,
                None,
                now,
            )
            .await
            .map_err(|error| ExecutionControlError::InvalidTransition(error.to_string()))?
            {
                agena_runtime_session_core::db::leases::LeaseAcquireOutcome::Acquired => {}
                agena_runtime_session_core::db::leases::LeaseAcquireOutcome::HeldBy { .. } => {
                    return Err(ExecutionControlError::AlreadyActive(session_id));
                }
            }
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let control = Arc::new(ExecutionControl::new(turn_id, reply_id, tx));
        let mut guard = self.inner.lock().await;
        guard.insert(session_id, Arc::clone(&control));
        drop(guard);

        // Refresh the lease heartbeat periodically so a reconciling process
        // sees this execution as live.
        if let Some(LeaseConfig { db, owner_id }) = &self.lease {
            let stop = CancellationToken::new();
            let stop_task = stop.clone();
            let db = Arc::clone(db);
            let owner_id = owner_id.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(LEASE_HEARTBEAT_INTERVAL);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tokio::select! {
                        _ = stop_task.cancelled() => break,
                        _ = ticker.tick() => {
                            let now = agena_runtime_session_core::db::leases::lease_now_ms();
                            let _ = agena_runtime_session_core::db::leases::heartbeat(
                                db.as_ref(), session_id, &owner_id, now,
                            ).await;
                        }
                    }
                }
            });
            self.lease_handles
                .lock()
                .await
                .insert(session_id, LeaseHandle { stop });
        }

        Ok((control, rx))
    }

    pub async fn unregister_if_matches(
        &self,
        session_id: i64,
        expected: &Arc<ExecutionControl<T>>,
    ) {
        {
            let mut guard = self.inner.lock().await;
            if let Some(current) = guard.get(&session_id)
                && Arc::ptr_eq(current, expected)
            {
                guard.remove(&session_id);
            }
        }

        // Stop the heartbeat and release the cross-process lease.
        if let Some(handle) = self.lease_handles.lock().await.remove(&session_id) {
            handle.stop.cancel();
        }
        if let Some(LeaseConfig { db, owner_id }) = &self.lease {
            let _ = agena_runtime_session_core::db::leases::release_lease(
                db.as_ref(),
                session_id,
                owner_id,
            )
            .await;
        }
    }

    pub async fn cancel_current(&self, session_id: i64) -> Result<(), ExecutionControlError> {
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

    /// Cancel exactly the execution the caller observed. A delayed request for
    /// an older execution can never affect a newer execution in the session.
    pub async fn cancel_exact(
        &self,
        session_id: i64,
        execution_id: ExecutionId,
    ) -> Result<CancellationResult, ExecutionControlError> {
        let control = match self.inner.lock().await.get(&session_id).cloned() {
            Some(control) => control,
            None => return Ok(CancellationResult::NotFound),
        };
        if control.execution_id() != execution_id {
            return Ok(CancellationResult::ExecutionMismatch);
        }
        if control.cancel.is_cancelled() {
            return Ok(CancellationResult::AlreadyTerminal);
        }
        control.transition(ExecutionPhase::Cancelling).await?;
        control.cancel.cancel();
        tokio::spawn(async move {
            tokio::time::sleep(OPERATION_CANCELLATION_GRACE).await;
            control.abort_operation().await;
        });
        Ok(CancellationResult::CancellationRequested)
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

    /// Wake the active execution that owns a canonical assistant reply after
    /// an interactive response has been durably committed.
    pub async fn signal_interaction_for_reply(
        &self,
        session_id: i64,
        reply_id: agena_domain::AssistantReplyId,
    ) -> Option<Arc<ExecutionControl<T>>> {
        let control = self.inner.lock().await.get(&session_id).cloned()?;
        if control.reply_id() != reply_id {
            return None;
        }
        control.signal_interaction();
        Some(control)
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

    use agena_domain::{CancellationResult, ExecutionLifecycle, ExecutionPhase};
    use tokio::sync::mpsc;

    use super::{ExecutionControl, ExecutionControlError, ExecutionRegistry};

    #[tokio::test]
    async fn a_owner_has_exactly_one_execution_writer() {
        let registry = ExecutionRegistry::<()>::new();
        let (first, _) = registry
            .register(
                7,
                agena_domain::TurnId::new(),
                agena_domain::AssistantReplyId::new(),
            )
            .await
            .expect("first execution");
        assert!(matches!(
            registry
                .register(
                    7,
                    agena_domain::TurnId::new(),
                    agena_domain::AssistantReplyId::new()
                )
                .await,
            Err(ExecutionControlError::AlreadyActive(7))
        ));
        registry.unregister_if_matches(7, &first).await;
        assert!(
            registry
                .register(
                    7,
                    agena_domain::TurnId::new(),
                    agena_domain::AssistantReplyId::new()
                )
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn cancellation_moves_to_cancelling_before_signalling_worker() {
        let registry = ExecutionRegistry::<()>::new();
        let (control, _) = registry
            .register(
                9,
                agena_domain::TurnId::new(),
                agena_domain::AssistantReplyId::new(),
            )
            .await
            .expect("execution");
        registry.cancel_current(9).await.expect("cancel");
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
    async fn delayed_cancel_cannot_cancel_a_newer_execution() {
        let registry = ExecutionRegistry::<()>::new();
        let turn_id = agena_domain::TurnId::new();
        let reply_id = agena_domain::AssistantReplyId::new();
        let (first, _) = registry
            .register(12, turn_id, reply_id)
            .await
            .expect("first execution");
        let old_id = first.execution_id();
        registry.unregister_if_matches(12, &first).await;

        let (second, _) = registry
            .register(12, turn_id, reply_id)
            .await
            .expect("second execution");
        assert_eq!(
            registry
                .cancel_exact(12, old_id)
                .await
                .expect("typed result"),
            CancellationResult::ExecutionMismatch
        );
        assert!(!second.cancel.is_cancelled());
        assert_eq!(
            registry
                .cancel_exact(12, second.execution_id())
                .await
                .expect("typed result"),
            CancellationResult::CancellationRequested
        );
        assert!(second.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn unregister_never_removes_a_different_execution() {
        let registry = ExecutionRegistry::<()>::new();
        let (control, _) = registry
            .register(
                11,
                agena_domain::TurnId::new(),
                agena_domain::AssistantReplyId::new(),
            )
            .await
            .expect("execution");
        let unrelated = Arc::new(ExecutionControl::new(
            agena_domain::TurnId::new(),
            agena_domain::AssistantReplyId::new(),
            mpsc::unbounded_channel().0,
        ));
        registry.unregister_if_matches(11, &unrelated).await;
        assert!(registry.is_active(11).await);
        registry.unregister_if_matches(11, &control).await;
        assert!(!registry.is_active(11).await);
    }
}
