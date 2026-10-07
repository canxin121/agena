//! Generic per-owner execution registry: exclusive ownership, cancel, and
//! steer. Concrete session code supplies its own steer payload type.
//!
//! The registry is purely in-memory execution coordination, and it is also
//! the authority on which in-flight runs are *live*: exactly one server
//! process owns a data directory, so a run marker whose session has no slot
//! here is abandoned work that recovery terminalizes.
//!
//! A slot is owned by a [`ExecutionPermit`], not by a bare `register`/`forget`
//! pair. The permit releases the slot when it drops, so a task that panics past
//! its join — or that is aborted mid-flight — cannot leave a permanent
//! occupant behind. A leaked slot would be read as "this process is still
//! running that session", which is the one answer recovery must never get
//! wrong: it would both suppress reconciliation of dead work and let a later
//! `register` fail forever with [`ExecutionControlError::AlreadyActive`].

use portable_atomic::{AtomicI64, AtomicU64};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
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
    #[error("execution was cancelled")]
    Cancelled,
    #[error("invalid execution transition: {0}")]
    InvalidTransition(String),
}

impl ExecutionControlError {
    fn invalid_transition_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::InvalidTransition(agena_failure::diagnostic::format_error_chain(error))
    }
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
        let mut lifecycle = self.lifecycle.lock().await;
        // A cancellation can arrive after a caller's last token check. Treat
        // that race as cancellation, rather than an invalid forward phase
        // transition that fails the whole reply with an internal error.
        if phase != ExecutionPhase::Cancelling
            && (self.cancel.is_cancelled()
                || matches!(
                    &*lifecycle,
                    ExecutionLifecycle::Active {
                        phase: ExecutionPhase::Cancelling,
                        ..
                    }
                ))
        {
            return Err(ExecutionControlError::Cancelled);
        }
        lifecycle
            .transition(phase)
            .map_err(|error| ExecutionControlError::invalid_transition_error(&error))
    }

    async fn request_cancel(&self) -> Result<CancellationResult, ExecutionControlError> {
        let mut lifecycle = self.lifecycle.lock().await;
        if self.cancel.is_cancelled() || matches!(&*lifecycle, ExecutionLifecycle::Terminal { .. })
        {
            return Ok(CancellationResult::AlreadyTerminal);
        }
        lifecycle
            .transition(ExecutionPhase::Cancelling)
            .map_err(|error| ExecutionControlError::invalid_transition_error(&error))?;
        // Publish the phase and token under the same lock used by transition
        // and finish. Repeated requests and completion races are idempotent.
        self.cancel.cancel();
        Ok(CancellationResult::CancellationRequested)
    }

    pub async fn finish(&self, outcome: ExecutionOutcome) -> Result<(), ExecutionControlError> {
        let mut lifecycle = self.lifecycle.lock().await;
        let outcome = if self.cancel.is_cancelled() {
            ExecutionOutcome::Cancelled
        } else {
            outcome
        };
        lifecycle
            .finish(outcome)
            .map_err(|error| ExecutionControlError::invalid_transition_error(&error))
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
    /// Shared with every [`ExecutionPermit`] so a permit released from a
    /// detached task (or a normal `Drop`) still resolves the right slot.
    inner: Arc<Mutex<HashMap<i64, Arc<ExecutionControl<T>>>>>,
    /// Delivery handshake for steered background notifications: `(session_id,
    /// notification part_id)` → the settle's one-shot. The stable-run loop
    /// fires it the moment its notification cursor observes the appended part
    /// ([`Self::ack_notification`]); the settle awaits it (or the execution's
    /// release) before concluding the wake landed, so a steer dropped at the
    /// end of a turn cannot leave the session silent.
    notification_acks: StdMutex<HashMap<(i64, i64), oneshot::Sender<()>>>,
}

#[derive(Debug)]
/// A live execution's ownership of its session's registry slot.
///
/// Holding the permit *is* being the live execution: `is_active(session_id)`
/// answers "does a permit exist", and recovery therefore never reconciles work
/// its holder is still running. Dropping it releases the slot, so the slot
/// cannot outlive the work that owns it — including when the owning task
/// panics or is aborted.
pub struct ExecutionPermit<T: Send + 'static> {
    session_id: i64,
    control: Arc<ExecutionControl<T>>,
    registry: Arc<Mutex<HashMap<i64, Arc<ExecutionControl<T>>>>>,
}

impl<T: Send + 'static> ExecutionPermit<T> {
    pub fn session_id(&self) -> i64 {
        self.session_id
    }

    pub fn control(&self) -> &Arc<ExecutionControl<T>> {
        &self.control
    }
}

impl<T: Send + 'static> Drop for ExecutionPermit<T> {
    fn drop(&mut self) {
        // `Drop` cannot await, so the release takes whichever path is
        // available without blocking a runtime thread: the synchronous fast
        // path when the lock is free, otherwise a detached task that takes the
        // async lock. Neither path can be skipped, so a permit dropped while a
        // read holds the guard still resolves its slot.
        let registry = Arc::clone(&self.registry);
        let control = Arc::clone(&self.control);
        let session_id = self.session_id;
        // The slot is keyed by an `Arc` pointer identity and the map's critical
        // sections are short, so a contended release is a same-tick race.
        if let Ok(mut slots) = registry.try_lock() {
            Self::remove_if_matches(&mut slots, session_id, &control);
            return;
        }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut slots = registry.lock().await;
                Self::remove_if_matches(&mut slots, session_id, &control);
            });
            return;
        }
        // No runtime at all (a registry used from a plain test): wait for the
        // async lock, which is free the moment the brief synchronous critical
        // section ends.
        let mut slots = registry.blocking_lock();
        Self::remove_if_matches(&mut slots, session_id, &control);
    }
}

impl<T: Send + 'static> ExecutionPermit<T> {
    fn remove_if_matches(
        slots: &mut HashMap<i64, Arc<ExecutionControl<T>>>,
        session_id: i64,
        control: &Arc<ExecutionControl<T>>,
    ) {
        if let Some(current) = slots.get(&session_id)
            && Arc::ptr_eq(current, control)
        {
            slots.remove(&session_id);
        }
    }
}

impl<T: Send + 'static> Default for ExecutionRegistry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + 'static> ExecutionRegistry<T> {
    /// A registry with no database binding: in-process execution coordination
    /// only. Cross-process session exclusivity comes from the data directory
    /// itself — exactly one server process owns it (17.2) — not from here.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            notification_acks: StdMutex::new(HashMap::new()),
        }
    }

    pub async fn register(
        &self,
        session_id: i64,
        turn_id: agena_domain::TurnId,
        reply_id: agena_domain::AssistantReplyId,
    ) -> Result<(ExecutionPermit<T>, mpsc::Receiver<Vec<T>>), ExecutionControlError> {
        self.register_with_restore(session_id, turn_id, reply_id, None, None)
            .await
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
    ) -> Result<(ExecutionPermit<T>, mpsc::Receiver<Vec<T>>), ExecutionControlError> {
        let (tx, rx) = mpsc::channel(STEER_QUEUE_CAPACITY);
        let control = Arc::new(ExecutionControl::new_with_restore(
            turn_id,
            reply_id,
            tx,
            restore_document,
            user_idempotency_key,
        ));

        // Check and insert while holding one guard. Splitting these into two
        // critical sections lets concurrent starters both observe the slot as
        // empty and then overwrite each other, leaving two live executions
        // for one session while only the newer control remains cancellable.
        // Cross-process exclusivity is structural (one process per data
        // directory), so this guard only needs to settle same-process races.
        let mut guard = self.inner.lock().await;
        if guard.contains_key(&session_id) {
            return Err(ExecutionControlError::AlreadyActive(session_id));
        }
        guard.insert(session_id, Arc::clone(&control));
        drop(guard);

        let permit = ExecutionPermit {
            session_id,
            control,
            registry: Arc::clone(&self.inner),
        };
        Ok((permit, rx))
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
        control.request_cancel().await?;
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
        control.request_cancel().await
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
            && tx.send(()).is_err()
        {
            tracing::debug!(
                session_id,
                part_id,
                "notification acknowledgement waiter was already dropped"
            );
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

    use super::{ExecutionControl, ExecutionControlError, ExecutionPermit, ExecutionRegistry};

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
        drop(first);
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
        let (permit, _) = registry
            .register(
                9,
                agena_domain::TurnId::new(),
                agena_domain::AssistantReplyId::new(),
            )
            .await
            .expect("execution");
        registry.cancel_current(9).await.expect("cancel");
        assert!(permit.control().cancel.is_cancelled());
        assert!(matches!(
            permit.control().lifecycle().await,
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
        let old_id = first.control().execution_id();
        drop(first);

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
        assert!(!second.control().cancel.is_cancelled());
        assert_eq!(
            registry
                .cancel_exact(12, second.control().execution_id())
                .await
                .expect("typed result"),
            CancellationResult::CancellationRequested
        );
        assert!(second.control().cancel.is_cancelled());
    }

    #[tokio::test]
    async fn a_dropped_permit_never_removes_a_different_execution() {
        let registry = ExecutionRegistry::<()>::new();
        let (permit, _) = registry
            .register(
                11,
                agena_domain::TurnId::new(),
                agena_domain::AssistantReplyId::new(),
            )
            .await
            .expect("execution");
        let control = Arc::clone(permit.control());
        // A stale permit for the same session — the shape a task that lost its
        // slot to a replacement would hold — must not evict the current one.
        let stale = ExecutionPermit {
            session_id: 11,
            control: Arc::new(ExecutionControl::new(
                agena_domain::TurnId::new(),
                agena_domain::AssistantReplyId::new(),
                mpsc::channel(1).0,
            )),
            registry: Arc::clone(&registry.inner),
        };
        drop(stale);
        assert!(registry.is_active(11).await);
        assert!(
            registry
                .execution_control(11, None)
                .await
                .is_some_and(|current| Arc::ptr_eq(&current, &control))
        );
        drop(permit);
        assert!(!registry.is_active(11).await);
    }

    #[tokio::test]
    async fn a_dropped_permit_releases_the_slot_for_a_replacement() {
        let registry = ExecutionRegistry::<()>::new();
        let (permit, _) = registry
            .register(
                5,
                agena_domain::TurnId::new(),
                agena_domain::AssistantReplyId::new(),
            )
            .await
            .expect("execution");
        let execution_id = permit.control().execution_id();
        assert!(registry.is_active(5).await);
        // Dropping the permit is the whole release protocol: no explicit
        // unregister call exists, so a task that panics past its join (or is
        // aborted) cannot leave the slot occupied and make recovery read dead
        // work as live.
        drop(permit);
        assert!(!registry.is_active(5).await);
        assert!(registry.execution_control(5, None).await.is_none());
        assert!(registry.cancellation_token(5).await.is_none());
        let (replacement, _) = registry
            .register(
                5,
                agena_domain::TurnId::new(),
                agena_domain::AssistantReplyId::new(),
            )
            .await
            .expect("the released slot accepts a replacement");
        assert_ne!(replacement.control().execution_id(), execution_id);
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
        let (permit, _) = registry
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
        drop(permit);
    }

    #[tokio::test]
    async fn wait_until_cancelled_released_waits_for_cancelling_execution_to_unregister() {
        let registry = Arc::new(ExecutionRegistry::<()>::new());
        let (permit, _) = registry
            .register(
                9,
                agena_domain::TurnId::new(),
                agena_domain::AssistantReplyId::new(),
            )
            .await
            .expect("execution");
        registry.cancel_current(9).await.expect("cancel");
        assert!(permit.control().cancel.is_cancelled());

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            drop(permit);
        });

        registry
            .wait_until_cancelled_released(9, Duration::from_secs(2))
            .await
            .expect("released after cancellation unregisters");
        assert!(!registry.is_active(9).await);
    }
}
