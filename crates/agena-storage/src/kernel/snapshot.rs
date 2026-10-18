//! A pinned immutable membership root, read in bounded forward pages. No
//! connection or database transaction lives for the consumer's lifetime.

use crate::backend::kernel::{HistorySnapshotLease, HistorySnapshotOpened};

use super::*;
use tokio::io::AsyncWrite;

/// Counts and digest of all complete JSONL records before the success footer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReport {
    pub entries: u64,
    pub records: u64,
    pub bytes: u64,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, Serialize)]
pub struct HistorySnapshotInfo {
    pub metadata: SessionMetadata,
    pub entry_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotCursor {
    pub token: RequestId,
    pub segment_sequence: i64,
    pub ordinal: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotEntry {
    pub position: SnapshotCursor,
    pub entry: EntryView,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotPage {
    pub entries: Vec<SnapshotEntry>,
    pub next: Option<SnapshotCursor>,
    pub has_more: bool,
}

struct SnapshotInner {
    backend: Arc<dyn KernelBackend>,
    lease: HistorySnapshotLease,
    info: ReadView<HistorySnapshotInfo>,
}

impl Drop for SnapshotInner {
    fn drop(&mut self) {
        // Only releases a family. The writer reclaims pins/old references in
        // bounded maintenance transactions; Drop promises no successful I/O.
        self.backend.release_history_snapshot(self.lease);
    }
}

#[derive(Clone)]
pub struct HistorySnapshot(Arc<SnapshotInner>);

impl HistorySnapshot {
    fn new(backend: Arc<dyn KernelBackend>, opened: HistorySnapshotOpened) -> Result<Self> {
        backend.retain_history_snapshot(opened.lease)?;
        Ok(Self(Arc::new(SnapshotInner {
            backend,
            lease: opened.lease,
            info: opened.info,
        })))
    }

    pub fn info(&self) -> &ReadView<HistorySnapshotInfo> {
        &self.0.info
    }

    /// Forward transcript order, at most 200 entries and about 1 MiB of entry
    /// bodies. Membership is fixed; control outcomes and source heads are read
    /// separately at their declared per-record cut.
    pub async fn page(
        &self,
        after: Option<SnapshotCursor>,
        limit: usize,
    ) -> Result<ReadView<SnapshotPage>> {
        self.0
            .backend
            .snapshot_page(self.0.lease, after, limit)
            .await
    }

    /// Write the complete pinned membership as the versioned kernel JSONL
    /// archive. Metadata/membership are fixed at root creation; control facts
    /// are observed per record and each source stops at the committed head
    /// captured when visited. This is not a globally atomic database backup.
    /// A failed/cancelled export has no successful footer. The sink owns any
    /// partial output. No read transaction is held while awaiting the sink.
    pub async fn export_jsonl<W: AsyncWrite + Unpin + Send>(
        &self,
        target: &mut W,
    ) -> Result<ExportReport> {
        self.0
            .backend
            .export_jsonl(self.0.lease, &self.0.info, target)
            .await
    }
}

impl KernelReader {
    pub async fn history_snapshot(&self, session: SessionId) -> Result<HistorySnapshot> {
        let opened = self
            .0
            .backend
            .open_history_snapshot(session, self.0.epoch)
            .await?;
        HistorySnapshot::new(self.0.backend.clone(), opened)
    }

    pub async fn export_jsonl<W: AsyncWrite + Unpin + Send>(
        &self,
        session: SessionId,
        target: &mut W,
    ) -> Result<ExportReport> {
        self.history_snapshot(session)
            .await?
            .export_jsonl(target)
            .await
    }
}
