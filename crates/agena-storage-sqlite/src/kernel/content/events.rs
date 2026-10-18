//! Event admission shares the text writer's FIFO, live registry, checkpoints
//! and terminal barriers. Ordinary log packets use compact binary records;
//! semantic documents and PTY screens carry bounded checkpoint/patch chains.

use agena_domain::{ContentDocument, ContentKind, MAX_DOCUMENT_BYTES, MAX_TERMINAL_FRAME_BYTES};
use agena_storage::{
    backend::{
        content_events::{EncodedEvent, MAX_EVENT_PAYLOAD_BYTES},
        kernel::EventInput,
    },
    kernel::*,
};

use super::*;

const MAX_EVENT_PAGE_BYTES: usize = 1024 * 1024;
const MAX_EVENT_PAGE_RECORDS: usize = 256;
const MAX_TERMINAL_SOURCE_BYTES: usize = 2 * (MAX_TERMINAL_FRAME_BYTES + 1024);
const DOCUMENT_RESERVATION: usize = 4 * (MAX_DOCUMENT_BYTES + 1024) + 4096;
const TERMINAL_RESERVATION: usize = 4 * (MAX_TERMINAL_FRAME_BYTES + 1024) + 4096;
pub(crate) const FRAME_BUDGET_BYTES: usize = 2 * TERMINAL_RESERVATION;

struct EventPageCapture {
    head: ContentStreamView,
    tail: Vec<ImmutableEventBlock>,
    large_read: Option<OwnedSemaphorePermit>,
}

pub(crate) struct AppendEvent {
    pub scope: OutputScope,
    pub content: ContentId,
    pub epoch: ContentEpoch,
    timestamp_ms: i64,
    input: EventInput,
    pub response: oneshot::Sender<Result<EventCursor>>,
    permit: OwnedSemaphorePermit,
    _queue_slot: OwnedSemaphorePermit,
}

fn encoded_size<T: serde::Serialize>(value: &T, limit: usize) -> Result<usize> {
    struct Count {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit - self.bytes {
                return Err(std::io::Error::other("event input exceeds its byte budget"));
            }
            self.bytes += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count { bytes: 0, limit };
    serde_json::to_writer(&mut count, value).map_err(invalid)?;
    Ok(count.bytes)
}

fn prepare(
    backend: &Backend,
    scope: OutputScope,
    content: ContentId,
    epoch: ContentEpoch,
    timestamp_ms: Option<i64>,
    input: EventInput,
) -> Result<(AppendEvent, oneshot::Receiver<Result<EventCursor>>)> {
    let (budget, charge) = match &input {
        EventInput::Mixed(input) => {
            input.validate().map_err(invalid)?;
            if !matches!(
                &**input,
                agena_domain::ContentInput::Text { .. }
                    | agena_domain::ContentInput::Log { .. }
                    | agena_domain::ContentInput::Structured { .. }
            ) {
                return Err(invalid(
                    "mixed streams accept text, log and structured input",
                ));
            }
            encoded_size(input, MAX_APPEND_BYTES)?;
            (
                &backend.content_budget,
                DOCUMENT_RESERVATION + 2 * input.byte_len(),
            )
        }
        EventInput::Log { text, .. } => {
            if text.len() > MAX_APPEND_BYTES {
                return Err(invalid("log packet exceeds 256 KiB"));
            }
            (&backend.content_budget, 2 * text.len() + 1024)
        }
        EventInput::Document(mutation) => {
            mutation.validate().map_err(invalid)?;
            encoded_size(mutation, MAX_DOCUMENT_BYTES + 1024)?;
            (&backend.content_budget, DOCUMENT_RESERVATION)
        }
        EventInput::Terminal(screen) | EventInput::TerminalPatch { screen, .. } => {
            let rows = match &input {
                EventInput::TerminalPatch { rows_changed, .. } => Some(rows_changed.as_slice()),
                _ => None,
            };
            screen.validate(rows).map_err(invalid)?;
            encoded_size(screen, MAX_TERMINAL_FRAME_BYTES)?;
            (&backend.frame_budget, TERMINAL_RESERVATION)
        }
    };
    let permit = budget
        .clone()
        .try_acquire_many_owned(charge as u32)
        .map_err(|_| KernelError::Busy)?;
    let slot = backend
        .content_queue_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let (response, receive) = oneshot::channel();
    let pending = AppendEvent {
        scope,
        content,
        epoch,
        timestamp_ms: timestamp_ms.unwrap_or_else(now_ms),
        input,
        response,
        permit,
        _queue_slot: slot,
    };
    Ok((pending, receive))
}

pub(crate) async fn admit(
    backend: &Backend,
    scope: OutputScope,
    content: ContentId,
    epoch: ContentEpoch,
    timestamp_ms: Option<i64>,
    input: EventInput,
) -> Result<EventCursor> {
    let (pending, receive) = prepare(backend, scope, content, epoch, timestamp_ms, input)?;
    backend
        .sender
        .try_send(Work::AppendEvent(Box::new(pending)))
        .map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => KernelError::Busy,
            mpsc::error::TrySendError::Closed(_) => KernelError::Closed,
        })?;
    receive.await.map_err(|_| KernelError::Closed)?
}

pub(crate) fn try_admit(
    backend: &Backend,
    scope: OutputScope,
    content: ContentId,
    epoch: ContentEpoch,
    timestamp_ms: Option<i64>,
    input: EventInput,
) -> Result<EventCursor> {
    backend.cache.ensure_open()?;
    let (pending, mut receive) = prepare(backend, scope, content, epoch, timestamp_ms, input)?;
    if backend.live.try_append_event(pending)?.is_some() {
        return Err(KernelError::Conflict(
            "content capture is closed or its lease was revoked".into(),
        ));
    }
    receive.try_recv().map_err(|_| KernelError::Closed)?
}

impl LiveContent {
    pub(crate) fn append_event(&self, append: AppendEvent) -> Option<AppendEvent> {
        self.append_event_inner(append, false)
            .expect("blocking memory lock cannot be busy")
    }
    pub(crate) fn try_append_event(&self, append: AppendEvent) -> Result<Option<AppendEvent>> {
        self.append_event_inner(append, true)
    }
    fn append_event_inner(
        &self,
        mut append: AppendEvent,
        synchronous: bool,
    ) -> Result<Option<AppendEvent>> {
        let mut registry = self.admission_lock(synchronous)?;
        let Some(source) = registry.sources.get_mut(&append.content) else {
            return Ok(Some(append));
        };
        let result = (|| {
            source.scope.check(append.scope)?;
            if source.head.epoch != append.epoch {
                return Err(KernelError::Ownership);
            }
            if source.abandoned || source.head.state != CaptureState::Open {
                return Err(KernelError::Conflict("capture is closed".into()));
            }
            if source.terminal_gate.is_some() {
                return Err(KernelError::Busy);
            }
            if source.losses.len() == MAX_PENDING_LOSS_WINDOWS {
                return Err(KernelError::Busy);
            }
            let Buffer::Events {
                assembler,
                blocks,
                projection,
                projection_permit,
            } = &mut source.buffer
            else {
                return Err(invalid("event append requires an event stream"));
            };
            let sequence = source
                .head
                .accepted
                .checked_add(1)
                .filter(|seq| *seq <= i64::MAX as u64)
                .ok_or_else(|| invalid("event sequence overflow"))?;
            let input = std::mem::replace(
                &mut append.input,
                EventInput::Log {
                    channel: agena_domain::CommandOutputStream::Stdout,
                    text: String::new(),
                },
            );
            let payload = match input {
                EventInput::Mixed(input) => EventPayload::Mixed {
                    input,
                    checkpoint: None,
                },
                EventInput::Log { channel, text } => EventPayload::Log { channel, text },
                EventInput::Document(mutation) => EventPayload::DocumentPatch {
                    base: source.head.accepted,
                    mutation,
                },
                EventInput::Terminal(screen) => EventPayload::TerminalCheckpoint { screen },
                EventInput::TerminalPatch {
                    base,
                    screen,
                    rows_changed,
                } => {
                    if base.content != source.head.id || base.epoch != source.head.epoch {
                        return Err(KernelError::ContentReset);
                    }
                    EventPayload::TerminalPatch {
                        base: base.sequence,
                        screen,
                        rows_changed,
                    }
                }
            };
            let mut record = EventRecord {
                sequence,
                timestamp_ms: append.timestamp_ms,
                payload,
            };
            let mut next = projection.clone();
            next.apply(&record)?;
            // Replace a patch with an equivalent full checkpoint at least
            // every 64 events. Its sequence/timestamp and resulting view stay
            // the same, and indexed replay never needs an unbounded chain.
            if let EventPayload::Mixed { checkpoint, .. } = &mut record.payload
                && (sequence == 1 || sequence % 64 == 0)
            {
                *checkpoint = next.document().cloned().map(Box::new);
            }
            if sequence % 64 == 0
                && !record.payload.is_checkpoint()
                && source.head.kind != ContentKind::Document
            {
                if let Some(document) = next.document() {
                    record.payload = EventPayload::DocumentCheckpoint {
                        document: Box::new(document.clone()),
                    };
                }
                if let Some(screen) = next.terminal() {
                    record.payload = EventPayload::TerminalCheckpoint {
                        screen: Box::new(screen.clone()),
                    };
                }
            }
            let encoded = EncodedEvent::new(record.timestamp_ms, &record.payload)?;
            let bytes = encoded.encoded_bytes() + 4;
            let projection_bytes = if let Some(document) = next.document() {
                3 * (document.byte_len() + 128)
            } else if let Some(screen) = next.terminal() {
                3 * (encoded_size(screen, MAX_EVENT_PAYLOAD_BYTES)? + 128)
            } else {
                0
            };
            let cap = if source.head.kind == ContentKind::Terminal {
                MAX_TERMINAL_SOURCE_BYTES
            } else {
                MAX_SOURCE_CHARGE
            };
            if source.charge + bytes > cap {
                return Err(KernelError::Busy);
            }
            // Reservation precedes all mutations. Packet headers disappear
            // after admission; only immutable bytes and the current bounded
            // projection keep permits through checkpoint/replacement.
            if append.permit.num_permits() < bytes + projection_bytes {
                return Err(KernelError::Busy);
            }
            let projection_charge = append
                .permit
                .split(projection_bytes)
                .expect("validated projection reservation");
            let byte_charge = append
                .permit
                .split(bytes)
                .expect("validated block reservation");
            let pushed = assembler.push(encoded)?;
            blocks.extend(pushed.before);
            blocks.extend(pushed.after);
            *projection = next;
            *projection_permit = Some(projection_charge);
            if let Some(permit) = source.permit.as_mut() {
                permit.merge(byte_charge);
            } else {
                source.permit = Some(byte_charge);
            }
            source.charge += bytes;
            source.head.accepted = sequence;
            source.dirty_since.get_or_insert_with(Instant::now);
            source.sender.send_replace(source.head.clone());
            self.2.notify_one();
            source.head.event_cursor()
        })();
        if result.is_ok() {
            metrics::add(&self.diagnostics().0.accepted_events, 1);
        }
        let _ = append.response.send(result);
        Ok(None)
    }

    fn event_page_capture(
        &self,
        content: ContentId,
        after: u64,
        limit: usize,
        frame_reads: &Arc<Semaphore>,
    ) -> Result<Option<EventPageCapture>> {
        let registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(source) = registry.sources.get(&content) else {
            return Ok(None);
        };
        let Buffer::Events {
            assembler, blocks, ..
        } = &source.buffer
        else {
            return Ok(None);
        };
        let mut captured = Vec::new();
        let mut large_read = None;
        let committed_records = source
            .head
            .committed
            .saturating_sub(after)
            .min(limit as u64);
        let mut remaining = limit - committed_records as usize;
        let mut position = after.max(source.head.committed);
        if remaining > 0 {
            for block in blocks {
                if block.end() <= position {
                    continue;
                }
                let count = (block.end() - position.max(block.start())).min(remaining as u64);
                captured.push(block.clone());
                position = position.max(block.start()) + count;
                remaining -= count as usize;
                if remaining == 0 {
                    break;
                }
            }
            if remaining > 0 {
                // A single atomic frame can exceed the ordinary page budget.
                // Admit before copying its live bytes, not only before decode.
                if assembler.pending_bytes() > MAX_EVENT_PAGE_BYTES {
                    large_read = Some(
                        frame_reads
                            .clone()
                            .try_acquire_owned()
                            .map_err(|_| KernelError::Busy)?,
                    );
                }
                captured.extend(assembler.snapshot_range(position, remaining));
            }
        }
        Ok(Some(EventPageCapture {
            head: source.head.clone(),
            tail: captured,
            large_read,
        }))
    }

    fn event_projection_capture(
        &self,
        content: ContentId,
    ) -> Option<(ContentStreamView, EventProjection)> {
        let registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let source = registry.sources.get(&content)?;
        let Buffer::Events { projection, .. } = &source.buffer else {
            return None;
        };
        Some((source.head.clone(), projection.clone()))
    }
}

pub(crate) async fn open(
    connection: &mut SqliteConnection,
    scope: port::Scope,
    round: RoundId,
    tool: ToolId,
    kind: EventKind,
) -> Result<Mutation> {
    commands::check_scope(connection, scope, true).await?;
    if commands::check_tool(connection, scope, tool).await? != round {
        return Err(KernelError::Ownership);
    }
    let active: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agena_content_streams WHERE capture_state='open'")
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    if active >= MAX_LIVE_SOURCES {
        return Err(KernelError::Busy);
    }
    let entry: i64 =
        sqlx::query_scalar("SELECT id FROM agena_entries WHERE tool_id=? AND kind='tool'")
            .bind(tool.get())
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    let id = ContentId::new();
    let epoch = ContentEpoch::new();
    bind_tool_entry(connection, entry).await?;
    let kind = match kind {
        EventKind::Log => ContentKind::Log,
        EventKind::Mixed => ContentKind::Document,
        EventKind::Structured => ContentKind::Structured,
        EventKind::Terminal => ContentKind::Terminal,
    };
    sqlx::query("INSERT INTO agena_content_streams (id,entry_id,session_id,execution_id,generation,kind,epoch,capture_state) VALUES (?,?,?,?,?,?,?,'open')")
        .bind(id.to_string()).bind(entry).bind(scope.session.get()).bind(scope.execution.to_string()).bind(scope.generation.get()).bind(commands::enum_text(&kind)?).bind(epoch.to_string())
        .execute(&mut *connection).await.map_err(sql_error)?;
    initialize(connection, id, kind).await
}

pub(crate) async fn initialize(
    connection: &mut SqliteConnection,
    id: ContentId,
    kind: ContentKind,
) -> Result<Mutation> {
    assign_ordinal(connection, id).await?;
    let mut head = stored_head(connection, id).await?;
    if kind == ContentKind::Structured {
        let payload = EventPayload::DocumentCheckpoint {
            document: Box::new(ContentDocument::default()),
        };
        let mut assembler =
            EventBlockAssembler::new(0, DEFAULT_EVENT_BLOCK_BYTES).map_err(storage)?;
        assembler
            .push(EncodedEvent::new(now_ms(), &payload)?)
            .map_err(storage)?;
        head.accepted = 1;
        head.committed = 1;
        persist(
            connection,
            &mut [StagedSource {
                head: head.clone(),
                losses: Vec::new(),
                blocks: vec![PendingBlock::Events(
                    assembler.checkpoint().expect("initial document checkpoint"),
                )],
            }],
        )
        .await?;
    }
    Ok(Mutation::Content(head))
}

fn event_kind(kind: ContentKind) -> Result<EventKind> {
    match kind {
        ContentKind::Log => Ok(EventKind::Log),
        ContentKind::Document => Ok(EventKind::Mixed),
        ContentKind::Structured => Ok(EventKind::Structured),
        ContentKind::Terminal => Ok(EventKind::Terminal),
        _ => Err(invalid(
            "event read requires log, structured or terminal content",
        )),
    }
}

fn accept_record(
    records: &mut Vec<EventRecord>,
    record: EventRecord,
    position: &mut u64,
    bytes: &mut usize,
    limit: usize,
) -> Result<bool> {
    if record.sequence <= *position {
        return Ok(true);
    }
    if record.sequence != *position + 1 {
        return Err(storage("event range has a gap"));
    }
    let charge = match &record.payload {
        EventPayload::Log { text, .. } => text.len() + 64,
        _ => encoded_size(&record.payload, MAX_TERMINAL_FRAME_BYTES + 1024)? + 64,
    };
    if !records.is_empty() && (records.len() >= limit || *bytes + charge > MAX_EVENT_PAGE_BYTES) {
        return Ok(false);
    }
    *position = record.sequence;
    *bytes += charge;
    records.push(record);
    Ok(true)
}

/// Shared physical loader for cached readers and writer-side retention.
/// This never consults or fills a reader cache and acquires no reader lease.
/// Every stored record is validated before either path can inspect a suffix.
pub(super) async fn stored_block(
    connection: &mut SqliteConnection,
    head: &ContentStreamView,
    start: i64,
) -> Result<ImmutableEventBlock> {
    let row=sqlx::query("SELECT logical_end,codec,digest,body FROM agena_content_blocks WHERE content_id=? AND logical_start=?")
        .bind(head.id.to_string()).bind(start).fetch_one(connection).await.map_err(sql_error)?;
    if row.try_get::<String, _>("codec").map_err(storage)? != "events_v1" {
        return Err(storage("event block has an incompatible codec"));
    }
    let end: i64 = row.try_get("logical_end").map_err(storage)?;
    let bytes: Vec<u8> = row.try_get("body").map_err(storage)?;
    let digest: Vec<u8> = row.try_get("digest").map_err(storage)?;
    let block = ImmutableEventBlock::from_stored(
        u64::try_from(start).map_err(storage)?,
        u64::try_from(end).map_err(storage)?,
        bytes,
        digest
            .try_into()
            .map_err(|_| storage("invalid event block checksum length"))?,
    )
    .map_err(storage)?;
    block
        .validate_kind(event_kind(head.kind)?)
        .map_err(storage)?;
    Ok(block)
}

pub(super) async fn block(
    backend: &Backend,
    connection: &mut SqliteConnection,
    session: SessionId,
    head: &ContentStreamView,
    start: i64,
) -> Result<ReadView<ImmutableEventBlock>> {
    // The caller authorizes membership and checks the current retention floor
    // in its SQLite snapshot before reaching this immutable-body cache. Session
    // commit fencing and close release these views just like other read cache
    // entries; the source ReadLease accounts for both bytes and seek indexes.
    let key = read_cache::Key::EventBlock(session, head.id, head.epoch, start as u64);
    let fill = match backend.cache.lookup(&key)? {
        read_cache::Lookup::Hit(block) => return Ok(block),
        read_cache::Lookup::Miss(fill) => fill,
    };
    metrics::add(&backend.live.diagnostics().0.event_block_queries, 1);
    let block = stored_block(connection, head, start).await?;
    metrics::add(&backend.live.diagnostics().0.event_block_rows, 1);
    metrics::add(
        &backend.live.diagnostics().0.event_block_validated_bytes,
        block.bytes().len(),
    );
    let bytes = block.retained_bytes();
    let view = backend.wrap_scoped(session, Some(ReadSource::Content(head.id)), block, bytes)?;
    backend.cache.install(key, fill, &view);
    Ok(view)
}

pub(crate) async fn page(
    backend: &Backend,
    session: SessionId,
    content: ContentId,
    after: Option<EventCursor>,
    limit: usize,
) -> Result<EventPage> {
    page_inner(backend, session, content, after, limit, None).await
}

pub(crate) async fn page_at(
    backend: &Backend,
    session: SessionId,
    boundary: &ContentStreamView,
    after: Option<EventCursor>,
    limit: usize,
) -> Result<EventPage> {
    page_inner(backend, session, boundary.id, after, limit, Some(boundary)).await
}

/// Membership and stored identity are authoritative only while the existing
/// session commit fence remains unchanged. Open heads are always read live.
#[derive(Clone)]
struct ContentReadContext {
    head: ContentStreamView,
}

fn same_identity(a: &ContentStreamView, b: &ContentStreamView) -> bool {
    a.id == b.id
        && a.entry == b.entry
        && a.epoch == b.epoch
        && a.kind == b.kind
        && a.activity_id == b.activity_id
        && a.ordinal == b.ordinal
}

struct PageRead {
    position: u64,
    records: Vec<EventRecord>,
    bytes: usize,
    limit: usize,
    done: bool,
    large_read: Option<OwnedSemaphorePermit>,
}

impl PageRead {
    fn new(head: &ContentStreamView, after: Option<EventCursor>, limit: usize) -> Result<Self> {
        event_kind(head.kind)?;
        if after.is_some_and(|cursor| {
            cursor.content != head.id
                || cursor.epoch != head.epoch
                || cursor.sequence > head.accepted
        }) {
            return Err(KernelError::ContentReset);
        }
        let position = after.map_or(0, |cursor| cursor.sequence);
        if position < head.retained_start {
            return Err(KernelError::ContentGap {
                content: head.id,
                requested: position,
                retained_start: head.retained_start,
            });
        }
        Ok(Self {
            position,
            records: Vec::new(),
            bytes: 0,
            limit,
            done: false,
            large_read: None,
        })
    }

    fn block(
        &mut self,
        backend: &Backend,
        head: &ContentStreamView,
        block: &ImmutableEventBlock,
        ceiling: u64,
    ) -> Result<()> {
        if block.start() > self.position || block.end() <= self.position {
            return Err(storage("event range has a gap"));
        }
        if block.bytes().len() > MAX_EVENT_PAGE_BYTES {
            if !self.records.is_empty() {
                self.done = true;
                return Ok(());
            }
            if self.large_read.is_none() {
                self.large_read = Some(
                    backend
                        .frame_reads
                        .clone()
                        .try_acquire_owned()
                        .map_err(|_| KernelError::Busy)?,
                );
            }
        }
        block
            .validate_kind(event_kind(head.kind)?)
            .map_err(storage)?;
        let count = ceiling
            .saturating_sub(self.position)
            .min((self.limit - self.records.len()) as u64) as usize;
        for record in block.iter_after(self.position).take(count) {
            let record = record.map_err(storage)?;
            metrics::add(&backend.live.diagnostics().0.event_page_decoded_records, 1);
            if !accept_record(
                &mut self.records,
                record,
                &mut self.position,
                &mut self.bytes,
                self.limit,
            )? {
                self.done = true;
                break;
            }
        }
        self.done |= self.records.len() >= self.limit || self.bytes >= MAX_EVENT_PAGE_BYTES;
        Ok(())
    }

    fn finish(self, head: ContentStreamView) -> EventPage {
        EventPage {
            next: EventCursor {
                content: head.id,
                epoch: head.epoch,
                sequence: self.position,
            },
            has_more: self.position < head.accepted,
            stream: head,
            records: self.records,
        }
    }
}

fn cached_page(
    backend: &Backend,
    session: SessionId,
    content: ContentId,
    after: Option<EventCursor>,
    limit: usize,
) -> Result<Option<EventPage>> {
    let stamp = backend.cache.stamp()?;
    if !backend.cache.is_current(stamp)? {
        return Err(KernelError::Busy);
    }
    // Never acquire live while holding a cache guard. The complete live cut
    // precedes authorization lookup, and the final stamp rejects a COMMIT race.
    let capture = backend.live.event_page_capture(
        content,
        after.map_or(0, |cursor| cursor.sequence),
        limit,
        &backend.frame_reads,
    )?;
    let result = (|| {
        let context = match backend
            .cache
            .lookup::<ContentReadContext>(&read_cache::Key::ContentReadContext(session, content))?
        {
            read_cache::Lookup::Hit(context) => context,
            read_cache::Lookup::Miss(_) => return Ok(None),
        };
        let (head, tail, large_read) = match capture {
            Some(EventPageCapture {
                head,
                tail,
                large_read,
            }) => {
                if !same_identity(&head, &context.head) {
                    return Err(KernelError::ContentReset);
                }
                (head, tail, large_read)
            }
            None if context.head.state != CaptureState::Open => {
                (context.head.clone(), Vec::new(), None)
            }
            None => return Ok(None),
        };
        let mut page = PageRead::new(&head, after, limit)?;
        page.large_read = large_read;
        // Discover the whole bounded span before decoding. A later cache miss
        // must not make the cold path decode an already-read prefix again.
        let end = head
            .committed
            .min(page.position.saturating_add(limit as u64));
        let mut covered = page.position;
        let mut blocks = Vec::new();
        while covered < end {
            let Some(block) = backend
                .cache
                .event_block_containing(session, content, head.epoch, covered)?
            else {
                return Ok(None);
            };
            covered = block.end();
            blocks.push(block);
        }
        for block in blocks {
            page.block(backend, &head, &block, head.committed)?;
            if page.done {
                break;
            }
        }
        if !page.done && page.position >= head.committed {
            for block in tail {
                page.block(backend, &head, &block, head.accepted)?;
                if page.done {
                    break;
                }
            }
        }
        Ok(Some(page.finish(head)))
    })();
    if !backend.cache.is_current(stamp)? {
        return Err(KernelError::Busy);
    }
    result
}

async fn page_inner(
    backend: &Backend,
    session: SessionId,
    content: ContentId,
    after: Option<EventCursor>,
    limit: usize,
    boundary: Option<&ContentStreamView>,
) -> Result<EventPage> {
    let limit = limit.clamp(1, MAX_EVENT_PAGE_RECORDS);
    if boundary.is_none()
        && let Some(page) = cached_page(backend, session, content, after, limit)?
    {
        return Ok(page);
    }
    let _read = backend
        .content_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let fill = backend.cache.stamp()?;
    let capture = if boundary.is_none() {
        backend.live.event_page_capture(
            content,
            after.map_or(0, |cursor| cursor.sequence),
            limit,
            &backend.frame_reads,
        )?
    } else {
        None
    };
    let mut transaction = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let measurement = backend.live.diagnostics().read_transaction();
    let result = async {
        authorize(&mut transaction, session, content).await?;
        let (head, tail, context, large_read) = if let Some(boundary) = boundary {
            (validate_boundary(&mut transaction, boundary).await?, Vec::new(), None, None)
        } else {
            let stored = stored_head(&mut transaction, content).await?;
            let context = ContentReadContext { head: stored.clone() };
            let (head, tail, large_read) = match capture {
                Some(EventPageCapture { head, tail, large_read }) => {
                    if !same_identity(&head, &stored) {
                        return Err(KernelError::ContentReset);
                    }
                    (head, tail, large_read)
                }
                None => (stored, Vec::new(), None),
            };
            (head, tail, Some(context), large_read)
        };
        let mut page = PageRead::new(&head, after, limit)?;
        page.large_read = large_read;
        while !page.done && page.position < head.committed {
            let block = if let Some(block) = backend.cache.event_block_containing(session, content, head.epoch, page.position)? {
                block
            } else {
                // Usually a single physical block covers the requested page.
                // Do not collect 64 future metadata rows for every 32 events.
                let row = sqlx::query("SELECT logical_start,length(body) AS bytes FROM agena_content_blocks WHERE content_id=? AND logical_start<=? ORDER BY logical_start DESC LIMIT 1")
                    .bind(content.to_string()).bind(page.position as i64).fetch_optional(&mut *transaction).await.map_err(sql_error)?
                    .ok_or_else(|| storage("committed event range has no block"))?;
                if row.try_get::<i64, _>("bytes").map_err(storage)? > MAX_EVENT_PAGE_BYTES as i64 {
                    if !page.records.is_empty() {
                        page.done = true;
                        break;
                    }
                    if page.large_read.is_none() {
                        page.large_read = Some(backend.frame_reads.clone().try_acquire_owned().map_err(|_| KernelError::Busy)?);
                    }
                }
                block(backend, &mut transaction, session, &head, row.try_get("logical_start").map_err(storage)?).await?
            };
            page.block(backend, &head, &block, head.committed)?;
        }
        if !page.done && page.position >= head.committed {
            for block in tail {
                page.block(backend, &head, &block, head.accepted)?;
                if page.done { break; }
            }
        }
        let large_read = page.large_read.take();
        Ok((page.finish(head), context, large_read))
    }.await;
    // Even a rejected range closes its read snapshot before returning admission.
    // Only acknowledged rollbacks enter the actual transaction-duration metric.
    transaction.rollback().await.map_err(sql_error)?;
    measurement.complete();
    let (page, context, _large_read) = result?;
    if let Some(context) = context {
        if !backend.cache.is_current(fill)? {
            return Err(KernelError::Busy);
        }
        let bytes = modeled_read_bytes(&context.head)?;
        let view =
            backend.wrap_scoped(session, Some(ReadSource::Content(content)), context, bytes)?;
        backend.cache.install(
            read_cache::Key::ContentReadContext(session, content),
            fill,
            &view,
        );
        if !backend.cache.is_current(fill)? {
            return Err(KernelError::Busy);
        }
    }
    Ok(page)
}

pub(crate) async fn snapshot(
    backend: &Backend,
    session: SessionId,
    content: ContentId,
) -> Result<EventSnapshot> {
    snapshot_inner(backend, session, content, None).await
}

pub(crate) async fn snapshot_at(
    backend: &Backend,
    session: SessionId,
    boundary: &ContentStreamView,
) -> Result<EventSnapshot> {
    snapshot_inner(backend, session, boundary.id, Some(boundary)).await
}

async fn snapshot_inner(
    backend: &Backend,
    session: SessionId,
    content: ContentId,
    boundary: Option<&ContentStreamView>,
) -> Result<EventSnapshot> {
    let _read = backend
        .content_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let capture = boundary
        .is_none()
        .then(|| backend.live.event_projection_capture(content))
        .flatten();
    let mut transaction = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    let measurement = backend.live.diagnostics().read_transaction();
    let result = async {
    authorize(&mut transaction, session, content).await?;
    if let Some((head, projection)) = capture {
        return projection.snapshot(head);
    }
    let head = if let Some(boundary) = boundary {
        validate_boundary(&mut transaction, boundary).await?
    } else {
        stored_head(&mut transaction, content).await?
    };
    let kind = event_kind(head.kind)?;
    if head.retained_start > 0 && head.retained_start >= head.committed {
        return Err(KernelError::ContentGap {
            content,
            requested: head.committed,
            retained_start: head.retained_start,
        });
    }
    if head.committed == 0 {
        return EventProjection::new(kind).snapshot(head);
    }
    let checkpoint=sqlx::query("SELECT logical_sequence,block_start FROM agena_content_checkpoints WHERE content_id=? AND logical_sequence<=? ORDER BY logical_sequence DESC LIMIT 1")
        .bind(content.to_string()).bind(head.committed as i64).fetch_optional(&mut *transaction).await.map_err(sql_error)?.ok_or_else(||KernelError::NotFound("event checkpoint".into()))?;
    let sequence: i64 = checkpoint.try_get("logical_sequence").map_err(storage)?;
    let start: i64 = checkpoint.try_get("block_start").map_err(storage)?;
    let starts: Vec<i64>=sqlx::query_scalar("SELECT logical_start FROM agena_content_blocks WHERE content_id=? AND logical_start>=? AND logical_start<? ORDER BY logical_start LIMIT 66")
        .bind(content.to_string()).bind(start).bind(head.committed as i64).fetch_all(&mut *transaction).await.map_err(sql_error)?;
    let _large = backend
        .frame_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let mut projection = None;
    for start in starts {
        let block = block(backend, &mut transaction, session, &head, start).await?;
        block.validate_kind(kind).map_err(storage)?;
        for record in block.iter_after((sequence as u64).saturating_sub(1)) {
            let record = record.map_err(storage)?;
            if record.sequence < sequence as u64 {
                continue;
            }
            if record.sequence > head.committed {
                break;
            }
            match projection.as_mut() {
                Some(projection) => EventProjection::apply(projection, &record)?,
                None => {
                    if record.sequence != sequence as u64 {
                        return Err(storage("indexed checkpoint record is missing"));
                    }
                    projection = Some(EventProjection::from_checkpoint(&record).map_err(storage)?)
                }
            }
        }
    }
    let projection = projection.ok_or_else(|| storage("indexed checkpoint record is missing"))?;
    if projection.sequence() != head.committed {
        return Err(storage("event checkpoint chain is incomplete"));
    }
    projection.snapshot(head)
    }.await;
    transaction.rollback().await.map_err(sql_error)?;
    measurement.complete();
    result
}
