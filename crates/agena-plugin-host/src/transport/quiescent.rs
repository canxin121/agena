//! Quiescent lifecycle wrapper for every plugin transport.
//!
//! Graceful shutdown first closes admission, then waits for every accepted
//! dispatch/notification/stream to settle, and only then runs the plugin's
//! shutdown hook and closes the underlying transport. This is the host-side
//! equivalent of an effect scope reaching quiescence rather than merely
//! requesting cancellation. Forced close reaches the underlying transport
//! first so a pending request cannot prevent process cleanup.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;

use super::{PluginTransport, ToolStreamHandle};
use crate::error::TransportError;
use crate::sdk::host_api::HostClient;
use crate::sdk::rpc::method;
use crate::sdk::{PluginError, ToolInvokeInput};

struct ActivityState {
    accepting: AtomicBool,
    active: AtomicUsize,
    idle: Notify,
    closing: CancellationToken,
    finish: Mutex<FinishState>,
}

#[derive(Default)]
struct FinishState {
    outcome: Option<Result<(), TransportError>>,
}

impl ActivityState {
    fn new() -> Self {
        Self {
            accepting: AtomicBool::new(true),
            active: AtomicUsize::new(0),
            idle: Notify::new(),
            closing: CancellationToken::new(),
            finish: Mutex::new(FinishState::default()),
        }
    }

    fn enter(self: &Arc<Self>) -> Result<ActivityGuard, TransportError> {
        if !self.accepting.load(Ordering::Acquire) {
            return Err(TransportError::disconnected(
                "plugin transport is no longer accepting new calls",
            ));
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        // Close the race where shutdown flips admission after the first check
        // but before this call becomes visible in the active count.
        if !self.accepting.load(Ordering::Acquire) {
            if self.active.fetch_sub(1, Ordering::AcqRel) == 1 {
                self.idle.notify_waiters();
            }
            return Err(TransportError::disconnected(
                "plugin transport began shutting down while the call was entering",
            ));
        }
        Ok(ActivityGuard {
            state: Arc::clone(self),
        })
    }

    async fn wait_idle(&self) {
        loop {
            let notified = self.idle.notified();
            if self.active.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }
}

struct ActivityGuard {
    state: Arc<ActivityState>,
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        if self.state.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.state.idle.notify_waiters();
        }
    }
}

/// Admission- and quiescence-aware wrapper used for every loaded transport.
pub struct QuiescentTransport {
    inner: Arc<dyn PluginTransport>,
    state: Arc<ActivityState>,
}

impl QuiescentTransport {
    pub fn new(inner: Arc<dyn PluginTransport>) -> Self {
        Self {
            inner,
            state: Arc::new(ActivityState::new()),
        }
    }

    pub fn wrap(inner: Arc<dyn PluginTransport>) -> Arc<dyn PluginTransport> {
        Arc::new(Self::new(inner))
    }

    pub fn active_calls(&self) -> usize {
        self.state.active.load(Ordering::Acquire)
    }

    pub fn is_accepting(&self) -> bool {
        self.state.accepting.load(Ordering::Acquire)
    }

    async fn finish(&self, graceful: bool) -> Result<(), TransportError> {
        let mut finish = self.state.finish.lock().await;
        if let Some(outcome) = &finish.outcome {
            self.state.wait_idle().await;
            return outcome.clone();
        }
        self.state.accepting.store(false, Ordering::Release);
        self.state.closing.cancel();
        if graceful {
            self.state.wait_idle().await;
            // The shutdown hook runs outside admission tracking after all
            // ordinary calls settle, so it cannot race with a new invocation.
            let _ = self
                .inner
                .dispatch(
                    method::META_SHUTDOWN,
                    serde_json::Value::Object(Default::default()),
                )
                .await;
        }
        let result = self.inner.close().await;
        // Cache the terminal transport outcome before awaiting wrappers. A
        // cancelled waiter must not erase a cleanup failure or rerun close.
        finish.outcome = Some(result.clone());
        self.state.wait_idle().await;
        result
    }
}

#[async_trait]
impl PluginTransport for QuiescentTransport {
    async fn initialize(
        &self,
        initialization: super::initialization::PluginInitialization,
    ) -> Result<crate::sdk::InitOutcome, TransportError> {
        let _activity = self.state.enter()?;
        self.inner.initialize(initialization).await
    }

    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let _activity = self.state.enter()?;
        self.inner.dispatch(method, params).await
    }

    async fn notify(&self, method: &str, params: serde_json::Value) -> Result<(), TransportError> {
        let _activity = self.state.enter()?;
        self.inner.notify(method, params).await
    }

    async fn attach_host(&self, host: Arc<dyn HostClient>) -> Result<(), TransportError> {
        let _activity = self.state.enter()?;
        self.inner.attach_host(host).await
    }

    async fn bind_host(
        &self,
        host: Arc<crate::host::HostHandle>,
        scope: Arc<crate::effect_scope::PluginEffectScope>,
    ) -> Result<(), TransportError> {
        let _activity = self.state.enter()?;
        self.inner.bind_host(host, scope).await
    }

    async fn can_reuse(&self, owner: &Arc<crate::effect_scope::PluginEffectScope>) -> bool {
        self.is_accepting() && self.inner.can_reuse(owner).await
    }

    async fn try_rebind_host(
        &self,
        host: Arc<crate::host::HostHandle>,
        scope: Arc<crate::effect_scope::PluginEffectScope>,
        previous_scope: Arc<crate::effect_scope::PluginEffectScope>,
        initialization: super::initialization::PluginInitialization,
    ) -> Result<bool, TransportError> {
        let Ok(_activity) = self.state.enter() else {
            return Ok(false);
        };
        self.inner
            .try_rebind_host(host, scope, previous_scope, initialization)
            .await
    }

    async fn invoke_stream(
        &self,
        input: ToolInvokeInput,
    ) -> Result<Option<ToolStreamHandle>, TransportError> {
        let activity = self.state.enter()?;
        let Some(mut inner) = self.inner.invoke_stream(input).await? else {
            return Ok(None);
        };
        let stream_id = inner.stream_id.clone();
        let (chunk_tx, chunk_rx) = tokio::sync::mpsc::channel(32);
        let (end_tx, end_rx) = tokio::sync::oneshot::channel();
        let closing = self.state.closing.clone();
        let stream_id_for_task = stream_id.clone();
        tokio::spawn(async move {
            let _activity = activity;
            let terminal = |result: Result<_, tokio::sync::oneshot::error::RecvError>| {
                result.unwrap_or_else(|error| {
                    Err(PluginError::internal(format!(
                        "plugin stream ended without a terminal result: {error}"
                    )))
                })
            };
            let mut chunks_open = true;
            let mut forward_chunks = true;
            let mut outcome = None;
            let result = loop {
                if !chunks_open {
                    break match outcome {
                        Some(result) => result,
                        None => terminal(inner.end.await),
                    };
                }
                tokio::select! {
                    biased;
                    // Completion seals ingress, then drains every accepted
                    // record. A ready terminal must never overtake buffered
                    // output, even before the initial HTTP response arrives.
                    result = &mut inner.end, if outcome.is_none() => {
                        outcome = Some(terminal(result));
                        inner.chunks.close();
                    }
                    chunk = inner.chunks.recv(), if chunks_open => {
                        let Some(chunk) = chunk else {
                            chunks_open = false;
                            continue;
                        };
                        // Retain the activity until the actual terminal result,
                        // even when the caller has dropped its chunk receiver.
                        if !forward_chunks || closing.is_cancelled() {
                            continue;
                        }
                        loop {
                            // Errors must release the terminal waiter even
                            // when it never consumes output. Preserve what
                            // fits; the error describes the incomplete tail.
                            if outcome.as_ref().is_some_and(Result::is_err) {
                                if chunk_tx.try_send(chunk).is_err() {
                                    forward_chunks = false;
                                }
                                break;
                            }
                            tokio::select! {
                                biased;
                                result = &mut inner.end, if outcome.is_none() => {
                                    outcome = Some(terminal(result));
                                    inner.chunks.close();
                                }
                                _ = closing.cancelled() => break,
                                result = chunk_tx.reserve() => {
                                    match result {
                                        Ok(permit) => { permit.send(chunk); }
                                        Err(_) => forward_chunks = false,
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            };
            drop(chunk_tx);
            if end_tx.send(result).is_err() {
                tracing::debug!(
                    stream_id = %stream_id_for_task,
                    "quiescent plugin stream terminal-result receiver was dropped"
                );
            }
        });
        Ok(Some(ToolStreamHandle {
            stream_id,
            chunks: chunk_rx,
            end: end_rx,
        }))
    }

    async fn ingest_stream_event(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<bool, TransportError> {
        let _activity = self.state.enter()?;
        self.inner.ingest_stream_event(method, params).await
    }

    async fn shutdown(&self) -> Result<(), TransportError> {
        self.finish(true).await
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.finish(false).await
    }
}
