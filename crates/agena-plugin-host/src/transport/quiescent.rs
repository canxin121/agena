//! Quiescent lifecycle wrapper for every plugin transport.
//!
//! Graceful shutdown first closes admission, then waits for every accepted
//! dispatch/notification/stream to settle, and only then runs the plugin's
//! shutdown hook and closes the underlying transport. This is the host-side
//! equivalent of an effect scope reaching quiescence rather than merely
//! requesting cancellation. Forced close cancels accepted work before closing
//! the underlying transport, even while graceful shutdown is waiting for idle.

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
    forced_stop: CancellationToken,
    finish: Mutex<FinishState>,
}

#[derive(Default)]
struct FinishState {
    outcome: Option<FinishOutcome>,
}

struct FinishOutcome {
    shutdown: Result<(), TransportError>,
    close: Result<(), TransportError>,
}

impl ActivityState {
    fn new() -> Self {
        Self {
            accepting: AtomicBool::new(true),
            active: AtomicUsize::new(0),
            idle: Notify::new(),
            closing: CancellationToken::new(),
            forced_stop: CancellationToken::new(),
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

    async fn run<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, TransportError>>,
    ) -> Result<T, TransportError> {
        tokio::select! {
            biased;
            _ = self.forced_stop.cancelled() => Err(TransportError::Cancelled),
            result = future => result,
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
        if !graceful {
            // A graceful waiter may own `finish` while awaiting these calls.
            // Signal them before taking that lock so forced close can release
            // the waiter and reach the underlying transport cleanup.
            self.state.accepting.store(false, Ordering::Release);
            self.state.closing.cancel();
            self.state.forced_stop.cancel();
        }
        let mut finish = self.state.finish.lock().await;
        if let Some(outcome) = &finish.outcome {
            self.state.wait_idle().await;
            return if graceful {
                outcome.shutdown.clone()
            } else {
                outcome.close.clone()
            };
        }
        self.state.accepting.store(false, Ordering::Release);
        self.state.closing.cancel();
        let shutdown_error = if graceful {
            self.state.wait_idle().await;
            // The shutdown hook runs outside admission tracking after all
            // ordinary calls settle, so it cannot race with a new invocation.
            tokio::select! {
                biased;
                // Forced close deliberately interrupts an unfinished hook.
                // Preserve its existing cancellation semantics without hiding
                // an error actually returned by a completed shutdown hook.
                _ = self.state.forced_stop.cancelled() => None,
                result = self.inner.dispatch(
                    method::META_SHUTDOWN,
                    serde_json::Value::Object(Default::default()),
                ) => result.err(),
            }
        } else {
            None
        };
        // A rejected shutdown hook still requires transport cleanup. Keep the
        // two outcomes distinct: loader fallback close must not report a past
        // hook error as a new failure to close an already-closed transport.
        let close = self.inner.close().await;
        let result = match (shutdown_error, close.clone()) {
            (None, close) => close,
            (Some(error), Ok(())) => Err(error),
            (Some(error), Err(close_error)) => Err(TransportError::Io(format!(
                "{error}; transport close also failed: {close_error}"
            ))),
        };
        // Cache the terminal transport outcome before awaiting wrappers. A
        // cancelled waiter must not erase a cleanup failure or rerun close.
        finish.outcome = Some(FinishOutcome {
            shutdown: result.clone(),
            close,
        });
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
        self.state.run(self.inner.initialize(initialization)).await
    }

    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let _activity = self.state.enter()?;
        self.state.run(self.inner.dispatch(method, params)).await
    }

    async fn notify(&self, method: &str, params: serde_json::Value) -> Result<(), TransportError> {
        let _activity = self.state.enter()?;
        self.state.run(self.inner.notify(method, params)).await
    }

    async fn attach_host(&self, host: Arc<dyn HostClient>) -> Result<(), TransportError> {
        let _activity = self.state.enter()?;
        self.state.run(self.inner.attach_host(host)).await
    }

    async fn bind_host(
        &self,
        host: Arc<crate::host::HostHandle>,
        scope: Arc<crate::effect_scope::PluginEffectScope>,
    ) -> Result<(), TransportError> {
        let _activity = self.state.enter()?;
        self.state.run(self.inner.bind_host(host, scope)).await
    }

    async fn can_reuse(&self, owner: &Arc<crate::effect_scope::PluginEffectScope>) -> bool {
        self.is_accepting()
            && self
                .state
                .run(async { Ok(self.inner.can_reuse(owner).await) })
                .await
                .unwrap_or(false)
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
        self.state
            .run(
                self.inner
                    .try_rebind_host(host, scope, previous_scope, initialization),
            )
            .await
    }

    async fn fork_host(
        &self,
        host: Arc<crate::host::HostHandle>,
        scope: Arc<crate::effect_scope::PluginEffectScope>,
        previous_scope: Arc<crate::effect_scope::PluginEffectScope>,
        initialization: super::initialization::PluginInitialization,
    ) -> Result<Option<Arc<dyn PluginTransport>>, TransportError> {
        let _activity = self.state.enter()?;
        self.state
            .run(
                self.inner
                    .fork_host(host, scope, previous_scope, initialization),
            )
            .await
            .map(|transport| transport.map(Self::wrap))
    }

    fn abandon_host_fork(&self) {
        self.state.accepting.store(false, Ordering::Release);
        self.state.closing.cancel();
        self.inner.abandon_host_fork();
    }

    async fn invoke_stream(
        &self,
        input: ToolInvokeInput,
    ) -> Result<Option<ToolStreamHandle>, TransportError> {
        let activity = self.state.enter()?;
        let Some(mut inner) = self.state.run(self.inner.invoke_stream(input)).await? else {
            return Ok(None);
        };
        let stream_id = inner.stream_id.clone();
        let (chunk_tx, chunk_rx) = tokio::sync::mpsc::channel(32);
        let (mut end_tx, end_rx) = tokio::sync::oneshot::channel();
        let closing = self.state.closing.clone();
        let forced_stop = self.state.forced_stop.clone();
        let stream_id_for_task = stream_id.clone();
        tokio::spawn(async move {
            let _activity = activity;
            let cancelled = || {
                PluginError::from_kind(
                    crate::sdk::PluginErrorKind::Cancelled,
                    "plugin transport was forcibly closed",
                )
            };
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
            let result = 'relay: loop {
                if !chunks_open {
                    break match outcome {
                        Some(result) => result,
                        None => tokio::select! {
                            biased;
                            _ = forced_stop.cancelled() => Err(cancelled()),
                            _ = end_tx.closed() => return,
                            result = &mut inner.end => terminal(result),
                        },
                    };
                }
                tokio::select! {
                    biased;
                    _ = forced_stop.cancelled() => break Err(cancelled()),
                    // The request's transport lifetime owns cancellation.
                    // Releasing the inner handle must not be hidden by this
                    // forwarding task after its caller has gone away.
                    _ = end_tx.closed() => return,
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
                                _ = forced_stop.cancelled() => break 'relay Err(cancelled()),
                                _ = end_tx.closed() => return,
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
            // Dropping the original terminal receiver drives cancellation in
            // each concrete transport before reporting this wrapper's result.
            drop(inner);
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
        self.state
            .run(self.inner.ingest_stream_event(method, params))
            .await
    }

    async fn shutdown(&self) -> Result<(), TransportError> {
        self.finish(true).await
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.finish(false).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use serde_json::Value;

    use super::{PluginTransport, QuiescentTransport, TransportError};
    use crate::sdk::{Plugin, PluginError, PluginManifest};
    use crate::transport::inproc::InProcessTransport;

    struct ShutdownProbe {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    #[async_trait]
    impl Plugin for ShutdownProbe {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::new("test", "shutdown-propagation", "test")
        }

        async fn shutdown(&self) -> crate::sdk::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(PluginError::not_implemented("deliberate shutdown failure"))
            } else {
                Ok(())
            }
        }
    }

    /// Retain the real SDK dispatcher/in-process error conversion while making
    /// the underlying close operation observable and independently fallible.
    struct CloseProbe {
        inner: InProcessTransport<ShutdownProbe>,
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    #[async_trait]
    impl PluginTransport for CloseProbe {
        async fn dispatch(&self, method: &str, params: Value) -> Result<Value, TransportError> {
            self.inner.dispatch(method, params).await
        }

        async fn close(&self) -> Result<(), TransportError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.close().await?;
            if self.fail {
                Err(TransportError::Io("deliberate close failure".into()))
            } else {
                Ok(())
            }
        }
    }

    struct Probe {
        transport: Arc<QuiescentTransport>,
        shutdown_calls: Arc<AtomicUsize>,
        close_calls: Arc<AtomicUsize>,
    }

    impl Probe {
        fn new(fail_shutdown: bool, fail_close: bool) -> Self {
            let shutdown_calls = Arc::new(AtomicUsize::new(0));
            let close_calls = Arc::new(AtomicUsize::new(0));
            let transport = Arc::new(QuiescentTransport::new(Arc::new(CloseProbe {
                inner: InProcessTransport::new(ShutdownProbe {
                    calls: shutdown_calls.clone(),
                    fail: fail_shutdown,
                }),
                calls: close_calls.clone(),
                fail: fail_close,
            })));
            Self {
                transport,
                shutdown_calls,
                close_calls,
            }
        }

        fn assert_called_once(&self) {
            assert_eq!(self.shutdown_calls.load(Ordering::SeqCst), 1);
            assert_eq!(self.close_calls.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn failed_sdk_shutdown_reaches_loader_and_does_not_repeat_close() {
        let probe = Probe::new(true, false);
        let first = crate::loader::shutdown_transport(probe.transport.clone())
            .await
            .unwrap_err();
        assert!(matches!(&first, TransportError::Plugin(_)), "{first}");
        assert!(first.to_string().contains("deliberate shutdown failure"));
        assert!(
            !first
                .to_string()
                .contains("forced transport cleanup also failed")
        );
        let repeated = crate::loader::shutdown_transport(probe.transport.clone())
            .await
            .unwrap_err();
        assert_eq!(repeated.to_string(), first.to_string());
        // The loader's fallback and later explicit close only observe cleanup;
        // neither may rerun the failed plugin hook or invent a close failure.
        probe.transport.close().await.unwrap();
        probe.assert_called_once();
    }

    #[tokio::test]
    async fn shutdown_and_close_errors_are_both_retained_for_later_waiters() {
        let probe = Probe::new(true, true);
        let first = probe.transport.shutdown().await.unwrap_err().to_string();
        assert!(first.contains("deliberate shutdown failure"), "{first}");
        assert!(first.contains("deliberate close failure"), "{first}");
        assert_eq!(
            probe.transport.shutdown().await.unwrap_err().to_string(),
            first
        );
        let close = probe.transport.close().await.unwrap_err().to_string();
        assert!(close.contains("deliberate close failure"), "{close}");
        assert!(!close.contains("deliberate shutdown failure"), "{close}");
        probe.assert_called_once();
    }

    #[tokio::test]
    async fn successful_hook_does_not_hide_a_failed_transport_close() {
        let probe = Probe::new(false, true);
        let first = probe.transport.shutdown().await.unwrap_err().to_string();
        assert!(first.contains("deliberate close failure"), "{first}");
        assert_eq!(
            probe.transport.shutdown().await.unwrap_err().to_string(),
            first
        );
        assert_eq!(
            probe.transport.close().await.unwrap_err().to_string(),
            first
        );
        probe.assert_called_once();
    }
}
