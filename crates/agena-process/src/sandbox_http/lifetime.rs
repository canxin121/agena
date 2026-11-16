//! Lifetimes continue to advance while the returned response is not polled.

use super::{WorkerCancellation, count_bytes};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle, time::Instant};

#[derive(Clone)]
pub(super) struct RequestLifetime {
    cancellation: Option<WorkerCancellation>,
    deadline: Option<Instant>,
}

impl RequestLifetime {
    pub(super) fn new(
        started: Instant,
        total: Option<Duration>,
        cancellation: Option<WorkerCancellation>,
    ) -> io::Result<Self> {
        let deadline = total
            .map(|total| {
                started.checked_add(total).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "HTTP timeout is too large")
                })
            })
            .transpose()?;
        Ok(Self {
            cancellation,
            deadline,
        })
    }

    pub(super) fn check(&self) -> io::Result<()> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(WorkerCancellation::is_cancelled)
        {
            return Err(cancelled());
        }
        if self
            .deadline
            .is_some_and(|deadline| deadline <= Instant::now())
        {
            return Err(timed_out());
        }
        Ok(())
    }

    pub(super) async fn stopped(&self) -> io::Error {
        if let Err(error) = self.check() {
            return error;
        }
        tokio::select! {
            biased;
            _ = async {
                match &self.cancellation {
                    Some(cancellation) => cancellation.cancelled().await,
                    None => std::future::pending().await,
                }
            } => cancelled(),
            _ = async {
                match self.deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            } => timed_out(),
        }
    }
}

fn cancelled() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "originating HTTP call has ended",
    )
}

fn timed_out() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "HTTP operation exceeded its total timeout",
    )
}

/// The pump owns the socket body, so a quiet or backpressured caller cannot
/// postpone cancellation. Only one completed chunk is buffered at a time.
pub(super) struct PumpedBody {
    receiver: mpsc::Receiver<io::Result<Bytes>>,
    pump: JoinHandle<()>,
    finished: bool,
}

impl Stream for PumpedBody {
    type Item = io::Result<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }
        match self.receiver.poll_recv(context) {
            Poll::Ready(None) => match Pin::new(&mut self.pump).poll(context) {
                Poll::Ready(result) => {
                    self.finished = true;
                    Poll::Ready(result.err().map(|error| {
                        Err(io::Error::other(format!(
                            "HTTP response pump failed: {error}"
                        )))
                    }))
                }
                Poll::Pending => Poll::Pending,
            },
            result => result,
        }
    }
}

impl Drop for PumpedBody {
    fn drop(&mut self) {
        self.pump.abort();
    }
}

pub(super) fn pump_body(
    source: impl Stream<Item = io::Result<Bytes>> + Send + 'static,
    maximum: Option<u64>,
    lifetime: RequestLifetime,
) -> PumpedBody {
    let (sender, receiver) = mpsc::channel(1);
    let pump = tokio::spawn(async move {
        let mut source = Box::pin(source);
        let result = {
            let forward = async {
                let mut total = 0;
                while let Some(chunk) = source.next().await {
                    let chunk = chunk?;
                    count_bytes(&mut total, chunk.len(), maximum)?;
                    if sender.send(Ok(chunk)).await.is_err() {
                        return Ok::<_, io::Error>(());
                    }
                }
                Ok(())
            };
            // This covers both waiting for a socket and waiting for a free
            // channel slot. It is independent of the consumer's polling.
            tokio::select! {
                biased;
                error = lifetime.stopped() => Err(error),
                result = forward => result,
            }
        };
        // Release the real response/connection before waiting for the error to
        // fit behind buffered data. A full channel must not erase the failure
        // and turn cancellation or a short response into successful EOF.
        drop(source);
        if let Err(error) = result {
            let _ = sender.send(Err(error)).await;
        }
    });
    PumpedBody {
        receiver,
        pump,
        finished: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use tokio::sync::oneshot;

    struct ObservedBody {
        chunks: VecDeque<io::Result<Bytes>>,
        reached_pending: Option<oneshot::Sender<()>>,
        dropped: Option<oneshot::Sender<()>>,
    }

    impl Stream for ObservedBody {
        type Item = io::Result<Bytes>;

        fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let chunk = self.chunks.pop_front();
            if self.chunks.is_empty()
                && let Some(reached_pending) = self.reached_pending.take()
            {
                let _ = reached_pending.send(());
            }
            match chunk {
                Some(chunk) => Poll::Ready(Some(chunk)),
                None => Poll::Pending,
            }
        }
    }

    impl Drop for ObservedBody {
        fn drop(&mut self) {
            if let Some(dropped) = self.dropped.take() {
                let _ = dropped.send(());
            }
        }
    }

    fn observed(
        chunks: impl IntoIterator<Item = io::Result<Bytes>>,
    ) -> (ObservedBody, oneshot::Receiver<()>, oneshot::Receiver<()>) {
        let (pending, reached_pending) = oneshot::channel();
        let (drop, dropped) = oneshot::channel();
        (
            ObservedBody {
                chunks: chunks.into_iter().collect(),
                reached_pending: Some(pending),
                dropped: Some(drop),
            },
            reached_pending,
            dropped,
        )
    }

    async fn received(signal: oneshot::Receiver<()>) {
        tokio::time::timeout(Duration::from_secs(2), signal)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn cancellation_closes_a_quiet_body_without_polling_the_response() {
        let cancellation = WorkerCancellation::new();
        let lifetime =
            RequestLifetime::new(Instant::now(), None, Some(cancellation.clone())).unwrap();
        let (source, pending, dropped) = observed([]);
        let mut body = pump_body(source, None, lifetime);
        received(pending).await;
        cancellation.cancel();
        received(dropped).await;
        assert_eq!(
            body.next().await.unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(body.next().await.is_none());
    }

    #[tokio::test]
    async fn cancellation_closes_a_backpressured_body_before_delivering_its_error() {
        let cancellation = WorkerCancellation::new();
        let lifetime =
            RequestLifetime::new(Instant::now(), None, Some(cancellation.clone())).unwrap();
        let (source, pending, dropped) = observed([
            Ok(Bytes::from_static(b"first")),
            Ok(Bytes::from_static(b"second")),
        ]);
        let mut body = pump_body(source, None, lifetime);
        received(pending).await;
        cancellation.cancel();
        // The receiver is still full and unpolled. The actual response has to
        // close now, before there is room to deliver the terminal error.
        received(dropped).await;
        assert_eq!(body.next().await.unwrap().unwrap(), "first");
        assert_eq!(
            body.next().await.unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(body.next().await.is_none());
    }

    #[tokio::test]
    async fn absolute_deadline_closes_an_unpolled_backpressured_body() {
        let lifetime =
            RequestLifetime::new(Instant::now(), Some(Duration::from_millis(200)), None).unwrap();
        let (source, pending, dropped) = observed([
            Ok(Bytes::from_static(b"first")),
            Ok(Bytes::from_static(b"second")),
        ]);
        let mut body = pump_body(source, None, lifetime);
        received(pending).await;
        received(dropped).await;
        assert_eq!(body.next().await.unwrap().unwrap(), "first");
        assert_eq!(
            body.next().await.unwrap().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(body.next().await.is_none());
    }

    #[tokio::test]
    async fn total_deadline_keeps_time_spent_before_body_creation() {
        let lifetime = RequestLifetime::new(
            Instant::now() - Duration::from_secs(2),
            Some(Duration::from_secs(1)),
            None,
        )
        .unwrap();
        assert_eq!(
            lifetime.check().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        let (source, _, dropped) = observed([]);
        let mut body = pump_body(source, None, lifetime);
        received(dropped).await;
        assert_eq!(
            body.next().await.unwrap().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[tokio::test]
    async fn dropping_response_aborts_the_pump_and_releases_the_real_body() {
        let lifetime = RequestLifetime::new(Instant::now(), None, None).unwrap();
        let (source, pending, dropped) = observed([]);
        let body = pump_body(source, None, lifetime);
        received(pending).await;
        drop(body);
        received(dropped).await;
    }

    #[tokio::test]
    async fn source_errors_and_byte_limits_survive_a_full_response_channel() {
        for (second, maximum, expected) in [
            (
                Err(io::Error::other("fixture source failure")),
                None,
                io::ErrorKind::Other,
            ),
            (
                Ok(Bytes::from_static(b"second")),
                Some(5),
                io::ErrorKind::FileTooLarge,
            ),
        ] {
            let lifetime = RequestLifetime::new(Instant::now(), None, None).unwrap();
            let (source, _, dropped) = observed([Ok(Bytes::from_static(b"first")), second]);
            let mut body = pump_body(source, maximum, lifetime);
            received(dropped).await;
            assert_eq!(body.next().await.unwrap().unwrap(), "first");
            assert_eq!(body.next().await.unwrap().unwrap_err().kind(), expected);
            assert!(body.next().await.is_none());
        }
    }
}
