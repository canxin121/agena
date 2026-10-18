//! Text capabilities use byte offsets independently of packet/block boundaries.
//! An append receipt acknowledges memory admission. Terminal business commands
//! and the kernel's shared scheduler provide persistence without caller flushes.

use std::sync::Arc;

use agena_domain::{ContentEpoch, ContentId, ContentKind};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use super::*;
use crate::backend::kernel::OutputScope;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextCursor {
    pub content: ContentId,
    pub epoch: ContentEpoch,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentDescriptor {
    pub id: ContentId,
    pub entry: EntryId,
    pub epoch: ContentEpoch,
    pub kind: ContentKind,
    pub activity_id: Option<agena_domain::ActivityId>,
    pub ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureEnd {
    Complete,
    Interrupted { diagnostic: String },
}

impl CaptureEnd {
    pub fn state(&self) -> CaptureState {
        match self {
            Self::Complete => CaptureState::Complete,
            Self::Interrupted { .. } => CaptureState::Interrupted,
        }
    }
    pub fn diagnostic(&self) -> Option<&str> {
        match self {
            Self::Complete => None,
            Self::Interrupted { diagnostic } => Some(diagnostic),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentStreamView {
    pub id: ContentId,
    pub entry: EntryId,
    pub epoch: ContentEpoch,
    pub kind: ContentKind,
    #[serde(default)]
    pub activity_id: Option<agena_domain::ActivityId>,
    pub ordinal: u32,
    pub accepted: u64,
    pub committed: u64,
    #[serde(default)]
    pub capture_loss: CaptureLossCounters,
    /// Logical prefix explicitly removed by retention. Positions and epoch in
    /// the remaining suffix keep their meaning.
    pub retained_start: u64,
    pub state: CaptureState,
    pub capture_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentRetention {
    pub stream: ContentStreamView,
    pub blocks_removed: u32,
    pub pending_reclamation: bool,
}

impl ContentStreamView {
    pub fn text_cursor(&self) -> Result<TextCursor> {
        if self.kind != ContentKind::Text {
            return Err(KernelError::Invalid(
                "text cursor requires a text stream".into(),
            ));
        }
        Ok(TextCursor {
            content: self.id,
            epoch: self.epoch,
            offset: self.accepted,
        })
    }
}

/// A bounded byte page. Ranges may split a UTF-8 code point; consumers decoding
/// a sequence of pages retain the incomplete suffix between pages.
#[derive(Debug, Clone)]
pub struct TextRange {
    pub stream: ContentStreamView,
    pub start: u64,
    pub bytes: Vec<u8>,
    pub next: TextCursor,
    pub has_more: bool,
}

/// Coalesced heads, with no per-packet replay allocation or subscriber queue.
/// Subscribe before reading ranges; the current head is captured atomically
/// with installation of the receiver. Readers recover any skipped heads by
/// requesting the indexed byte range.
pub struct ContentWatch {
    receiver: watch::Receiver<ContentStreamView>,
    _lease: Option<ReadLease>,
}

impl ContentWatch {
    pub fn with_lease(mut self, lease: ReadLease) -> Self {
        self._lease = Some(lease);
        self
    }

    /// Returned head snapshots are accounted like ranges. If admission is Busy,
    /// release older views and retry head(), including after changed() has
    /// consumed a coalesced notification.
    pub fn head(&self) -> Result<ReadView<ContentStreamView>> {
        let value = self.receiver.borrow().clone();
        self._lease.as_ref().ok_or(KernelError::Closed)?.wrap(value)
    }

    pub async fn changed(&mut self) -> Result<ReadView<ContentStreamView>> {
        self.receiver
            .changed()
            .await
            .map_err(|_| KernelError::Closed)?;
        let value = self.receiver.borrow_and_update().clone();
        self._lease.as_ref().ok_or(KernelError::Closed)?.wrap(value)
    }

    /// Infrastructure constructor; issuing this read capability grants no write
    /// authority. Backends must validate history membership before calling it.
    pub fn from_receiver(receiver: watch::Receiver<ContentStreamView>) -> Self {
        Self {
            receiver,
            _lease: None,
        }
    }
}

pub(super) struct Producer {
    pub inner: Arc<KernelInner>,
    pub scope: OutputScope,
    pub stream: ContentStreamView,
}

impl Producer {
    pub(super) fn record_loss(&self, bytes: u64, diagnostic: &str) -> Result<()> {
        self.inner.backend.record_capture_loss(
            self.scope,
            self.stream.id,
            self.stream.epoch,
            bytes,
            diagnostic,
        )
    }
    pub(super) fn new(
        inner: Arc<KernelInner>,
        scope: OutputScope,
        stream: ContentStreamView,
    ) -> Result<Arc<Self>> {
        inner.backend.retain_output(scope, &stream)?;
        Ok(Arc::new(Self {
            inner,
            scope,
            stream,
        }))
    }
    pub(super) async fn finish(
        &self,
        request: RequestId,
    ) -> Result<CommitReceipt<ContentStreamView>> {
        self.close(request, CaptureEnd::Complete).await
    }
    pub(super) async fn close(
        &self,
        request: RequestId,
        ending: CaptureEnd,
    ) -> Result<CommitReceipt<ContentStreamView>> {
        let receipt = self
            .inner
            .command(
                request,
                Command::SealContent {
                    scope: self.scope,
                    content: self.stream.id,
                    epoch: self.stream.epoch,
                    ending,
                },
            )
            .await?;
        match receipt.value {
            Mutation::Content(value) => Ok(CommitReceipt {
                position: receipt.position,
                value,
            }),
            _ => Err(unexpected()),
        }
    }
}

impl Drop for Producer {
    fn drop(&mut self) {
        // Drop merely signals abandonment. The writer supervisor owns the
        // asynchronous interrupted checkpoint, including any storage failure.
        self.inner
            .backend
            .release_output(self.scope, self.stream.id);
    }
}

#[derive(Clone)]
pub struct TextOutput(Arc<Producer>);

impl TextOutput {
    pub fn record_loss(&self, bytes: u64, diagnostic: &str) -> Result<()> {
        self.0.record_loss(bytes, diagnostic)
    }
    fn new(inner: Arc<KernelInner>, scope: OutputScope, stream: ContentStreamView) -> Result<Self> {
        Ok(Self(Producer::new(inner, scope, stream)?))
    }
    pub fn id(&self) -> ContentId {
        self.0.stream.id
    }
    pub fn entry_id(&self) -> EntryId {
        self.0.stream.entry
    }
    pub fn epoch(&self) -> ContentEpoch {
        self.0.stream.epoch
    }

    /// All bytes are admitted in FIFO order, or none are. Retry Busy only after
    /// capacity becomes available; an accepted append must never be repeated.
    pub async fn append(&self, text: &str) -> Result<TextCursor> {
        self.0
            .inner
            .backend
            .append_text(self.0.scope, self.id(), self.epoch(), text)
            .await
    }

    /// Synchronous, all-or-none memory admission. No database/queue wait is
    /// performed. On Busy the producer must keep draining its OS input and
    /// account for uncaptured bytes; it must not invent an accepted cursor.
    pub fn try_append(&self, text: &str) -> Result<TextCursor> {
        self.0
            .inner
            .backend
            .try_append_text(self.0.scope, self.id(), self.epoch(), text)
    }

    /// A committed capture boundary. Finishing its round/tool also includes
    /// this barrier, so ordinary producers need no separate flush call.
    pub async fn finish(&self, request: RequestId) -> Result<CommitReceipt<ContentStreamView>> {
        self.0.finish(request).await
    }
    pub async fn interrupt(
        &self,
        request: RequestId,
        diagnostic: String,
    ) -> Result<CommitReceipt<ContentStreamView>> {
        self.0
            .close(request, CaptureEnd::Interrupted { diagnostic })
            .await
    }
}

async fn begin_text(
    round: &RoundHandle,
    request: RequestId,
    tool: Option<ToolId>,
    reasoning: bool,
) -> Result<CommitReceipt<TextOutput>> {
    let inner = &round.attempt.inner;
    let scope = round.attempt.scope;
    let receipt = inner
        .command(
            request,
            Command::OpenText {
                scope,
                round: round.id,
                tool,
                reasoning,
            },
        )
        .await?;
    match receipt.value {
        Mutation::Content(stream) => Ok(CommitReceipt {
            position: receipt.position,
            value: TextOutput::new(inner.clone(), OutputScope::Attempt(scope), stream)?,
        }),
        _ => Err(unexpected()),
    }
}

impl RoundHandle {
    pub async fn begin_text(
        &self,
        request: RequestId,
        reasoning: bool,
    ) -> Result<CommitReceipt<TextOutput>> {
        begin_text(self, request, None, reasoning).await
    }
}

impl ToolHandle {
    pub async fn begin_text(&self, request: RequestId) -> Result<CommitReceipt<TextOutput>> {
        begin_text(&self.round, request, Some(self.id), false).await
    }
}

impl ActivityHandle {
    pub async fn begin_text(&self, request: RequestId) -> Result<CommitReceipt<TextOutput>> {
        let receipt = self.open_output(request, ContentKind::Text).await?;
        Ok(CommitReceipt {
            position: receipt.position,
            value: TextOutput(receipt.value),
        })
    }
}

impl KernelReader {
    pub async fn text_range(
        &self,
        session: SessionId,
        content: ContentId,
        after: Option<TextCursor>,
        limit: usize,
    ) -> Result<ReadView<TextRange>> {
        self.0
            .backend
            .text_range(session, content, after, limit)
            .await
    }

    pub async fn watch_content(
        &self,
        session: SessionId,
        content: ContentId,
    ) -> Result<ContentWatch> {
        self.0.backend.watch_content(session, content).await
    }
}
