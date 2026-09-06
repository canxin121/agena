//! Stream registration and delivery share one lock. Publishing a stream and
//! replaying its early events must be atomic with delivery of later events.

use super::*;

#[derive(Default)]
pub(super) struct StreamRegistry {
    active: HashMap<String, ActiveStreamState>,
    buffered: HashMap<String, BufferedStream>,
}

struct ActiveStreamState {
    chunks: mpsc::Sender<ToolStreamChunk>,
    end: oneshot::Sender<Result<ToolStreamEnd, PluginError>>,
    monitor_stop: CancellationToken,
}

#[derive(Default)]
struct BufferedStream {
    chunks: Vec<ToolStreamChunk>,
    // The terminal result has its own slot. It cannot erase retained chunks
    // at capacity, and a later success cannot replace a recorded overflow.
    terminal: Option<Result<ToolStreamEnd, PluginError>>,
}

impl ActiveStreamState {
    fn finish(self, result: Result<ToolStreamEnd, PluginError>) {
        self.monitor_stop.cancel();
        if self.end.send(result).is_err() {
            tracing::debug!(target: "agena_plugin_host::stdio", "stdio plugin stream terminal receiver was dropped");
        }
    }
}

impl StreamRegistry {
    fn deliver_chunk(&mut self, chunk: ToolStreamChunk) {
        let stream_id = chunk.stream_id.clone();
        if let Some(state) = self.active.get(&stream_id) {
            match state.chunks.try_send(chunk) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    if let Some(state) = self.active.remove(&stream_id) {
                        state.monitor_stop.cancel();
                    }
                    self.buffered.remove(&stream_id);
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    if let Some(state) = self.active.remove(&stream_id) {
                        state.finish(Err(PluginError::internal(
                            "plugin stream consumer exceeded the 64-chunk buffer",
                        )));
                    }
                }
            }
            return;
        }
        if !self.buffered.contains_key(&stream_id) && self.buffered.len() >= MAX_BUFFERED_STREAMS {
            tracing::warn!(target: "agena_plugin_host::stdio", stream_id, "dropping unknown plugin stream event because its buffer is full");
            return;
        }
        let buffered = self.buffered.entry(stream_id).or_default();
        if buffered.terminal.is_some() {
            return;
        }
        if buffered.chunks.len() >= MAX_BUFFERED_STREAM_CHUNKS {
            buffered.chunks.clear();
            buffered.terminal = Some(Err(PluginError::internal(
                "plugin stream exceeded the 64-chunk pre-registration buffer",
            )));
        } else {
            buffered.chunks.push(chunk);
        }
    }

    fn finish_stream(&mut self, stream_id: String, result: Result<ToolStreamEnd, PluginError>) {
        if let Some(state) = self.active.remove(&stream_id) {
            state.finish(result);
            return;
        }
        if !self.buffered.contains_key(&stream_id) && self.buffered.len() >= MAX_BUFFERED_STREAMS {
            tracing::warn!(target: "agena_plugin_host::stdio", stream_id, "dropping unknown plugin stream terminal event because its buffer is full");
            return;
        }
        self.buffered
            .entry(stream_id)
            .or_default()
            .terminal
            .get_or_insert(result);
    }
}

impl Inner {
    pub(super) async fn deliver_stream_chunk(&self, chunk: ToolStreamChunk) {
        self.streams.lock().await.deliver_chunk(chunk);
    }

    pub(super) async fn finish_stream(
        &self,
        stream_id: String,
        result: Result<ToolStreamEnd, PluginError>,
    ) {
        self.streams.lock().await.finish_stream(stream_id, result);
    }

    pub(super) async fn register_stream(
        self: &Arc<Self>,
        stream_id: String,
        chunks: mpsc::Sender<ToolStreamChunk>,
        end: oneshot::Sender<Result<ToolStreamEnd, PluginError>>,
        generation: u64,
    ) -> Result<(), TransportError> {
        let handles = self.handles.lock().await;
        if !handles
            .as_ref()
            .is_some_and(|handles| handles.generation == generation && !handles.stop.is_cancelled())
        {
            return Err(TransportError::disconnected(
                "stdio plugin exited before its stream could be registered",
            ));
        }
        let monitor_stop = CancellationToken::new();
        {
            let mut streams = self.streams.lock().await;
            if streams.active.contains_key(&stream_id) {
                return Err(TransportError::Io(format!(
                    "stdio plugin reused active stream id `{stream_id}`"
                )));
            }
            streams.active.insert(
                stream_id.clone(),
                ActiveStreamState {
                    chunks: chunks.clone(),
                    end,
                    monitor_stop: monitor_stop.clone(),
                },
            );
            let buffered = streams.buffered.remove(&stream_id).unwrap_or_default();
            for chunk in buffered.chunks {
                streams.deliver_chunk(chunk);
            }
            if let Some(terminal) = buffered.terminal {
                streams.finish_stream(stream_id.clone(), terminal);
            }
        }
        drop(handles);
        let weak = Arc::downgrade(self);
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = monitor_stop.cancelled() => {}
                _ = shutdown.cancelled() => {}
                _ = chunks.closed() => {
                    if let Some(inner) = weak.upgrade() {
                        let mut streams = inner.streams.lock().await;
                        // A finished stream's delayed monitor must not remove
                        // a later registration that reused the same id.
                        if !monitor_stop.is_cancelled() {
                            if let Some(state) = streams.active.remove(&stream_id) {
                                state.monitor_stop.cancel();
                            }
                            streams.buffered.remove(&stream_id);
                        }
                    }
                }
            }
        });
        Ok(())
    }

    pub(super) async fn fail_active_streams(&self, error: PluginError) {
        let mut streams = self.streams.lock().await;
        streams.buffered.clear();
        for (_, state) in streams.active.drain() {
            state.finish(Err(error.clone()));
        }
    }
}
