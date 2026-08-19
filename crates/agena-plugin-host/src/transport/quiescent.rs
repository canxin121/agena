//! Quiescent lifecycle wrapper for every plugin transport.
//!
//! A transport shutdown first closes admission, then waits for every accepted
//! dispatch/notification/stream to settle, and only then runs the plugin's
//! shutdown hook and closes the underlying transport. This is the host-side
//! equivalent of an effect scope reaching quiescence rather than merely
//! requesting cancellation.

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
    finished: bool,
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
            return Err(TransportError::Disconnected);
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        // Close the race where shutdown flips admission after the first check
        // but before this call becomes visible in the active count.
        if !self.accepting.load(Ordering::Acquire) {
            if self.active.fetch_sub(1, Ordering::AcqRel) == 1 {
                self.idle.notify_waiters();
            }
            return Err(TransportError::Disconnected);
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
        if finish.finished {
            return Ok(());
        }
        self.state.accepting.store(false, Ordering::Release);
        self.state.closing.cancel();
        self.state.wait_idle().await;
        if graceful {
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
        finish.finished = true;
        result
    }
}

#[async_trait]
impl PluginTransport for QuiescentTransport {
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
        tokio::spawn(async move {
            let _activity = activity;
            let mut forward_chunks = true;
            while let Some(chunk) = inner.chunks.recv().await {
                // A dropped consumer must not abandon the underlying stream:
                // continue draining until the plugin reports a terminal result.
                // Once shutdown begins, forwarding must also stop applying
                // backpressure, otherwise an unread client channel could keep
                // the plugin from ever becoming quiescent.
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
            let result = inner.end.await.unwrap_or_else(|error| {
                Err(PluginError::internal(format!(
                    "plugin stream ended without a terminal result: {error}"
                )))
            });
            let _ = end_tx.send(result);
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
            Err(TransportError::Disconnected)
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
