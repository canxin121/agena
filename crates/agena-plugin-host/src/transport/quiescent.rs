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
            let forward = async move {
                let mut forward_chunks = true;
                while let Some(chunk) = inner.chunks.recv().await {
                    // A dropped consumer still drains until the transport
                    // reports completion. Shutdown removes client backpressure.
                    if !forward_chunks || closing.is_cancelled() {
                        continue;
                    }
                    tokio::select! {
                        biased;
                        _ = closing.cancelled() => {}
                        result = chunk_tx.send(chunk) => {
                            if result.is_err() {
                                forward_chunks = false;
                            }
                        }
                    }
                }
            };
            tokio::pin!(forward);
            let terminal = |result: Result<_, tokio::sync::oneshot::error::RecvError>| {
                result.unwrap_or_else(|error| {
                    Err(PluginError::internal(format!(
                        "plugin stream ended without a terminal result: {error}"
                    )))
                })
            };
            let result = tokio::select! {
                biased;
                result = &mut inner.end => {
                    let result = terminal(result);
                    // A terminal failure (including handoff cancellation)
                    // must reach the caller even if its chunk receiver is full.
                    // Successful completion still delivers all queued chunks.
                    if result.is_ok() { forward.await; }
                    result
                }
                () = &mut forward => terminal(inner.end.await),
            };
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::sdk::{ToolStreamChunk, ToolStreamEnd};

    struct BlockingTransport {
        entered: Arc<Notify>,
        release: Arc<Notify>,
        shutdowns: Arc<AtomicUsize>,
        closes: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl PluginTransport for BlockingTransport {
        async fn dispatch(
            &self,
            method: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, TransportError> {
            if method == method::META_SHUTDOWN {
                self.shutdowns.fetch_add(1, Ordering::AcqRel);
                return Ok(serde_json::Value::Null);
            }
            self.entered.notify_waiters();
            self.release.notified().await;
            Ok(serde_json::json!({"ok": true}))
        }

        async fn close(&self) -> Result<(), TransportError> {
            self.closes.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }

    #[tokio::test]
    async fn shutdown_waits_for_accepted_calls_and_rejects_new_calls() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let closes = Arc::new(AtomicUsize::new(0));
        let transport = Arc::new(QuiescentTransport::new(Arc::new(BlockingTransport {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
            shutdowns: Arc::clone(&shutdowns),
            closes: Arc::clone(&closes),
        })));

        let running = {
            let transport = Arc::clone(&transport);
            tokio::spawn(async move {
                transport
                    .dispatch("test/run", serde_json::Value::Null)
                    .await
            })
        };
        entered.notified().await;
        assert_eq!(transport.active_calls(), 1);

        let stopping = {
            let transport = Arc::clone(&transport);
            tokio::spawn(async move { transport.shutdown().await })
        };
        while transport.is_accepting() {
            tokio::task::yield_now().await;
        }
        assert!(matches!(
            transport
                .dispatch("test/late", serde_json::Value::Null)
                .await,
            Err(TransportError::Disconnected(_))
        ));
        assert_eq!(shutdowns.load(Ordering::Acquire), 0);
        assert_eq!(closes.load(Ordering::Acquire), 0);

        release.notify_waiters();
        running
            .await
            .expect("running task joins")
            .expect("call succeeds");
        stopping
            .await
            .expect("shutdown task joins")
            .expect("shutdown succeeds");
        assert_eq!(shutdowns.load(Ordering::Acquire), 1);
        assert_eq!(closes.load(Ordering::Acquire), 1);
        assert_eq!(transport.active_calls(), 0);
    }

    #[tokio::test]
    async fn close_is_idempotent() {
        let transport = QuiescentTransport::new(Arc::new(BlockingTransport {
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            shutdowns: Arc::new(AtomicUsize::new(0)),
            closes: Arc::new(AtomicUsize::new(0)),
        }));
        transport.close().await.expect("first close");
        transport.close().await.expect("second close");
        assert!(!transport.is_accepting());
    }

    struct ClosingTransport {
        entered: Notify,
        closed: CancellationToken,
        fail_close: bool,
        close_count: AtomicUsize,
    }

    #[async_trait]
    impl PluginTransport for ClosingTransport {
        async fn dispatch(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, TransportError> {
            self.entered.notify_one();
            self.closed.cancelled().await;
            Err(TransportError::disconnected("closed"))
        }

        async fn close(&self) -> Result<(), TransportError> {
            self.close_count.fetch_add(1, Ordering::AcqRel);
            self.closed.cancel();
            if self.fail_close {
                Err(TransportError::Io("injected cleanup failure".into()))
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn forced_close_reaches_the_transport_while_requests_are_pending() {
        let inner = Arc::new(ClosingTransport {
            entered: Notify::new(),
            closed: CancellationToken::new(),
            fail_close: false,
            close_count: AtomicUsize::new(0),
        });
        let transport = Arc::new(QuiescentTransport::new(inner.clone()));
        let request = tokio::spawn({
            let transport = transport.clone();
            async move {
                transport
                    .dispatch("test/wait", serde_json::Value::Null)
                    .await
            }
        });
        inner.entered.notified().await;
        let close =
            tokio::time::timeout(std::time::Duration::from_millis(100), transport.close()).await;
        inner.closed.cancel();
        assert!(request.await.unwrap().is_err());
        close
            .expect("force close must interrupt the underlying pending request")
            .unwrap();
        assert_eq!(inner.close_count.load(Ordering::Acquire), 1);
        assert_eq!(transport.active_calls(), 0);
    }

    #[tokio::test]
    async fn shutdown_deadline_still_closes_the_underlying_transport() {
        let inner = Arc::new(ClosingTransport {
            entered: Notify::new(),
            closed: CancellationToken::new(),
            fail_close: false,
            close_count: AtomicUsize::new(0),
        });
        let transport = Arc::new(QuiescentTransport::new(inner.clone()));
        let request = tokio::spawn({
            let transport = transport.clone();
            async move {
                transport
                    .dispatch("test/wait", serde_json::Value::Null)
                    .await
            }
        });
        inner.entered.notified().await;
        let shutdown = crate::loader::shutdown_transport(transport.clone()).await;
        let was_closed = inner.closed.is_cancelled();
        inner.closed.cancel();
        assert!(request.await.unwrap().is_err());
        assert!(matches!(shutdown, Err(TransportError::Timeout(_))));
        assert!(
            was_closed,
            "a graceful timeout must escalate to transport close"
        );
        assert_eq!(inner.close_count.load(Ordering::Acquire), 1);
        assert_eq!(transport.active_calls(), 0);
    }

    #[tokio::test]
    async fn repeated_close_keeps_the_cleanup_failure() {
        let inner = Arc::new(ClosingTransport {
            entered: Notify::new(),
            closed: CancellationToken::new(),
            fail_close: true,
            close_count: AtomicUsize::new(0),
        });
        let transport = QuiescentTransport::new(inner.clone());
        assert!(transport.close().await.is_err());
        let repeated = transport.close().await;
        assert_eq!(
            repeated.unwrap_err().to_string(),
            TransportError::Io("injected cleanup failure".into()).to_string()
        );
        assert_eq!(inner.close_count.load(Ordering::Acquire), 1);
    }

    struct StreamingTransport {
        shutdowns: Arc<AtomicUsize>,
        closes: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl PluginTransport for StreamingTransport {
        async fn dispatch(
            &self,
            method: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, TransportError> {
            if method == method::META_SHUTDOWN {
                self.shutdowns.fetch_add(1, Ordering::AcqRel);
            }
            Ok(serde_json::Value::Null)
        }

        async fn invoke_stream(
            &self,
            _input: ToolInvokeInput,
        ) -> Result<Option<ToolStreamHandle>, TransportError> {
            let (chunk_tx, chunk_rx) = tokio::sync::mpsc::channel(4);
            let (end_tx, end_rx) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                for index in 0..96 {
                    if chunk_tx
                        .send(ToolStreamChunk {
                            stream_id: "stream-1".to_string(),
                            text_delta: Some(format!("chunk-{index}")),
                            metadata: BTreeMap::new(),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                drop(chunk_tx);
                let _ = end_tx.send(Ok(ToolStreamEnd::text("stream-1", "done")));
            });
            Ok(Some(ToolStreamHandle {
                stream_id: "stream-1".to_string(),
                chunks: chunk_rx,
                end: end_rx,
            }))
        }

        async fn close(&self) -> Result<(), TransportError> {
            self.closes.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }

    #[tokio::test]
    async fn shutdown_drops_client_backpressure_but_waits_for_stream_terminal_state() {
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let closes = Arc::new(AtomicUsize::new(0));
        let transport = Arc::new(QuiescentTransport::new(Arc::new(StreamingTransport {
            shutdowns: Arc::clone(&shutdowns),
            closes: Arc::clone(&closes),
        })));
        let stream = transport
            .invoke_stream(ToolInvokeInput {
                tool_name: "stream".to_string(),
                session_id: 1,
                call_id: 1,
                workspace_root: "/tmp".to_string(),
                input: serde_json::json!({}),
            })
            .await
            .expect("stream starts")
            .expect("stream is supported");
        // Deliberately retain and never read the client receiver. The wrapper
        // reaches its forwarding-channel capacity while the underlying stream
        // is still producing.
        let _unread_stream = stream;
        while transport.active_calls() == 0 {
            tokio::task::yield_now().await;
        }

        tokio::time::timeout(std::time::Duration::from_secs(2), transport.shutdown())
            .await
            .expect("shutdown must not deadlock on unread client chunks")
            .expect("shutdown succeeds");

        assert_eq!(transport.active_calls(), 0);
        assert_eq!(shutdowns.load(Ordering::Acquire), 1);
        assert_eq!(closes.load(Ordering::Acquire), 1);
    }
}
