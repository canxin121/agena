//! Per-session run control: cancel + steer.
//!
//! Each in-flight run registers a `RunControl` with its session id; the
//! control owns a `CancellationToken` (so external callers can cancel the
//! run) and a `mpsc::UnboundedSender<Vec<PartContent>>` (so external
//! callers can inject "steer" messages that the run loop will see at the
//! next inter-run boundary).
//!
//! This is intentionally an in-process structure — when the API server
//! lives on a different host this gets fronted by a remote-control RPC.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::message::PartContent;

/// Errors surfaced by `cancel_active_run` / `steer_input`.
#[derive(Debug, thiserror::Error)]
pub enum RunControlError {
    #[error("no in-flight run for session {0}")]
    NoActiveRun(i64),
    #[error("run no longer accepts steer input (channel closed)")]
    SteerClosed,
}

#[derive(Debug)]
pub struct RunControl {
    pub cancel: CancellationToken,
    pub steer_tx: mpsc::UnboundedSender<Vec<PartContent>>,
    superseded: AtomicBool,
}

impl RunControl {
    fn new(steer_tx: mpsc::UnboundedSender<Vec<PartContent>>) -> Self {
        Self {
            cancel: CancellationToken::new(),
            steer_tx,
            superseded: AtomicBool::new(false),
        }
    }

    fn mark_superseded(&self) {
        self.superseded.store(true, Ordering::SeqCst);
    }

    pub fn is_superseded(&self) -> bool {
        self.superseded.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Default)]
pub struct RunRegistry {
    inner: Mutex<HashMap<i64, Arc<RunControl>>>,
}

impl RunRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a new control for `session_id`, replacing any prior entry
    /// (the prior entry is signalled cancel so its task winds down).
    /// Returns the registered control along with the steer receiver the
    /// run task should poll between rounds.
    pub async fn register(
        &self,
        session_id: i64,
    ) -> (Arc<RunControl>, mpsc::UnboundedReceiver<Vec<PartContent>>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let control = Arc::new(RunControl::new(tx));
        let mut guard = self.inner.lock().await;
        if let Some(prev) = guard.insert(session_id, Arc::clone(&control)) {
            prev.mark_superseded();
            prev.cancel.cancel();
        }
        (control, rx)
    }

    /// Allocate a new control for `session_id` only when there is no current
    /// in-flight run. Returns `None` instead of cancelling a newer run.
    #[allow(dead_code)]
    pub async fn try_register_if_inactive(
        &self,
        session_id: i64,
    ) -> Option<(Arc<RunControl>, mpsc::UnboundedReceiver<Vec<PartContent>>)> {
        let (tx, rx) = mpsc::unbounded_channel();
        let control = Arc::new(RunControl::new(tx));
        let mut guard = self.inner.lock().await;
        if guard.contains_key(&session_id) {
            return None;
        }
        guard.insert(session_id, Arc::clone(&control));
        Some((control, rx))
    }

    /// Remove the control for `session_id` if it still matches `expected`.
    /// Used by the run task on completion so a parallel `register` (e.g.
    /// re-entry) doesn't get clobbered.
    pub async fn unregister_if_matches(&self, session_id: i64, expected: &Arc<RunControl>) {
        let mut guard = self.inner.lock().await;
        if let Some(current) = guard.get(&session_id)
            && Arc::ptr_eq(current, expected)
        {
            guard.remove(&session_id);
        }
    }

    pub async fn cancel(&self, session_id: i64) -> Result<(), RunControlError> {
        let guard = self.inner.lock().await;
        let control = guard
            .get(&session_id)
            .ok_or(RunControlError::NoActiveRun(session_id))?;
        control.cancel.cancel();
        Ok(())
    }

    pub async fn steer(
        &self,
        session_id: i64,
        parts: Vec<PartContent>,
    ) -> Result<(), RunControlError> {
        let guard = self.inner.lock().await;
        let control = guard
            .get(&session_id)
            .ok_or(RunControlError::NoActiveRun(session_id))?;
        control
            .steer_tx
            .send(parts)
            .map_err(|_| RunControlError::SteerClosed)
    }

    pub async fn is_active(&self, session_id: i64) -> bool {
        self.inner.lock().await.contains_key(&session_id)
    }

    pub async fn active_session_ids(&self) -> Vec<i64> {
        self.inner.lock().await.keys().copied().collect()
    }
}
