//! Per-session turn control: cancel + steer.
//!
//! Each in-flight turn registers a `TurnControl` with its session id; the
//! control owns a `CancellationToken` (so external callers can cancel the
//! turn) and a `mpsc::UnboundedSender<Vec<PartContent>>` (so external
//! callers can inject "steer" messages that the turn loop will see at the
//! next inter-turn boundary).
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

/// Errors surfaced by `cancel_active_turn` / `steer_input`.
#[derive(Debug, thiserror::Error)]
pub enum TurnControlError {
    #[error("no in-flight turn for session {0}")]
    NoActiveTurn(i64),
    #[error("turn no longer accepts steer input (channel closed)")]
    SteerClosed,
}

#[derive(Debug)]
pub struct TurnControl {
    pub cancel: CancellationToken,
    pub steer_tx: mpsc::UnboundedSender<Vec<PartContent>>,
    superseded: AtomicBool,
}

impl TurnControl {
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
pub struct TurnRegistry {
    inner: Mutex<HashMap<i64, Arc<TurnControl>>>,
}

impl TurnRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a new control for `session_id`, replacing any prior entry
    /// (the prior entry is signalled cancel so its task winds down).
    /// Returns the registered control along with the steer receiver the
    /// turn task should poll between rounds.
    pub async fn register(
        &self,
        session_id: i64,
    ) -> (Arc<TurnControl>, mpsc::UnboundedReceiver<Vec<PartContent>>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let control = Arc::new(TurnControl::new(tx));
        let mut guard = self.inner.lock().await;
        if let Some(prev) = guard.insert(session_id, Arc::clone(&control)) {
            prev.mark_superseded();
            prev.cancel.cancel();
        }
        (control, rx)
    }

    /// Allocate a new control for `session_id` only when there is no current
    /// in-flight turn. Returns `None` instead of cancelling a newer turn.
    pub async fn try_register_if_inactive(
        &self,
        session_id: i64,
    ) -> Option<(Arc<TurnControl>, mpsc::UnboundedReceiver<Vec<PartContent>>)> {
        let (tx, rx) = mpsc::unbounded_channel();
        let control = Arc::new(TurnControl::new(tx));
        let mut guard = self.inner.lock().await;
        if guard.contains_key(&session_id) {
            return None;
        }
        guard.insert(session_id, Arc::clone(&control));
        Some((control, rx))
    }

    /// Remove the control for `session_id` if it still matches `expected`.
    /// Used by the turn task on completion so a parallel `register` (e.g.
    /// re-entry) doesn't get clobbered.
    pub async fn unregister_if_matches(&self, session_id: i64, expected: &Arc<TurnControl>) {
        let mut guard = self.inner.lock().await;
        if let Some(current) = guard.get(&session_id)
            && Arc::ptr_eq(current, expected)
        {
            guard.remove(&session_id);
        }
    }

    pub async fn cancel(&self, session_id: i64) -> Result<(), TurnControlError> {
        let guard = self.inner.lock().await;
        let control = guard
            .get(&session_id)
            .ok_or(TurnControlError::NoActiveTurn(session_id))?;
        control.cancel.cancel();
        Ok(())
    }

    pub async fn steer(
        &self,
        session_id: i64,
        parts: Vec<PartContent>,
    ) -> Result<(), TurnControlError> {
        let guard = self.inner.lock().await;
        let control = guard
            .get(&session_id)
            .ok_or(TurnControlError::NoActiveTurn(session_id))?;
        control
            .steer_tx
            .send(parts)
            .map_err(|_| TurnControlError::SteerClosed)
    }

    pub async fn is_active(&self, session_id: i64) -> bool {
        self.inner.lock().await.contains_key(&session_id)
    }

    pub async fn active_session_ids(&self) -> Vec<i64> {
        self.inner.lock().await.keys().copied().collect()
    }
}
