//! Managed immutable bytes. Preparation streams bounded chunks; publication
//! belongs to send/tool completion, and readers retain a separate GC pin.

use agena_domain::BlobId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::*;
use crate::backend::kernel::{BlobLease, BlobPin};

pub const BLOB_CHUNK_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobDescriptor {
    pub id: BlobId,
    pub byte_len: u64,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct BlobRange {
    pub start: u64,
    pub next: u64,
    pub bytes: Vec<u8>,
    pub eof: bool,
}

struct Pin {
    inner: Arc<KernelInner>,
    lease: BlobLease,
}
impl Pin {
    fn new(inner: Arc<KernelInner>, value: BlobPin) -> Result<Arc<Self>> {
        inner.backend.retain_blob(value.lease)?;
        Ok(Arc::new(Self {
            inner,
            lease: value.lease,
        }))
    }
}
impl Drop for Pin {
    fn drop(&mut self) {
        self.inner.backend.release_blob(self.lease);
    }
}

#[derive(Clone)]
pub struct PreparedBlob {
    descriptor: BlobDescriptor,
    pin: Arc<Pin>,
}
impl PreparedBlob {
    pub fn descriptor(&self) -> &BlobDescriptor {
        &self.descriptor
    }
    pub fn reader(&self) -> BlobReader {
        BlobReader {
            descriptor: self.descriptor.clone(),
            pin: self.pin.clone(),
        }
    }
}

#[derive(Clone)]
pub struct BlobReader {
    descriptor: BlobDescriptor,
    pin: Arc<Pin>,
}
impl BlobReader {
    /// Reuse an accessible immutable blob in a new entry revision. The read
    /// capability retains its lifetime pin and session ownership checks.
    pub fn into_prepared(self) -> PreparedBlob {
        PreparedBlob {
            descriptor: self.descriptor,
            pin: self.pin,
        }
    }

    pub fn descriptor(&self) -> &BlobDescriptor {
        &self.descriptor
    }
    pub async fn range(&self, offset: u64, limit: usize) -> Result<ReadView<BlobRange>> {
        self.pin
            .inner
            .backend
            .blob_range(self.pin.lease, offset, limit)
            .await
    }
    /// Each page checks its physical chunk digest. Full streaming copies also
    /// check the whole-blob SHA-256, including empty input.
    pub async fn copy_to<W: AsyncWrite + Unpin + Send>(&self, target: &mut W) -> Result<u64> {
        let mut offset = 0;
        let mut hash = Sha256::new();
        loop {
            let page = self.range(offset, BLOB_CHUNK_BYTES).await?;
            hash.update(&page.bytes);
            target
                .write_all(&page.bytes)
                .await
                .map_err(|e| KernelError::Storage(e.to_string()))?;
            offset = page.next;
            if page.eof {
                break;
            }
        }
        if offset != self.descriptor.byte_len
            || <[u8; 32]>::from(hash.finalize()) != self.descriptor.sha256
        {
            return Err(KernelError::Storage("blob length/checksum mismatch".into()));
        }
        Ok(offset)
    }
}

fn request_at(request: RequestId, tag: &[u8], offset: u64) -> RequestId {
    let mut hash = Sha256::new();
    hash.update(request.to_string());
    hash.update(tag);
    hash.update(offset.to_le_bytes());
    hex::encode(hash.finalize())[..32]
        .parse()
        .expect("128-bit request identity")
}

fn bindings(
    inner: &Arc<KernelInner>,
    session: SessionId,
    blobs: &[PreparedBlob],
) -> Result<Vec<BlobLease>> {
    if blobs.len() > 16 {
        return Err(KernelError::Invalid(
            "at most 16 managed blobs per entry".into(),
        ));
    }
    blobs
        .iter()
        .map(|blob| {
            if !Arc::ptr_eq(inner, &blob.pin.inner) || blob.pin.lease.session != session {
                return Err(KernelError::Ownership);
            }
            Ok(blob.pin.lease)
        })
        .collect()
}

async fn pin_blob(
    inner: &Arc<KernelInner>,
    session: SessionId,
    blob: BlobId,
    preparation: bool,
) -> Result<BlobPin> {
    let receipt = inner
        .command(
            RequestId::new(),
            Command::PinBlob {
                session,
                blob,
                epoch: inner.epoch,
                preparation,
            },
        )
        .await?;
    match receipt.value {
        Mutation::BlobPin(pin) => Ok(*pin),
        _ => Err(unexpected()),
    }
}

impl SessionCommands {
    /// Replace unclaimed input without changing its FIFO identity. Frozen or
    /// previously executed input cannot be edited, including queued retries.
    pub async fn edit_queued_with_blobs(
        &self,
        request: RequestId,
        session: SessionId,
        execution: ExecutionId,
        expected_revision: EntryRevision,
        document: ComposerDocument,
        blobs: &[PreparedBlob],
    ) -> Result<CommitReceipt<ExecutionView>> {
        let blobs = bindings(&self.0, session, blobs)?;
        execution_receipt(
            self.0
                .command(
                    request,
                    Command::EditQueued {
                        session,
                        execution,
                        expected_revision,
                        document,
                        blobs,
                    },
                )
                .await?,
        )
    }

    /// Copy only bytes explicitly supplied by the caller. Local-path attachment
    /// references keep their existing semantics and are never implicitly read.
    /// Replaying a request reads and verifies the same stream/chunk identities.
    pub async fn prepare_blob<R: AsyncRead + Unpin + Send>(
        &self,
        request: RequestId,
        session: SessionId,
        expected_bytes: u64,
        source: &mut R,
    ) -> Result<PreparedBlob> {
        let _upload = self.0.backend.admit_blob_upload()?;
        let receipt = self
            .0
            .command(
                request,
                Command::PrepareBlob {
                    session,
                    expected_bytes,
                },
            )
            .await?;
        let record = match receipt.value {
            Mutation::BlobRecord(record) => record,
            _ => return Err(unexpected()),
        };
        let pin = Pin::new(
            self.0.clone(),
            pin_blob(&self.0, session, record.id, true).await?,
        )?;
        let mut buffer = vec![0; BLOB_CHUNK_BYTES];
        let mut hash = Sha256::new();
        let mut offset = 0;
        while offset < expected_bytes {
            let length = (expected_bytes - offset).min(BLOB_CHUNK_BYTES as u64) as usize;
            source
                .read_exact(&mut buffer[..length])
                .await
                .map_err(|e| {
                    KernelError::Invalid(format!(
                        "blob input ended or failed before its declared length: {e}"
                    ))
                })?;
            hash.update(&buffer[..length]);
            let receipt = self
                .0
                .command(
                    request_at(request, b"chunk", offset),
                    Command::WriteBlob {
                        lease: pin.lease,
                        offset,
                        bytes: buffer[..length].to_vec(),
                    },
                )
                .await?;
            match receipt.value {
                Mutation::BlobOffset(end) if end == offset + length as u64 => offset = end,
                _ => return Err(unexpected()),
            }
        }
        if source
            .read(&mut buffer[..1])
            .await
            .map_err(|e| KernelError::Invalid(e.to_string()))?
            != 0
        {
            return Err(KernelError::Invalid(
                "blob input exceeds its declared length".into(),
            ));
        }
        let receipt = self
            .0
            .command(
                request_at(request, b"ready", 0),
                Command::ReadyBlob {
                    lease: pin.lease,
                    sha256: hash.finalize().into(),
                },
            )
            .await?;
        match receipt.value {
            Mutation::BlobReady(descriptor) => Ok(PreparedBlob { descriptor, pin }),
            _ => Err(unexpected()),
        }
    }

    pub async fn send_with_blobs(
        &self,
        request: RequestId,
        session: SessionId,
        document: ComposerDocument,
        blobs: &[PreparedBlob],
    ) -> Result<CommitReceipt<ExecutionView>> {
        let blobs = bindings(&self.0, session, blobs)?;
        execution_receipt(
            self.0
                .command(
                    request,
                    Command::Send {
                        session,
                        document,
                        blobs,
                    },
                )
                .await?,
        )
    }
}

impl ToolHandle {
    pub async fn complete_with_blobs(
        &self,
        request: RequestId,
        result: ToolResult,
        blobs: &[PreparedBlob],
    ) -> Result<CommitReceipt<ToolId>> {
        let inner = &self.round.attempt.inner;
        let scope = self.round.attempt.scope;
        let blobs = bindings(inner, scope.session, blobs)?;
        let receipt = inner
            .command(
                request,
                Command::CompleteTool {
                    scope,
                    tool: self.id,
                    result,
                    blobs,
                },
            )
            .await?;
        match receipt.value {
            Mutation::Tool(value) => Ok(CommitReceipt {
                position: receipt.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }
}

impl KernelReader {
    pub async fn open_blob(&self, session: SessionId, blob: BlobId) -> Result<BlobReader> {
        let value = pin_blob(&self.0, session, blob, false).await?;
        let descriptor = BlobDescriptor {
            id: value.record.id,
            byte_len: value.record.byte_len,
            sha256: value.record.sha256.ok_or_else(unexpected)?,
        };
        let pin = Pin::new(self.0.clone(), value)?;
        Ok(BlobReader { descriptor, pin })
    }
    pub async fn entry_blobs(
        &self,
        session: SessionId,
        entry: EntryId,
    ) -> Result<ReadView<Vec<BlobDescriptor>>> {
        self.0.backend.entry_blobs(session, entry).await
    }
}
