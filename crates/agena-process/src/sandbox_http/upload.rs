//! Source failures invalidate a request; a server closing its input early does
//! not invalidate an otherwise valid HTTP response such as 413 or a redirect.

use super::{count_bytes, ipc};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use std::io;
use tokio::{io::AsyncWrite, sync::watch};

#[derive(Clone)]
pub(super) struct UploadFailure {
    kind: io::ErrorKind,
    message: String,
}

impl UploadFailure {
    fn error(&self) -> io::Error {
        io::Error::new(self.kind, self.message.clone())
    }
}

pub(super) type Failures = watch::Receiver<Option<UploadFailure>>;

#[derive(Debug)]
pub(super) enum UploadError {
    Source(io::Error),
    Pipe(io::Error),
}

impl UploadError {
    pub(super) fn report(self, failures: &watch::Sender<Option<UploadFailure>>) -> io::Error {
        match self {
            Self::Source(error) => {
                failures.send_replace(Some(UploadFailure {
                    kind: error.kind(),
                    message: error.to_string(),
                }));
                error
            }
            Self::Pipe(error) => error,
        }
    }
}

pub(super) async fn write_chunks(
    writer: &mut (impl AsyncWrite + Unpin),
    source: impl Stream<Item = io::Result<Bytes>>,
    maximum: u64,
) -> Result<(), UploadError> {
    let mut source = Box::pin(source);
    let mut sent = 0;
    while let Some(chunk) = source.next().await {
        let chunk = chunk.map_err(UploadError::Source)?;
        count_bytes(&mut sent, chunk.len(), Some(maximum)).map_err(UploadError::Source)?;
        ipc::write_chunk(writer, &chunk)
            .await
            .map_err(UploadError::Pipe)?;
    }
    Ok(())
}

pub(super) fn check(failures: &Failures) -> io::Result<()> {
    match failures.borrow().as_ref() {
        Some(failure) => Err(failure.error()),
        None => Ok(()),
    }
}

pub(super) async fn failed(failures: &mut Failures) -> io::Error {
    loop {
        if let Err(error) = check(failures) {
            return error;
        }
        if failures.changed().await.is_err() {
            // The sender closes on a complete upload or a benign closed pipe.
            // Keep the other cancellation/response branches alive in select.
            return std::future::pending().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn upload_source_error_is_fatal_and_retained_after_task_exit() {
        let (sender, mut failures) = watch::channel(None);
        let error = write_chunks(
            &mut tokio::io::sink(),
            futures_util::stream::iter([Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "fixture body source failed",
            ))]),
            100,
        )
        .await
        .unwrap_err();
        assert!(matches!(&error, UploadError::Source(_)));
        assert_eq!(error.report(&sender).kind(), io::ErrorKind::InvalidData);
        drop(sender);
        assert_eq!(
            failed(&mut failures).await.kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[tokio::test]
    async fn unknown_length_upload_limit_is_a_source_failure() {
        let error = write_chunks(
            &mut tokio::io::sink(),
            futures_util::stream::iter([Ok(Bytes::from_static(b"too large"))]),
            3,
        )
        .await
        .unwrap_err();
        let UploadError::Source(error) = error else {
            panic!("request byte limit must invalidate the request");
        };
        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
    }

    #[tokio::test]
    async fn server_closing_input_early_does_not_publish_a_source_failure() {
        let (mut writer, reader) = tokio::io::duplex(16);
        drop(reader);
        let error = write_chunks(
            &mut writer,
            futures_util::stream::iter([Ok(Bytes::from_static(b"request"))]),
            100,
        )
        .await
        .unwrap_err();
        assert!(matches!(&error, UploadError::Pipe(_)));
        let (sender, mut failures) = watch::channel(None);
        assert_eq!(error.report(&sender).kind(), io::ErrorKind::BrokenPipe);
        drop(sender);
        assert!(check(&failures).is_ok());
        assert!(
            tokio::time::timeout(Duration::from_millis(10), failed(&mut failures))
                .await
                .is_err()
        );
    }
}
