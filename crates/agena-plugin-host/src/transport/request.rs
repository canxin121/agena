//! One remote request lifetime, including the stream after its setup reply.

use crate::sdk::PluginError;

use super::ToolStreamHandle;

pub(super) struct RemoteRequest {
    cancel: Option<Box<dyn FnOnce() + Send>>,
}

impl RemoteRequest {
    pub(super) fn new(cancel: impl FnOnce() + Send + 'static) -> Self {
        Self {
            cancel: Some(Box::new(cancel)),
        }
    }

    pub(super) fn complete(&mut self) {
        self.cancel.take();
    }

    /// Keep the originating RPC alive until the terminal frame is observed.
    /// Dropping the terminal receiver cancels exactly that request, including
    /// the interval between setup and the first notification. Chunk output may
    /// be ignored while the caller still waits for the final result.
    pub(super) fn bind_stream(mut self, mut source: ToolStreamHandle) -> ToolStreamHandle {
        let stream_id = source.stream_id.clone();
        let (chunks, chunk_rx) = tokio::sync::mpsc::channel(32);
        let (mut end, end_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut chunks_open = true;
            let mut forward_chunks = true;
            loop {
                tokio::select! {
                    biased;
                    _ = end.closed() => return,
                    _ = chunks.closed(), if forward_chunks => forward_chunks = false,
                    terminal = &mut source.end => {
                        let terminal = match terminal {
                            Ok(result) => {
                                // A local stream failure (for example bounded
                                // buffer overflow) also arrives on this channel.
                                // Keep cancellation armed on errors so the
                                // producer cannot outlive a failed consumer.
                                if result.is_ok() {
                                    self.complete();
                                }
                                result
                            }
                            Err(error) => Err(PluginError::internal(format!(
                                "plugin stream ended without a terminal result: {error}"
                            ))),
                        };
                        // End may win while preceding chunks are still buffered.
                        source.chunks.close();
                        while let Ok(chunk) = source.chunks.try_recv() {
                            if !forward_chunks {
                                continue;
                            }
                            tokio::select! {
                                biased;
                                _ = end.closed() => return,
                                result = chunks.send(chunk) => if result.is_err() { forward_chunks = false; },
                            }
                        }
                        let _ = end.send(terminal);
                        return;
                    }
                    chunk = source.chunks.recv(), if chunks_open => match chunk {
                        Some(chunk) => {
                            if !forward_chunks {
                                continue;
                            }
                            tokio::select! {
                                biased;
                                _ = end.closed() => return,
                                result = chunks.send(chunk) => if result.is_err() { forward_chunks = false; },
                            }
                        }
                        None => chunks_open = false,
                    },
                }
            }
        });
        ToolStreamHandle {
            stream_id,
            chunks: chunk_rx,
            end: end_rx,
        }
    }
}

impl Drop for RemoteRequest {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn discarding_chunks_preserves_the_terminal_request_lifetime() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let request = RemoteRequest::new(move || cancel.store(true, Ordering::SeqCst));
        let (_chunks_tx, chunks) = tokio::sync::mpsc::channel(1);
        let (terminal, end) = tokio::sync::oneshot::channel();
        let stream = request.bind_stream(ToolStreamHandle {
            stream_id: "test".into(),
            chunks,
            end,
        });
        drop(stream.chunks);
        terminal
            .send(Ok(crate::sdk::ToolStreamEnd::text("test", "done")))
            .unwrap();
        assert_eq!(stream.end.await.unwrap().unwrap().output_text, "done");
        assert!(!cancelled.load(Ordering::SeqCst));
    }
}
