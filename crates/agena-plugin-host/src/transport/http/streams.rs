//! Each HTTP stream belongs to the authority of its originating invocation.
//! Pending registration starts before sending HTTP, so early callbacks can be
//! retained without accepting arbitrary unknown stream IDs. Registration and
//! replay share the delivery lock; cancellation removes pending state in Drop.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::error::TransportError;
use crate::sdk::host_api::HostCallbackContext;
use crate::sdk::{PluginError, PluginErrorKind, ToolStreamChunk, ToolStreamEnd, ToolStreamError};
use crate::transport::ToolStreamHandle;

const MAX_PENDING_STREAMS: usize = 128;
const MAX_STREAM_CHUNKS: usize = 64;

#[derive(Default)]
pub(super) struct StreamRegistry {
    state: Mutex<Streams>,
}

#[derive(Default)]
struct Streams {
    closed: bool,
    pending: HashMap<String, PendingStream>,
    active: HashMap<String, ActiveStream>,
}

struct PendingStream {
    context: HostCallbackContext,
    stream_id: Option<String>,
    chunks: Vec<ToolStreamChunk>,
    terminal: Option<Result<ToolStreamEnd, PluginError>>,
}

struct ActiveStream {
    context: HostCallbackContext,
    chunks: mpsc::Sender<ToolStreamChunk>,
    end: oneshot::Sender<Result<ToolStreamEnd, PluginError>>,
    monitor_stop: CancellationToken,
}

impl ActiveStream {
    fn finish(self, result: Result<ToolStreamEnd, PluginError>) {
        self.monitor_stop.cancel();
        if self.end.send(result).is_err() {
            tracing::debug!("HTTP plugin stream terminal-result receiver was dropped");
        }
    }
}

pub(super) enum StreamEvent {
    Chunk(ToolStreamChunk),
    End(ToolStreamEnd),
    Error(ToolStreamError),
}

impl StreamEvent {
    fn stream_id(&self) -> &str {
        match self {
            Self::Chunk(chunk) => &chunk.stream_id,
            Self::End(end) => &end.stream_id,
            Self::Error(error) => &error.stream_id,
        }
    }
}

pub(super) struct PendingRegistration {
    registry: Arc<StreamRegistry>,
    token: Option<String>,
}

impl PendingRegistration {
    pub(super) fn register(
        mut self,
        stream_id: String,
    ) -> Result<ToolStreamHandle, TransportError> {
        let token = self
            .token
            .take()
            .expect("pending registration owns its token");
        self.registry.register(&token, stream_id)
    }
}

impl Drop for PendingRegistration {
    fn drop(&mut self) {
        if let Some(token) = &self.token {
            self.registry.lock().pending.remove(token);
        }
    }
}

fn denied(message: &str) -> TransportError {
    PluginError::from_kind(PluginErrorKind::PolicyDenied, message).into()
}

fn authority(context: &HostCallbackContext) -> Result<&str, TransportError> {
    context
        .authority_token
        .as_deref()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| denied("HTTP plugin stream requires the originating callback authority"))
}

impl StreamRegistry {
    fn lock(&self) -> MutexGuard<'_, Streams> {
        self.state.lock().unwrap_or_else(|error| {
            tracing::error!(diagnostic = %error, "recovering a poisoned HTTP stream registry");
            error.into_inner()
        })
    }

    pub(super) fn begin(
        self: &Arc<Self>,
        context: HostCallbackContext,
    ) -> Result<PendingRegistration, TransportError> {
        let token = authority(&context)?.to_owned();
        let mut streams = self.lock();
        if streams.closed {
            return Err(TransportError::disconnected(
                "HTTP plugin stream transport is closed",
            ));
        }
        if streams.pending.contains_key(&token)
            || streams
                .active
                .values()
                .any(|stream| stream.context.authority_token.as_deref() == Some(&token))
        {
            return Err(denied(
                "HTTP plugin invocation authority already owns a stream",
            ));
        }
        if streams.pending.len() >= MAX_PENDING_STREAMS {
            return Err(TransportError::Rpc(
                "HTTP plugin exceeded the 128 pending-stream limit".into(),
            ));
        }
        streams.pending.insert(
            token.clone(),
            PendingStream {
                context,
                stream_id: None,
                chunks: Vec::new(),
                terminal: None,
            },
        );
        Ok(PendingRegistration {
            registry: Arc::clone(self),
            token: Some(token),
        })
    }

    pub(super) fn ingest(
        &self,
        context: HostCallbackContext,
        event: StreamEvent,
    ) -> Result<(), TransportError> {
        let token = authority(&context)?;
        let stream_id = event.stream_id();
        let mut streams = self.lock();
        if streams.closed {
            return Err(TransportError::disconnected(
                "HTTP plugin stream transport is closed",
            ));
        }
        if let Some(active) = streams.active.get(stream_id) {
            if active.context != context {
                return Err(denied(
                    "HTTP plugin callback authority does not own this stream",
                ));
            }
            streams.deliver(event);
            return Ok(());
        }
        if streams.pending.iter().any(|(owner, pending)| {
            owner != token && pending.stream_id.as_deref() == Some(stream_id)
        }) {
            return Err(denied(
                "HTTP plugin stream id is reserved by another invocation",
            ));
        }
        let pending = streams.pending.get_mut(token).ok_or_else(|| {
            denied("HTTP plugin callback has no matching active or pending stream invocation")
        })?;
        if pending.context != context {
            return Err(denied(
                "HTTP plugin callback context does not match the pending invocation",
            ));
        }
        if pending
            .stream_id
            .as_deref()
            .is_some_and(|expected| expected != stream_id)
        {
            return Err(denied(
                "HTTP plugin invocation supplied more than one stream id",
            ));
        }
        pending
            .stream_id
            .get_or_insert_with(|| stream_id.to_owned());
        // Preserve the first terminal, including overflow, until registration.
        // Its separate slot cannot discard a full but valid chunk buffer.
        if pending.terminal.is_none() {
            match event {
                StreamEvent::Chunk(chunk) => {
                    if pending.chunks.len() >= MAX_STREAM_CHUNKS {
                        pending.chunks.clear();
                        pending.terminal = Some(Err(PluginError::internal(
                            "plugin stream exceeded the 64-chunk pre-registration buffer",
                        )));
                    } else {
                        pending.chunks.push(chunk);
                    }
                }
                StreamEvent::End(end) => pending.terminal = Some(Ok(end)),
                StreamEvent::Error(error) => pending.terminal = Some(Err(error.error)),
            }
        }
        Ok(())
    }

    fn register(
        self: &Arc<Self>,
        token: &str,
        stream_id: String,
    ) -> Result<ToolStreamHandle, TransportError> {
        let (chunks, chunk_rx) = mpsc::channel(MAX_STREAM_CHUNKS);
        let (end, end_rx) = oneshot::channel();
        let monitor_stop = CancellationToken::new();
        {
            let mut streams = self.lock();
            let pending = streams.pending.remove(token).ok_or_else(|| {
                TransportError::disconnected("HTTP plugin stream invocation is no longer pending")
            })?;
            if streams.active.contains_key(&stream_id)
                || streams
                    .pending
                    .values()
                    .any(|other| other.stream_id.as_deref() == Some(&stream_id))
            {
                return Err(TransportError::Rpc(format!(
                    "HTTP plugin reused stream id `{stream_id}`"
                )));
            }
            if pending
                .stream_id
                .as_deref()
                .is_some_and(|expected| expected != stream_id)
            {
                return Err(TransportError::Rpc(
                    "HTTP plugin stream response disagrees with its early callbacks".into(),
                ));
            }
            streams.active.insert(
                stream_id.clone(),
                ActiveStream {
                    context: pending.context,
                    chunks: chunks.clone(),
                    end,
                    monitor_stop: monitor_stop.clone(),
                },
            );
            for chunk in pending.chunks {
                streams.deliver(StreamEvent::Chunk(chunk));
            }
            if let Some(terminal) = pending.terminal
                && let Some(active) = streams.active.remove(&stream_id)
            {
                active.finish(terminal);
            }
        }
        let registry = Arc::downgrade(self);
        let abandoned_id = stream_id.clone();
        tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = monitor_stop.cancelled() => {}
                _ = chunks.closed() => {
                    if let Some(registry) = registry.upgrade() {
                        let mut streams = registry.lock();
                        // A delayed monitor may observe a new stream with the
                        // same remote ID. Remove only its own channel pair.
                        if streams.active.get(&abandoned_id).is_some_and(|active| active.chunks.same_channel(&chunks))
                            && let Some(active) = streams.active.remove(&abandoned_id)
                        {
                            active.monitor_stop.cancel();
                        }
                    }
                }
            }
        });
        Ok(ToolStreamHandle {
            stream_id,
            chunks: chunk_rx,
            end: end_rx,
        })
    }

    pub(super) fn close(&self, error: PluginError) {
        let mut streams = self.lock();
        streams.closed = true;
        streams.fail(error);
    }

    pub(super) fn interrupt(&self, error: PluginError) {
        self.lock().fail(error);
    }
}

impl Streams {
    fn fail(&mut self, error: PluginError) {
        self.pending.clear();
        for (_, active) in self.active.drain() {
            active.finish(Err(error.clone()));
        }
    }
    fn deliver(&mut self, event: StreamEvent) {
        let stream_id = event.stream_id().to_owned();
        match event {
            StreamEvent::Chunk(chunk) => {
                if let Some(active) = self.active.get(&stream_id) {
                    match active.chunks.try_send(chunk) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            if let Some(active) = self.active.remove(&stream_id) {
                                active.monitor_stop.cancel();
                            }
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            if let Some(active) = self.active.remove(&stream_id) {
                                active.finish(Err(PluginError::internal(
                                    "plugin stream consumer exceeded the 64-chunk buffer",
                                )));
                            }
                        }
                    }
                }
            }
            StreamEvent::End(end) => {
                if let Some(active) = self.active.remove(&stream_id) {
                    active.finish(Ok(end));
                }
            }
            StreamEvent::Error(error) => {
                if let Some(active) = self.active.remove(&stream_id) {
                    active.finish(Err(error.error));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
