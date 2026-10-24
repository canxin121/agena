//! Bounded content reads over membership-authorized kernel capabilities.

use crate::{Application, ApplicationError};
use agena_domain::{
    ContentChunk, ContentCursor, ContentEpoch, ContentId, ContentKind, ContentPage, ContentPayload,
    ContentRange, ContentResource, ContentState, ContentTextPage, ContentTextPosition,
    ContentTextSlice,
};
use agena_storage::kernel::{
    CaptureState, ContentStreamView, EventCursor, EventPayload, KernelError, KernelReader,
    TextCursor,
};

enum ContentReadError {
    Kernel(KernelError),
    Application(ApplicationError),
}

impl From<KernelError> for ContentReadError {
    fn from(error: KernelError) -> Self {
        Self::Kernel(error)
    }
}

impl From<ApplicationError> for ContentReadError {
    fn from(error: ApplicationError) -> Self {
        Self::Application(error)
    }
}

impl From<ContentReadError> for ApplicationError {
    fn from(error: ContentReadError) -> Self {
        match error {
            ContentReadError::Kernel(error) => crate::kernel_error(error),
            ContentReadError::Application(error) => error,
        }
    }
}

pub struct ContentWatch {
    pub resource: ContentResource,
    session: agena_storage::kernel::SessionId,
    reader: KernelReader,
    changes: agena_storage::kernel::ContentWatch,
}

/// A slow transport retains one source cursor and rereads bounded pages.
pub struct ContentDelivery {
    watch: ContentWatch,
    cursor: Option<ContentCursor>,
    max_bytes: usize,
    waiting: bool,
    closed: bool,
    admission_retries: u64,
}

impl ContentDelivery {
    /// Read-admission contention retried without advancing the source cursor.
    pub fn admission_retries(&self) -> u64 {
        self.admission_retries
    }

    pub async fn next(&mut self) -> Result<Option<ContentPage>, ApplicationError> {
        if self.closed {
            return Ok(None);
        }
        if self.waiting {
            match self.watch.changes.changed().await {
                Ok(_) => {}
                // changed() consumes the notification before charging its
                // returned head. Retry the read, never wait for a second change.
                Err(KernelError::Busy) => self.admission_retries += 1,
                Err(error) => return Err(crate::kernel_error(error)),
            }
            self.waiting = false;
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut backoff = std::time::Duration::from_millis(1);
        let page = loop {
            match self.watch.read_inner(self.cursor, self.max_bytes).await {
                Ok(page) => break page,
                Err(ContentReadError::Kernel(KernelError::Busy))
                    if tokio::time::Instant::now() < deadline =>
                {
                    self.admission_retries += 1;
                    // The failed read has dropped every read view/SQL permit.
                    // Keep one cursor while bounded native admission recovers.
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(std::time::Duration::from_millis(100));
                }
                Err(error) => return Err(error.into()),
            }
        };
        self.cursor = Some(page.next_cursor);
        self.waiting = !page.has_more && page.resource.state == ContentState::Active;
        self.closed = !page.has_more && page.resource.state != ContentState::Active;
        Ok(Some(page))
    }
}

impl ContentWatch {
    pub fn delivery(self, after: Option<ContentCursor>, max_bytes: usize) -> ContentDelivery {
        ContentDelivery {
            watch: self,
            cursor: after,
            max_bytes: max_bytes.clamp(4, 1024 * 1024),
            waiting: false,
            closed: false,
            admission_retries: 0,
        }
    }

    pub async fn read(
        &self,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, ApplicationError> {
        self.read_inner(after, max_bytes).await.map_err(Into::into)
    }

    async fn read_inner(
        &self,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, ContentReadError> {
        let head = self.changes.head()?;
        let max_bytes = max_bytes.clamp(4, 1024 * 1024);
        // Recovery creates a new epoch. An authoritative page lets live
        // reducers replace the old tail; fixed-generation copies detect it.
        let after = after.filter(|cursor| ContentEpoch::from(cursor.epoch) == head.epoch);
        if head.kind == ContentKind::Text {
            let cursor = after.map(|c| TextCursor {
                content: head.id,
                epoch: ContentEpoch::from(c.epoch),
                offset: c.sequence,
            });
            let mut gap = false;
            let mut result = self
                .reader
                .text_range(self.session, head.id, cursor, max_bytes)
                .await;
            for _ in 0..2 {
                match &result {
                    Err(agena_storage::kernel::KernelError::ContentGap {
                        retained_start, ..
                    }) => {
                        gap = true;
                        let epoch = self.changes.head()?.epoch;
                        result = self
                            .reader
                            .text_range(
                                self.session,
                                head.id,
                                Some(TextCursor {
                                    content: head.id,
                                    epoch,
                                    offset: *retained_start,
                                }),
                                max_bytes,
                            )
                            .await;
                    }
                    Err(agena_storage::kernel::KernelError::ContentReset)
                        if cursor.is_some_and(|cursor| {
                            self.changes
                                .head()
                                .is_ok_and(|head| head.epoch != cursor.epoch)
                        }) =>
                    {
                        result = self
                            .reader
                            .text_range(self.session, head.id, None, max_bytes)
                            .await;
                    }
                    _ => break,
                }
            }
            let page = result?;
            // Prefix retention may cut a UTF-8 scalar. Its remaining bytes
            // belong to the declared gap, never to a replacement character.
            let skipped = if gap {
                page.bytes
                    .iter()
                    .take_while(|byte| (**byte & 0xc0) == 0x80)
                    .count()
            } else {
                0
            };
            let body = &page.bytes[skipped..];
            // UTF-8 pages can split a scalar. Return only its complete prefix;
            // the next read starts at the same incomplete scalar, never drops it.
            let length = match std::str::from_utf8(body) {
                Ok(_) => body.len(),
                Err(error) if error.error_len().is_none() => error.valid_up_to(),
                Err(error) => return Err(ApplicationError::internal_error(&error).into()),
            };
            let text = String::from_utf8(body[..length].to_vec())
                .map_err(|error| ApplicationError::internal_error(&error))?;
            let next = ContentCursor {
                epoch: page.stream.epoch.into(),
                sequence: page.start.saturating_add((skipped + length) as u64),
            };
            let chunks = if text.is_empty() {
                Vec::new()
            } else {
                vec![ContentChunk {
                    cursor: next,
                    captured_at_ms: 0,
                    payload: ContentPayload::Text { text },
                }]
            };
            return Ok(ContentPage {
                resource: resource(self.session.get(), &page.stream),
                chunks,
                next_cursor: next,
                has_more: page.has_more || length < body.len(),
                gap,
            });
        }
        let cursor = after.map(|c| EventCursor {
            content: head.id,
            epoch: ContentEpoch::from(c.epoch),
            sequence: c.sequence,
        });
        let mut gap = false;
        let mut result = self
            .reader
            .event_page(self.session, head.id, cursor, 64)
            .await;
        for _ in 0..2 {
            match &result {
                Err(agena_storage::kernel::KernelError::ContentGap { retained_start, .. }) => {
                    gap = true;
                    let epoch = self.changes.head()?.epoch;
                    result = self
                        .reader
                        .event_page(
                            self.session,
                            head.id,
                            Some(EventCursor {
                                content: head.id,
                                epoch,
                                sequence: *retained_start,
                            }),
                            64,
                        )
                        .await;
                }
                Err(agena_storage::kernel::KernelError::ContentReset)
                    if cursor.is_some_and(|cursor| {
                        self.changes
                            .head()
                            .is_ok_and(|head| head.epoch != cursor.epoch)
                    }) =>
                {
                    result = self
                        .reader
                        .event_page(self.session, head.id, None, 64)
                        .await;
                }
                _ => break,
            }
        }
        let page = result?;
        let mut chunks = Vec::new();
        let mut bytes = 0usize;
        let mut next = after
            .filter(|cursor| ContentEpoch::from(cursor.epoch) == page.stream.epoch)
            .map(|cursor| ContentCursor {
                sequence: cursor.sequence.max(page.stream.retained_start),
                ..cursor
            })
            .unwrap_or(ContentCursor {
                epoch: page.stream.epoch.into(),
                sequence: page.stream.retained_start,
            });
        for record in &page.records {
            let payload = match &record.payload {
                EventPayload::Mixed { input, checkpoint } => ContentPayload::DocumentEvent {
                    input: input.clone(),
                    checkpoint: checkpoint.clone(),
                },
                EventPayload::Log { channel, text } => ContentPayload::Log {
                    stream: channel.clone(),
                    text: text.clone(),
                },
                EventPayload::DocumentCheckpoint { document } => {
                    ContentPayload::StructuredSnapshot {
                        document: (**document).clone(),
                    }
                }
                EventPayload::DocumentPatch { base, mutation } => ContentPayload::Structured {
                    base_cursor: ContentCursor {
                        epoch: page.stream.epoch.into(),
                        sequence: *base,
                    },
                    event: (**mutation).clone(),
                },
                EventPayload::TerminalCheckpoint { screen } => ContentPayload::Terminal {
                    screen: (**screen).clone(),
                },
                EventPayload::TerminalPatch {
                    base,
                    screen,
                    rows_changed,
                } => ContentPayload::TerminalPatch {
                    base_cursor: ContentCursor {
                        epoch: page.stream.epoch.into(),
                        sequence: *base,
                    },
                    screen: (**screen).clone(),
                    rows_changed: rows_changed.clone(),
                },
            };
            let size = payload.byte_len();
            // An atomic checkpoint can exceed an ordinary page budget.
            if !chunks.is_empty() && bytes.saturating_add(size) > max_bytes {
                break;
            }
            bytes = bytes.saturating_add(size);
            next = ContentCursor {
                epoch: page.stream.epoch.into(),
                sequence: record.sequence,
            };
            chunks.push(ContentChunk {
                cursor: next,
                captured_at_ms: record.timestamp_ms,
                payload,
            });
        }
        Ok(ContentPage {
            resource: resource(self.session.get(), &page.stream),
            has_more: page.has_more || chunks.len() < page.records.len(),
            chunks,
            next_cursor: next,
            gap,
        })
    }
}

impl Application {
    pub async fn read_content_text(
        &self,
        params: agena_api::content::ReadContentTextParams,
    ) -> Result<ContentTextPage, ApplicationError> {
        let watch = self
            .watch_content(params.session_id, params.resource_id)
            .await?;
        let after = params.position.map(|p| ContentCursor {
            sequence: p.after.sequence.saturating_add(
                if watch.resource.kind == ContentKind::Text {
                    p.offset as u64
                } else {
                    0
                },
            ),
            ..p.after
        });
        let page = watch.read(after, params.max_bytes).await?;
        let mut slices = Vec::new();
        let first = page.chunks.first().map(|chunk| ContentCursor {
            epoch: chunk.cursor.epoch,
            sequence: chunk.cursor.sequence.saturating_sub(1),
        });
        let mut next = ContentTextPosition {
            after: if watch.resource.kind == ContentKind::Text {
                after
                    .filter(|cursor| cursor.epoch == page.resource.cursor.epoch)
                    .unwrap_or(ContentCursor {
                        epoch: page.resource.cursor.epoch,
                        sequence: 0,
                    })
            } else {
                first.unwrap_or(page.next_cursor)
            },
            offset: 0,
        };
        let mut bytes = 0usize;
        let budget = params.max_bytes.clamp(4, 1024 * 1024);
        for (index, chunk) in page.chunks.iter().enumerate() {
            let (stream, text) = match &chunk.payload {
                ContentPayload::Text { text } => (None, Some(text.as_str())),
                ContentPayload::Log { stream, text } => (Some(stream.clone()), Some(text.as_str())),
                ContentPayload::DocumentEvent { input, .. } => match &**input {
                    agena_domain::ContentInput::Text { text } => (None, Some(text.as_str())),
                    agena_domain::ContentInput::Log { stream, text } => {
                        (Some(stream.clone()), Some(text.as_str()))
                    }
                    _ => (None, None),
                },
                _ => (None, None),
            };
            if let Some(text) = text {
                let start = if watch.resource.kind != ContentKind::Text && index == 0 {
                    params
                        .position
                        .filter(|p| {
                            p.after.epoch == chunk.cursor.epoch
                                && p.after.sequence.saturating_add(1) == chunk.cursor.sequence
                        })
                        .map_or(0, |p| p.offset)
                        .min(text.len())
                } else {
                    0
                };
                if !text.is_char_boundary(start) {
                    return Err(ApplicationError::bad_request(
                        "The text offset is not a UTF-8 boundary.",
                    ));
                }
                let mut end = text
                    .len()
                    .min(start.saturating_add(budget.saturating_sub(bytes)));
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                if end == start && start < text.len() {
                    return Ok(ContentTextPage {
                        resource: page.resource,
                        slices,
                        next_position: next,
                        has_more: true,
                        gap: page.gap,
                    });
                }
                slices.push(ContentTextSlice {
                    cursor: chunk.cursor,
                    captured_at_ms: chunk.captured_at_ms,
                    offset: start,
                    stream,
                    text: text[start..end].to_owned(),
                });
                bytes += end - start;
                if end < text.len() {
                    next.offset = end;
                    return Ok(ContentTextPage {
                        resource: page.resource,
                        slices,
                        next_position: next,
                        has_more: true,
                        gap: page.gap,
                    });
                }
            }
            next = ContentTextPosition {
                after: chunk.cursor,
                offset: 0,
            };
        }
        if page.chunks.is_empty() {
            next.after = page.next_cursor;
        }
        Ok(ContentTextPage {
            resource: page.resource,
            slices,
            next_position: next,
            has_more: page.has_more,
            gap: page.gap,
        })
    }

    pub async fn read_content(
        &self,
        session_id: i64,
        id: ContentId,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, ApplicationError> {
        self.watch_content(session_id, id)
            .await?
            .read(after, max_bytes)
            .await
    }

    pub async fn watch_content(
        &self,
        session_id: i64,
        id: ContentId,
    ) -> Result<ContentWatch, ApplicationError> {
        let session = crate::kernel_session_id(session_id)?;
        let reader = self.session_kernel()?.reader.clone();
        // Membership is checked by the backend on bootstrap and every range;
        // a rewind/deletion revokes retained watch capabilities immediately.
        let changes = reader
            .watch_content(session, id)
            .await
            .map_err(crate::kernel_error)?;
        let head = changes.head().map_err(crate::kernel_error)?;
        let resource = resource(session_id, &head);
        Ok(ContentWatch {
            resource,
            session,
            reader,
            changes,
        })
    }
}

fn resource(session_id: i64, head: &ContentStreamView) -> ContentResource {
    ContentResource {
        resource_id: head.id,
        owner_session_id: session_id,
        part_id: head.entry.get(),
        kind: head.kind,
        state: match head.state {
            CaptureState::Open => ContentState::Active,
            CaptureState::Complete => ContentState::Complete,
            CaptureState::Interrupted => ContentState::Interrupted,
        },
        cursor: ContentCursor {
            epoch: head.epoch.into(),
            sequence: head.accepted,
        },
        committed_cursor: ContentCursor {
            epoch: head.epoch.into(),
            sequence: head.committed,
        },
        total_bytes: (head.kind == ContentKind::Text).then_some(head.accepted),
        dropped_bytes: head.capture_loss.bytes,
        dropped_packets: head.capture_loss.packets,
        committed_dropped_bytes: head.capture_loss.committed_bytes,
        committed_dropped_packets: head.capture_loss.committed_packets,
        retained_ranges: if head.accepted > head.retained_start {
            vec![ContentRange {
                first: head.retained_start.saturating_add(1),
                last: head.accepted,
            }]
        } else {
            Vec::new()
        },
        capture_error: head.capture_error.clone(),
    }
}
