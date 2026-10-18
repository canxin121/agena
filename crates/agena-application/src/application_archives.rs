//! Portable current-format archives through purpose-specific runtime services.

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::{Application, ApplicationError};

static ARCHIVE_IMPORTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
const MAX_ARCHIVE_LINE_BYTES: usize = 32 * 1024 * 1024 + 1;

impl Application {
    /// Write a fixed-membership archive with bounded buffers. Failure leaves
    /// partial bytes without a successful footer in the caller-owned sink.
    pub async fn export_session_to<W: AsyncWrite + Unpin + Send>(
        &self,
        session_id: i64,
        target: &mut W,
    ) -> Result<(), ApplicationError> {
        self.session_execution_services()?
            .queries
            .export_session_to(session_id, target)
            .await
            .map_err(|error| ApplicationError::from_failure(*error.failure))
    }

    /// Stage the incoming bytes on disk, then let the kernel validate the
    /// complete footer and atomically publish one fresh session. The owned
    /// temporary file is removed on success, failure, and cancellation.
    pub async fn import_session_archive<R: AsyncRead + Unpin + Send>(
        &self,
        request_id: agena_domain::CommandId,
        source: &mut R,
    ) -> Result<agena_api::resource::SessionArchiveImportResource, ApplicationError> {
        let _permit = ARCHIVE_IMPORTS.try_acquire().map_err(|error| {
            ApplicationError::service_unavailable(format!("archive import admission: {error}"))
        })?;
        let archive = self
            .run_blocking(|_| tempfile::NamedTempFile::new())
            .await?
            .map_err(|error| ApplicationError::internal_error(&error))?;
        let (file, path) = archive.into_parts();
        let mut file = tokio::fs::File::from_std(file);
        let mut buffer = vec![0_u8; 64 * 1024];
        let mut line_bytes = 0_usize;
        loop {
            let count = source
                .read(&mut buffer)
                .await
                .map_err(|error| ApplicationError::bad_request_error(&error))?;
            if count == 0 {
                break;
            }
            for piece in buffer[..count].split_inclusive(|byte| *byte == b'\n') {
                line_bytes = line_bytes.saturating_add(piece.len());
                if line_bytes > MAX_ARCHIVE_LINE_BYTES {
                    return Err(ApplicationError::bad_request(
                        "archive record exceeds byte limit",
                    ));
                }
                if piece.ends_with(b"\n") {
                    line_bytes = 0;
                }
            }
            file.write_all(&buffer[..count])
                .await
                .map_err(|error| ApplicationError::internal_error(&error))?;
        }
        file.flush()
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?;
        drop(file);
        let outcome = self
            .session_execution_services()?
            .commands
            .import_session_file(request_id, path.as_ref())
            .await
            .map_err(|error| ApplicationError::from_failure(error.failure))?;
        // Publication has already succeeded. Return its committed identity
        // without making success depend on another fallible state/history read.
        Ok(agena_api::resource::SessionArchiveImportResource {
            session_id: outcome.session_id,
        })
    }
}
