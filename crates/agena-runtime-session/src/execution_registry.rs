//! Generic per-owner execution registry: exclusive ownership, cancel, and
//! steer. Concrete session code supplies its own steer payload type.
//!
//! The registry is purely in-memory execution coordination. Cross-process
//! session exclusivity is no longer its job: in v2 the data facade owns
//! `execution_leases` and validates them on every write (design 14.2, 15.6),
//! so the manager never holds a lease outside a facade call. `register` here
//! only guarantees that one process does not run the same session twice.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    },
    time::Duration,
};

use agena_domain::{
    CancellationResult, ComposerDocument, ExecutionId, ExecutionLifecycle, ExecutionOutcome,
    ExecutionPhase,
};
use tokio::sync::{Mutex, Notify, mpsc, oneshot};
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
/// Handle to control one execution.
pub struct ExecutionControl<T> {
    execution_id: ExecutionId,
    turn_id: agena_domain::TurnId,
    reply_id: agena_domain::AssistantReplyId,
    pub cancel: CancellationToken,
    pub steer_tx: mpsc::Sender<Vec<T>>,
    lifecycle: Mutex<ExecutionLifecycle>,
    interaction_epoch: AtomicU64,
    interaction_notify: Notify,
    /// The exact document submitted by the user, retained on the execution so
    /// a cancellation can restore the editor without reconstructing it from a
    /// normalized provider prompt.
    restore_document: Option<ComposerDocument>,
    /// Retained so a cancellation that races an idempotency replay can avoid
    /// restoring a document that was never inserted by this execution.
    user_idempotency_key: Option<String>,
    /// Submission bookkeeping is separate from the run id because `0` is the
    /// sentinel for a submission that has not returned yet. A replay of an
    /// idempotency key must never make an older user marker retractable.
    user_run_id: AtomicI64,
    user_run_submitted: AtomicBool,
    user_run_created: AtomicBool,
}

const STEER_QUEUE_CAPACITY: usize = 64;

impl<T> ExecutionControl<T> {
    fn new(
        turn_id: agena_domain::TurnId,
        reply_id: agena_domain::AssistantReplyId,
        steer_tx: mpsc::Sender<Vec<T>>,
    ) -> Self {
        let execution_id = ExecutionId::new();
        Self {
            execution_id,
            turn_id,
            reply_id,
            cancel: CancellationToken::new(),
            steer_tx,
            lifecycle: Mutex::new(ExecutionLifecycle::start(execution_id)),
            interaction_epoch: AtomicU64::new(0),
            interaction_notify: Notify::new(),
            restore_document: None,
            user_idempotency_key: None,
            user_run_id: AtomicI64::new(0),
            user_run_submitted: AtomicBool::new(false),
            user_run_created: AtomicBool::new(false),
        }
    }

    fn new_with_restore(
        turn_id: agena_domain::TurnId,
        reply_id: agena_domain::AssistantReplyId,
        steer_tx: mpsc::Sender<Vec<T>>,
        restore_document: Option<ComposerDocument>,
        user_idempotency_key: Option<String>,
    ) -> Self {
        let mut control = Self::new(turn_id, reply_id, steer_tx);
        control.restore_document = restore_document;
        control.user_idempotency_key = user_idempotency_key;
        control
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

    pub fn restore_document(&self) -> Option<&ComposerDocument> {
        self.restore_document.as_ref()
    }

    pub fn user_idempotency_key(&self) -> Option<&str> {
        self.user_idempotency_key.as_deref()
    }

    /// Record the user marker only after its transaction has committed. An
    /// idempotency replay is deliberately marked as not-created, so a retry
    /// cannot withdraw a message created by an earlier execution.
    pub fn set_user_run(&self, run_id: i64, created: bool) {
        self.user_run_submitted.store(true, Ordering::Release);
        self.user_run_created.store(created, Ordering::Release);
        self.user_run_id
            .store(if created { run_id } else { 0 }, Ordering::Release);
    }

    pub fn user_run_submitted(&self) -> bool {
        self.user_run_submitted.load(Ordering::Acquire)
    }

    pub fn user_run_created(&self) -> bool {
        self.user_run_created.load(Ordering::Acquire)
    }

    pub fn user_run_id(&self) -> Option<i64> {
        let id = self.user_run_id.load(Ordering::Acquire);
        (id > 0).then_some(id)
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
}

#[derive(Debug)]
/// Registry of active executions.
pub struct ExecutionRegistry<T> {
    inner: Mutex<HashMap<i64, Arc<ExecutionControl<T>>>>,
    /// Delivery handshake for steered background notifications: `(session_id,
    /// notification part_id)` → the settle's one-shot. The stable-run loop
    /// fires it the moment its notification cursor observes the appended part
    /// ([`Self::ack_notification`]); the settle awaits it (or the execution's
    /// release) before concluding the wake landed, so a steer dropped at the
    /// end of a turn cannot leave the session silent.
    notification_acks: StdMutex<HashMap<(i64, i64), oneshot::Sender<()>>>,
}

impl<T: Send + 'static> Default for ExecutionRegistry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + 'static> ExecutionRegistry<T> {
    /// A registry with no database binding: in-process execution coordination
    /// only. Cross-process session exclusivity is enforced by the v2 facade's
    /// lease validation on every write, not here.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            notification_acks: StdMutex::new(HashMap::new()),
        }
    }

    pub async fn register(
        &self,
        session_id: i64,
        turn_id: agena_domain::TurnId,
        reply_id: agena_domain::AssistantReplyId,
    ) -> Result<(Arc<ExecutionControl<T>>, mpsc::Receiver<Vec<T>>), ExecutionControlError> {
        let (tx, rx) = mpsc::channel(STEER_QUEUE_CAPACITY);
        let control = Arc::new(ExecutionControl::new(turn_id, reply_id, tx));

        // Check and insert while holding one guard. Splitting these into two
        // critical sections lets concurrent starters both observe the slot as
        // empty and then overwrite each other, leaving two live executions
        // for one session while only the newer control remains cancellable.
        // The cross-process lease is still taken atomically by the facade on
        // the first write of the run (design 15.6 step 1).
        let mut guard = self.inner.lock().await;
        if guard.contains_key(&session_id) {
            return Err(ExecutionControlError::AlreadyActive(session_id));
        }
        guard.insert(session_id, Arc::clone(&control));
        drop(guard);

        Ok((control, rx))
    }

    /// Register an execution while retaining the original composer document
    /// for cancellation recovery.
    pub async fn register_with_restore(
        &self,
        session_id: i64,
        turn_id: agena_domain::TurnId,
        reply_id: agena_domain::AssistantReplyId,
        restore_document: Option<ComposerDocument>,
        user_idempotency_key: Option<String>,
    ) -> Result<(Arc<ExecutionControl<T>>, mpsc::Receiver<Vec<T>>), ExecutionControlError> {
        let (tx, rx) = mpsc::channel(STEER_QUEUE_CAPACITY);
        let control = Arc::new(ExecutionControl::new_with_restore(
            turn_id,
            reply_id,
            tx,
            restore_document,
            user_idempotency_key,
        ));
        let mut guard = self.inner.lock().await;
        if guard.contains_key(&session_id) {
            return Err(ExecutionControlError::AlreadyActive(session_id));
        }
        guard.insert(session_id, Arc::clone(&control));
        drop(guard);
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
    }

    /// If `session_id` is occupied by an execution whose cancel token has
    /// already tripped, wait (bounded by `timeout`) for it to unregister so
    /// the caller can register a replacement. An occupant that is NOT
    /// cancelling fails immediately with `AlreadyActive`; the same error is
    /// returned if the cancelling run has not released the session before
    /// `timeout` elapses.
    ///
    /// This closes the interrupt-and-send race: the client submits the next
    /// user turn as soon as cancellation is acknowledged, which can land
    /// before the cancelled run has finished unwinding and unregistered.
    pub async fn wait_until_cancelled_released(
        &self,
        session_id: i64,
        timeout: Duration,
    ) -> Result<(), ExecutionControlError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            {
                let guard = self.inner.lock().await;
                match guard.get(&session_id) {
                    None => return Ok(()),
                    Some(control) if control.cancel.is_cancelled() => {}
                    Some(_) => return Err(ExecutionControlError::AlreadyActive(session_id)),
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ExecutionControlError::AlreadyActive(session_id));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
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
        let steer_tx = self
            .inner
            .lock()
            .await
            .get(&session_id)
            .map(|control| control.steer_tx.clone())
            .ok_or(ExecutionControlError::NoActiveExecution(session_id))?;
        steer_tx
            .send(parts)
            .await
            .map_err(|_| ExecutionControlError::SteerClosed)
    }

    pub async fn is_active(&self, session_id: i64) -> bool {
        self.inner.lock().await.contains_key(&session_id)
    }

    /// Register the delivery handshake for a notification part the settle just
    /// appended and steered. The returned receiver resolves when the stable-run
    /// loop's notification cursor observes the part at a safe part boundary
    /// ([`Self::ack_notification`]) — the confirmation that the steer reached
    /// a live loop that will take the next provider round over it.
    pub fn register_notification_ack(
        &self,
        session_id: i64,
        part_id: i64,
    ) -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        self.notification_acks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert((session_id, part_id), tx);
        rx
    }

    /// Acknowledge that the loop's notification cursor observed `part_id` —
    /// fired by the stable-run loop for every newly-seen `system_notification`
    /// part, confirming the corresponding settle's wake steer was delivered.
    pub fn ack_notification(&self, session_id: i64, part_id: i64) {
        if let Some(tx) = self
            .notification_acks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(session_id, part_id))
        {
            let _ = tx.send(());
        }
    }

    /// Wait until `session_id` has no active execution. The notification settle
    /// uses this to distinguish "the steered execution is alive and will drain
    /// the steer" (stays pending until the loop acks) from "the execution
    /// exited without observing the notification" (returns, and the settle
    /// starts a fresh wake over the appended part). Polls like
    /// [`Self::wait_until_cancelled_released`]; this is a rare settle path.
    pub async fn wait_until_released(&self, session_id: i64) {
        loop {
            if !self.is_active(session_id).await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Wait for one exact execution to leave the registry. A replacement may
    /// be registered immediately afterwards, so waiting only for a session to
    /// become idle would be racy.
    pub async fn wait_until_execution_released(
        &self,
        session_id: i64,
        expected_execution_id: ExecutionId,
    ) {
        loop {
            let still_active = self
                .inner
                .lock()
                .await
                .get(&session_id)
                .is_some_and(|control| control.execution_id() == expected_execution_id);
            if !still_active {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Clone the active execution control, optionally requiring an exact
    /// execution id. The returned handle stays usable after unregistering.
    pub async fn execution_control(
        &self,
        session_id: i64,
        expected_execution_id: Option<ExecutionId>,
    ) -> Option<Arc<ExecutionControl<T>>> {
        let control = self.inner.lock().await.get(&session_id).cloned()?;
        if expected_execution_id.is_some_and(|expected| control.execution_id() != expected) {
            return None;
        }
        Some(control)
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
    use std::time::Duration;

    use agena_domain::{CancellationResult, ExecutionLifecycle, ExecutionPhase};
    use tokio::sync::{Barrier, mpsc};

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

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn concurrent_registration_has_one_winner() {
        const STARTERS: usize = 32;
        let registry = Arc::new(ExecutionRegistry::<()>::new());
        let barrier = Arc::new(Barrier::new(STARTERS));
        let mut tasks = Vec::with_capacity(STARTERS);
        for _ in 0..STARTERS {
            let registry = Arc::clone(&registry);
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                registry
                    .register(
                        42,
                        agena_domain::TurnId::new(),
                        agena_domain::AssistantReplyId::new(),
                    )
                    .await
            }));
        }

        let mut winners = 0;
        let mut already_active = 0;
        for task in tasks {
            match task.await.expect("registration task") {
                Ok(_) => winners += 1,
                Err(ExecutionControlError::AlreadyActive(42)) => already_active += 1,
                Err(error) => panic!("unexpected registration error: {error}"),
            }
        }
        assert_eq!(winners, 1);
        assert_eq!(already_active, STARTERS - 1);
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
            mpsc::channel(1).0,
        ));
        registry.unregister_if_matches(11, &unrelated).await;
        assert!(registry.is_active(11).await);
        registry.unregister_if_matches(11, &control).await;
        assert!(!registry.is_active(11).await);
    }

    #[tokio::test]
    async fn wait_until_cancelled_released_returns_ok_without_an_execution() {
        let registry = ExecutionRegistry::<()>::new();
        registry
            .wait_until_cancelled_released(99, Duration::from_secs(1))
            .await
            .expect("no execution is already released");
    }

    #[tokio::test]
    async fn wait_until_cancelled_released_fails_while_execution_active() {
        let registry = ExecutionRegistry::<()>::new();
        let (control, _) = registry
            .register(
                7,
                agena_domain::TurnId::new(),
                agena_domain::AssistantReplyId::new(),
            )
            .await
            .expect("execution");
        assert!(matches!(
            registry
                .wait_until_cancelled_released(7, Duration::from_millis(50))
                .await,
            Err(ExecutionControlError::AlreadyActive(7))
        ));
        registry.unregister_if_matches(7, &control).await;
    }

    #[tokio::test]
    async fn wait_until_cancelled_released_waits_for_cancelling_execution_to_unregister() {
        let registry = Arc::new(ExecutionRegistry::<()>::new());
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

        let worker_registry = Arc::clone(&registry);
        let worker_control = Arc::clone(&control);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            worker_registry
                .unregister_if_matches(9, &worker_control)
                .await;
        });

        registry
            .wait_until_cancelled_released(9, Duration::from_secs(2))
            .await
            .expect("released after cancellation unregisters");
        assert!(!registry.is_active(9).await);
    }
}
