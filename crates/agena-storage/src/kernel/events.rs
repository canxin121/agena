//! Event values and bounded projection rules shared by admission and replay.
//! Cursor identity belongs to the stream, so block records store only local
//! sequence numbers rather than repeating content/session/epoch envelopes.

use agena_domain::{CommandOutputStream, ContentDocument, DocumentMutation, TerminalSnapshot};
use serde::{Deserialize, Serialize};

use super::output::Producer;
use super::*;
use super::{KernelError, Result};
use crate::backend::kernel::EventInput;
use crate::backend::kernel::OutputScope;
use agena_domain::{ContentEpoch, ContentId};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventCursor {
    pub content: ContentId,
    pub epoch: ContentEpoch,
    pub sequence: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventPage {
    pub stream: ContentStreamView,
    pub records: Vec<EventRecord>,
    pub next: EventCursor,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventSnapshot {
    pub stream: ContentStreamView,
    pub through: EventCursor,
    pub document: Option<Box<ContentDocument>>,
    pub terminal: Option<Box<TerminalSnapshot>>,
}

impl ContentStreamView {
    pub fn event_cursor(&self) -> Result<EventCursor> {
        if self.kind == agena_domain::ContentKind::Text {
            return Err(KernelError::Invalid(
                "event cursor requires an event stream".into(),
            ));
        }
        Ok(EventCursor {
            content: self.id,
            epoch: self.epoch,
            sequence: self.accepted,
        })
    }
}

macro_rules! event_output {
    ($name:ident,$open:ident,$kind:ident) => {
        #[derive(Clone)]
        pub struct $name(Arc<Producer>);
        impl $name {
            pub fn record_loss(&self, bytes: u64, diagnostic: &str) -> Result<()> {
                self.0.record_loss(bytes, diagnostic)
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
            pub async fn finish(
                &self,
                request: RequestId,
            ) -> Result<CommitReceipt<ContentStreamView>> {
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
        impl ToolHandle {
            pub async fn $open(&self, request: RequestId) -> Result<CommitReceipt<$name>> {
                let scope = self.round.attempt.scope;
                let inner = &self.round.attempt.inner;
                let receipt = inner
                    .command(
                        request,
                        Command::OpenEvents {
                            scope,
                            round: self.round.id,
                            tool: self.id,
                            kind: EventKind::$kind,
                        },
                    )
                    .await?;
                match receipt.value {
                    Mutation::Content(stream) => Ok(CommitReceipt {
                        position: receipt.position,
                        value: $name(Producer::new(
                            inner.clone(),
                            OutputScope::Attempt(scope),
                            stream,
                        )?),
                    }),
                    _ => Err(unexpected()),
                }
            }
        }
        impl ActivityHandle {
            pub async fn $open(&self, request: RequestId) -> Result<CommitReceipt<$name>> {
                let kind = match EventKind::$kind {
                    EventKind::Log => agena_domain::ContentKind::Log,
                    EventKind::Mixed => agena_domain::ContentKind::Document,
                    EventKind::Structured => agena_domain::ContentKind::Structured,
                    EventKind::Terminal => agena_domain::ContentKind::Terminal,
                };
                let receipt = self.open_output(request, kind).await?;
                Ok(CommitReceipt {
                    position: receipt.position,
                    value: $name(receipt.value),
                })
            }
        }
    };
}
event_output!(LogOutput, begin_log, Log);
event_output!(MixedOutput, begin_mixed, Mixed);

impl MixedOutput {
    pub fn try_append(&self, input: agena_domain::ContentInput) -> Result<EventCursor> {
        self.0.inner.backend.try_append_event(
            self.0.scope,
            self.id(),
            self.epoch(),
            None,
            EventInput::Mixed(Box::new(input)),
        )
    }
    pub async fn append(&self, input: agena_domain::ContentInput) -> Result<EventCursor> {
        self.0
            .inner
            .backend
            .append_event(
                self.0.scope,
                self.id(),
                self.epoch(),
                None,
                EventInput::Mixed(Box::new(input)),
            )
            .await
    }
}

event_output!(DocumentOutput, begin_document, Structured);
event_output!(TerminalOutput, begin_terminal, Terminal);

impl LogOutput {
    pub fn try_append(&self, channel: CommandOutputStream, text: &str) -> Result<EventCursor> {
        self.try_append_at_inner(None, channel, text)
    }
    pub fn try_append_at(
        &self,
        timestamp_ms: i64,
        channel: CommandOutputStream,
        text: &str,
    ) -> Result<EventCursor> {
        self.try_append_at_inner(Some(timestamp_ms), channel, text)
    }
    fn try_append_at_inner(
        &self,
        timestamp_ms: Option<i64>,
        channel: CommandOutputStream,
        text: &str,
    ) -> Result<EventCursor> {
        if text.len() > 256 * 1024 {
            return Err(KernelError::Invalid(
                "log packet exceeds 256 KiB; stream bounded chunks".into(),
            ));
        }
        self.0.inner.backend.try_append_event(
            self.0.scope,
            self.id(),
            self.epoch(),
            timestamp_ms,
            EventInput::Log {
                channel,
                text: text.into(),
            },
        )
    }
    pub async fn append(&self, channel: CommandOutputStream, text: &str) -> Result<EventCursor> {
        self.append_inner(None, channel, text).await
    }
    pub async fn append_at(
        &self,
        timestamp_ms: i64,
        channel: CommandOutputStream,
        text: &str,
    ) -> Result<EventCursor> {
        self.append_inner(Some(timestamp_ms), channel, text).await
    }
    async fn append_inner(
        &self,
        time: Option<i64>,
        channel: CommandOutputStream,
        text: &str,
    ) -> Result<EventCursor> {
        if text.len() > 256 * 1024 {
            return Err(KernelError::Invalid(
                "log packet exceeds 256 KiB; stream bounded chunks".into(),
            ));
        }
        self.0
            .inner
            .backend
            .append_event(
                self.0.scope,
                self.id(),
                self.epoch(),
                time,
                EventInput::Log {
                    channel,
                    text: text.into(),
                },
            )
            .await
    }
}
impl DocumentOutput {
    pub fn try_apply(&self, mutation: DocumentMutation) -> Result<EventCursor> {
        self.0.inner.backend.try_append_event(
            self.0.scope,
            self.id(),
            self.epoch(),
            None,
            EventInput::Document(Box::new(mutation)),
        )
    }
    pub async fn apply(&self, mutation: DocumentMutation) -> Result<EventCursor> {
        self.0
            .inner
            .backend
            .append_event(
                self.0.scope,
                self.id(),
                self.epoch(),
                None,
                EventInput::Document(Box::new(mutation)),
            )
            .await
    }
}
impl TerminalOutput {
    pub fn try_snapshot(&self, screen: TerminalSnapshot) -> Result<EventCursor> {
        self.0.inner.backend.try_append_event(
            self.0.scope,
            self.id(),
            self.epoch(),
            None,
            EventInput::Terminal(Box::new(screen)),
        )
    }
    pub fn try_patch(
        &self,
        base: EventCursor,
        screen: TerminalSnapshot,
        rows_changed: Vec<u16>,
    ) -> Result<EventCursor> {
        self.0.inner.backend.try_append_event(
            self.0.scope,
            self.id(),
            self.epoch(),
            None,
            EventInput::TerminalPatch {
                base,
                screen: Box::new(screen),
                rows_changed,
            },
        )
    }
    pub async fn snapshot(&self, screen: TerminalSnapshot) -> Result<EventCursor> {
        self.0
            .inner
            .backend
            .append_event(
                self.0.scope,
                self.id(),
                self.epoch(),
                None,
                EventInput::Terminal(Box::new(screen)),
            )
            .await
    }
    pub async fn patch(
        &self,
        base: EventCursor,
        screen: TerminalSnapshot,
        rows_changed: Vec<u16>,
    ) -> Result<EventCursor> {
        self.0
            .inner
            .backend
            .append_event(
                self.0.scope,
                self.id(),
                self.epoch(),
                None,
                EventInput::TerminalPatch {
                    base,
                    screen: Box::new(screen),
                    rows_changed,
                },
            )
            .await
    }
}
impl KernelReader {
    pub async fn event_page(
        &self,
        session: SessionId,
        content: ContentId,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<ReadView<EventPage>> {
        self.0
            .backend
            .event_page(session, content, after, limit)
            .await
    }
    pub async fn event_snapshot(
        &self,
        session: SessionId,
        content: ContentId,
    ) -> Result<ReadView<EventSnapshot>> {
        self.0.backend.event_snapshot(session, content).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Log,
    Mixed,
    Structured,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventPayload {
    Mixed {
        input: Box<agena_domain::ContentInput>,
        checkpoint: Option<Box<ContentDocument>>,
    },
    Log {
        channel: CommandOutputStream,
        text: String,
    },
    DocumentCheckpoint {
        document: Box<ContentDocument>,
    },
    DocumentPatch {
        base: u64,
        mutation: Box<DocumentMutation>,
    },
    TerminalCheckpoint {
        screen: Box<TerminalSnapshot>,
    },
    TerminalPatch {
        base: u64,
        screen: Box<TerminalSnapshot>,
        rows_changed: Vec<u16>,
    },
}

impl EventPayload {
    pub fn kind(&self) -> EventKind {
        match self {
            Self::Mixed { .. } => EventKind::Mixed,
            Self::Log { .. } => EventKind::Log,
            Self::DocumentCheckpoint { .. } | Self::DocumentPatch { .. } => EventKind::Structured,
            Self::TerminalCheckpoint { .. } | Self::TerminalPatch { .. } => EventKind::Terminal,
        }
    }
    pub fn validate(&self) -> Result<()> {
        let result = match self {
            Self::Mixed { input, checkpoint } => {
                match &**input {
                    agena_domain::ContentInput::Text { .. }
                    | agena_domain::ContentInput::Log { .. }
                    | agena_domain::ContentInput::Structured { .. } => input
                        .validate()
                        .map_err(|e| KernelError::Invalid(e.into()))?,
                    _ => {
                        return Err(KernelError::Invalid(
                            "mixed document streams accept text, log and document mutations".into(),
                        ));
                    }
                }
                checkpoint
                    .as_ref()
                    .map_or(Ok(()), |document| document.validate())
            }
            Self::Log { .. } => Ok(()),
            Self::DocumentCheckpoint { document } => document.validate(),
            Self::DocumentPatch { base, mutation } => {
                if *base == 0 {
                    Err("document patch has no checkpoint")
                } else {
                    mutation.validate()
                }
            }
            Self::TerminalCheckpoint { screen } => screen.validate(None),
            Self::TerminalPatch {
                base,
                screen,
                rows_changed,
            } => {
                if *base == 0 {
                    Err("terminal patch has no checkpoint")
                } else {
                    screen.validate(Some(rows_changed))
                }
            }
        };
        result.map_err(|e| KernelError::Invalid(e.into()))
    }
    pub fn is_checkpoint(&self) -> bool {
        matches!(
            self,
            Self::DocumentCheckpoint { .. }
                | Self::TerminalCheckpoint { .. }
                | Self::Mixed {
                    checkpoint: Some(_),
                    ..
                }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRecord {
    pub sequence: u64,
    pub timestamp_ms: i64,
    pub payload: EventPayload,
}

#[derive(Clone)]
enum ProjectionValue {
    Log,
    Mixed(Arc<ContentDocument>),
    Document(Option<Arc<ContentDocument>>),
    Terminal(Option<Arc<TerminalSnapshot>>),
}

/// A rejected mutation leaves this value untouched. Admission and readers use
/// the same base/geometry checks, including patch chains following a restored
/// checkpoint. The domain types bound document size and terminal geometry.
#[derive(Clone)]
pub struct EventProjection {
    sequence: u64,
    value: ProjectionValue,
}

impl EventProjection {
    pub fn new(kind: EventKind) -> Self {
        Self {
            sequence: 0,
            value: match kind {
                EventKind::Log => ProjectionValue::Log,
                EventKind::Mixed => ProjectionValue::Mixed(Arc::default()),
                EventKind::Structured => ProjectionValue::Document(None),
                EventKind::Terminal => ProjectionValue::Terminal(None),
            },
        }
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn document(&self) -> Option<&ContentDocument> {
        match &self.value {
            ProjectionValue::Document(value) => value.as_deref(),
            ProjectionValue::Mixed(value) => Some(value),
            _ => None,
        }
    }
    pub fn terminal(&self) -> Option<&TerminalSnapshot> {
        match &self.value {
            ProjectionValue::Terminal(value) => value.as_deref(),
            _ => None,
        }
    }

    pub fn from_checkpoint(record: &EventRecord) -> Result<Self> {
        if !record.payload.is_checkpoint() || record.sequence == 0 {
            return Err(KernelError::Invalid(
                "replay requires a complete checkpoint".into(),
            ));
        }
        let mut projection = Self::new(record.payload.kind());
        projection.sequence = record.sequence - 1;
        projection.apply(record)?;
        Ok(projection)
    }

    pub fn apply(&mut self, record: &EventRecord) -> Result<()> {
        record.payload.validate()?;
        if self.sequence.checked_add(1) != Some(record.sequence)
            || record.sequence > i64::MAX as u64
        {
            return Err(KernelError::Conflict(
                "event sequence has a gap or exceeds its range".into(),
            ));
        }
        let next = match (&self.value, &record.payload) {
            (ProjectionValue::Log, EventPayload::Log { .. }) => ProjectionValue::Log,
            (ProjectionValue::Mixed(document), EventPayload::Mixed { input, checkpoint }) => {
                let document = if let Some(checkpoint) = checkpoint {
                    Arc::from(checkpoint.clone())
                } else if let agena_domain::ContentInput::Structured { event } = &**input {
                    Arc::new(
                        document
                            .updated(event)
                            .map_err(|e| KernelError::Invalid(e.into()))?,
                    )
                } else {
                    document.clone()
                };
                ProjectionValue::Mixed(document)
            }
            (ProjectionValue::Document(_), EventPayload::DocumentCheckpoint { document }) => {
                ProjectionValue::Document(Some(Arc::from(document.clone())))
            }
            (
                ProjectionValue::Document(Some(document)),
                EventPayload::DocumentPatch { base, mutation },
            ) if *base == self.sequence => ProjectionValue::Document(Some(Arc::new(
                document
                    .updated(mutation)
                    .map_err(|e| KernelError::Invalid(e.into()))?,
            ))),
            (ProjectionValue::Terminal(_), EventPayload::TerminalCheckpoint { screen }) => {
                ProjectionValue::Terminal(Some(Arc::from(screen.clone())))
            }
            (
                ProjectionValue::Terminal(Some(previous)),
                EventPayload::TerminalPatch {
                    base,
                    screen,
                    rows_changed,
                },
            ) if *base == self.sequence
                && previous.rows == screen.rows
                && previous.cols == screen.cols
                && previous.alternate_screen == screen.alternate_screen =>
            {
                let mut next = screen.clone();
                next.cells.extend(
                    previous
                        .cells
                        .iter()
                        .filter(|cell| rows_changed.binary_search(&cell.row).is_err())
                        .cloned(),
                );
                next.cells.sort_by_key(|cell| (cell.row, cell.col));
                next.validate(None)
                    .map_err(|e| KernelError::Invalid(e.into()))?;
                ProjectionValue::Terminal(Some(Arc::from(next)))
            }
            _ => {
                return Err(KernelError::Conflict(
                    "event kind, checkpoint, base or terminal geometry does not match".into(),
                ));
            }
        };
        self.value = next;
        self.sequence = record.sequence;
        Ok(())
    }

    pub fn snapshot(&self, stream: ContentStreamView) -> Result<EventSnapshot> {
        let document = match &self.value {
            ProjectionValue::Document(value) => value.as_deref().cloned().map(Box::new),
            ProjectionValue::Mixed(value) => Some(Box::new((**value).clone())),
            _ => None,
        };
        let terminal = match &self.value {
            ProjectionValue::Terminal(value) => value.as_deref().cloned().map(Box::new),
            _ => None,
        };
        // Opening a terminal precedes launching the PTY and capturing its first
        // screen. A frozen prompt can legitimately observe that empty source.
        // Its optional screen is absent, rather than a missing checkpoint.
        if document.is_none()
            && terminal.is_none()
            && !matches!(&self.value, ProjectionValue::Terminal(None) if self.sequence == 0)
        {
            return Err(KernelError::NotFound("event checkpoint".into()));
        }
        Ok(EventSnapshot {
            through: EventCursor {
                content: stream.id,
                epoch: stream.epoch,
                sequence: self.sequence,
            },
            stream,
            document,
            terminal,
        })
    }
}
