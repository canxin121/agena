//! Live admission, immutable text checkpoints and indexed byte reads. Only the
//! command writer persists source bytes. Pipe readers can admit synchronously
//! under a short memory lock; terminal barriers fence concurrent admission.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Mutex, MutexGuard, TryLockError},
    time::Instant,
};

use agena_domain::{ContentEpoch, ContentId};
use agena_storage::backend::{
    content_blocks::{DEFAULT_TEXT_BLOCK_BYTES, ImmutableTextBlock, TextBlockAssembler},
    content_events::{DEFAULT_EVENT_BLOCK_BYTES, EventBlockAssembler, ImmutableEventBlock},
    kernel::OutputScope,
};
use tokio::sync::watch;

use super::*;

pub(super) mod events;
pub(super) mod loss;
mod retention;
pub(super) use retention::prune;

pub(super) const MAX_LIVE_SOURCES: i64 = 128;
pub(super) const MAX_APPEND_BYTES: usize = 256 * 1024;
const MAX_SOURCE_CHARGE: usize = 512 * 1024;
const CHECKPOINT_BYTES: usize = 64 * 1024;
const CHECKPOINT_INTERVAL: Duration = Duration::from_millis(500);
const MAX_CHECKPOINT_BYTES: usize = 512 * 1024;
const MAX_RANGE_BYTES: usize = 256 * 1024;
const ORPHAN_GRACE: Duration = Duration::from_secs(30);
const MAX_PENDING_LOSS_WINDOWS: usize = 32;

pub(super) struct Append {
    pub scope: OutputScope,
    pub content: ContentId,
    pub epoch: ContentEpoch,
    pub text: String,
    pub response: oneshot::Sender<Result<TextCursor>>,
    pub permit: OwnedSemaphorePermit,
    pub _queue_slot: OwnedSemaphorePermit,
}

struct LiveSource {
    scope: OutputScope,
    round: Option<RoundId>,
    tool: Option<ToolId>,
    head: ContentStreamView,
    buffer: Buffer,
    permit: Option<OwnedSemaphorePermit>,
    charge: usize,
    dirty_since: Option<Instant>,
    producers: usize,
    claimed: bool,
    created: Instant,
    abandoned: bool,
    terminal_gate: Option<RequestId>,
    losses: VecDeque<CaptureLoss>,
    sender: watch::Sender<ContentStreamView>,
}

enum Buffer {
    Text {
        assembler: TextBlockAssembler,
        blocks: VecDeque<ImmutableTextBlock>,
    },
    Events {
        assembler: EventBlockAssembler,
        blocks: VecDeque<ImmutableEventBlock>,
        projection: EventProjection,
        projection_permit: Option<OwnedSemaphorePermit>,
    },
}

#[derive(Clone)]
enum PendingBlock {
    Text(ImmutableTextBlock),
    Events(ImmutableEventBlock),
}

impl PendingBlock {
    fn charge(&self) -> usize {
        match self {
            Self::Text(block) => block.bytes().len(),
            Self::Events(block) => {
                block.bytes().len() - 4 + 4 * (block.end() - block.start()) as usize
            }
        }
    }
    fn start(&self) -> u64 {
        match self {
            Self::Text(block) => block.start(),
            Self::Events(block) => block.start(),
        }
    }
    fn end(&self) -> u64 {
        match self {
            Self::Text(block) => block.end(),
            Self::Events(block) => block.end(),
        }
    }
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Text(block) => block.bytes(),
            Self::Events(block) => block.bytes(),
        }
    }
    fn digest(&self) -> &[u8; 32] {
        match self {
            Self::Text(block) => block.digest(),
            Self::Events(block) => block.digest(),
        }
    }
    fn codec(&self) -> &'static str {
        match self {
            Self::Text(_) => "utf8",
            Self::Events(_) => "events_v1",
        }
    }
}

impl Buffer {
    fn stage(&mut self, full: bool) -> Vec<PendingBlock> {
        match self {
            Self::Text { assembler, blocks } => {
                if full {
                    blocks.extend(assembler.checkpoint());
                }
                blocks.iter().cloned().map(PendingBlock::Text).collect()
            }
            Self::Events {
                assembler, blocks, ..
            } => {
                if full {
                    blocks.extend(assembler.checkpoint());
                }
                blocks.iter().cloned().map(PendingBlock::Events).collect()
            }
        }
    }
    fn acknowledge(&mut self, head: u64) {
        match self {
            Self::Text { blocks, .. } => blocks.retain(|block| block.end() > head),
            Self::Events { blocks, .. } => blocks.retain(|block| block.end() > head),
        }
    }
}

#[derive(Default)]
struct Registry {
    sources: HashMap<ContentId, LiveSource>,
    order: VecDeque<ContentId>,
}

pub(super) struct LiveContent(Mutex<Registry>, KernelDiagnostics, tokio::sync::Notify);

/// Only the single persistence writer takes this guard. Every return path,
/// including transaction rollback/cancellation, reopens provisional gates.
pub(super) struct AdmissionGates<'a>(&'a LiveContent);
impl Drop for AdmissionGates<'_> {
    fn drop(&mut self) {
        let mut registry = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        for source in registry.sources.values_mut() {
            source.terminal_gate = None;
        }
    }
}

#[derive(Clone)]
pub(super) struct StagedSource {
    pub head: ContentStreamView,
    blocks: Vec<PendingBlock>,
    losses: Vec<CaptureLoss>,
}

impl LiveContent {
    pub(super) fn new(diagnostics: KernelDiagnostics) -> Self {
        Self(
            Mutex::new(Registry::default()),
            diagnostics,
            tokio::sync::Notify::new(),
        )
    }
    pub(super) fn diagnostics(&self) -> &KernelDiagnostics {
        &self.1
    }
    pub(super) fn admission_gates(&self) -> AdmissionGates<'_> {
        AdmissionGates(self)
    }
    pub(super) fn release_gate(&self, request: RequestId) {
        let mut registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for source in registry.sources.values_mut() {
            if source.terminal_gate == Some(request) {
                source.terminal_gate = None;
            }
        }
    }
    pub(super) async fn admission_changed(&self) {
        self.2.notified().await;
    }
    fn admission_lock(&self, synchronous: bool) -> Result<MutexGuard<'_, Registry>> {
        let guard = if synchronous {
            match self.0.try_lock() {
                Ok(guard) => Ok(guard),
                Err(TryLockError::Poisoned(error)) => Ok(error.into_inner()),
                Err(TryLockError::WouldBlock) => Err(KernelError::Busy),
            }
        } else {
            Ok(self.0.lock().unwrap_or_else(|error| error.into_inner()))
        }?;
        // Check under the admission lock too. A commit with an unknown outcome
        // may have closed the backend after the caller's initial open check.
        if synchronous {
            self.1.1.ensure_open()?;
        }
        Ok(guard)
    }
    pub fn retain(&self, scope: OutputScope, stream: &ContentStreamView) -> Result<()> {
        let mut registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(source) = registry.sources.get_mut(&stream.id) {
            if source.scope != scope || source.head.epoch != stream.epoch {
                return Err(KernelError::Ownership);
            }
            source.producers = source.producers.checked_add(1).ok_or(KernelError::Busy)?;
            source.claimed = true;
            source.abandoned = false;
        }
        // Replaying a completed open returns its original identity, but that
        // handle cannot reopen a sealed source or accept further bytes.
        Ok(())
    }

    pub fn release(&self, scope: OutputScope, content: ContentId) {
        let mut registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(source) = registry.sources.get_mut(&content)
            && source.scope == scope
        {
            source.producers = source.producers.saturating_sub(1);
            source.abandoned = source.producers == 0;
        }
    }

    pub fn install_open(
        &self,
        scope: OutputScope,
        round: Option<RoundId>,
        tool: Option<ToolId>,
        head: ContentStreamView,
    ) {
        let mut registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if registry.sources.contains_key(&head.id) || head.state != CaptureState::Open {
            return;
        }
        let (sender, _) = watch::channel(head.clone());
        let buffer = if head.kind == agena_domain::ContentKind::Text {
            Buffer::Text {
                assembler: TextBlockAssembler::new(head.committed, DEFAULT_TEXT_BLOCK_BYTES)
                    .expect("fixed block size"),
                blocks: VecDeque::new(),
            }
        } else {
            let kind = match head.kind {
                agena_domain::ContentKind::Log => EventKind::Log,
                agena_domain::ContentKind::Document => EventKind::Mixed,
                agena_domain::ContentKind::Structured => EventKind::Structured,
                agena_domain::ContentKind::Terminal => EventKind::Terminal,
                _ => unreachable!("validated event source kind"),
            };
            let mut projection = EventProjection::new(kind);
            if kind == EventKind::Structured {
                projection
                    .apply(&EventRecord {
                        sequence: 1,
                        timestamp_ms: 0,
                        payload: EventPayload::DocumentCheckpoint {
                            document: Box::default(),
                        },
                    })
                    .expect("initial empty document");
            }
            Buffer::Events {
                assembler: EventBlockAssembler::new(head.committed, DEFAULT_EVENT_BLOCK_BYTES)
                    .expect("fixed block size"),
                blocks: VecDeque::new(),
                projection,
                projection_permit: None,
            }
        };
        if head.kind == agena_domain::ContentKind::Structured && head.committed == 1 {
            metrics::add(&self.1.0.event_blocks, 1);
        }
        let source = LiveSource {
            scope,
            round,
            tool,
            buffer,
            permit: None,
            charge: 0,
            dirty_since: None,
            producers: 0,
            claimed: false,
            created: Instant::now(),
            abandoned: false,
            terminal_gate: None,
            losses: VecDeque::new(),
            sender,
            head: head.clone(),
        };
        registry.order.push_back(head.id);
        registry.sources.insert(head.id, source);
    }

    /// FIFO admission for ordinary async producers. Synchronous capture uses
    /// the same mutation after obtaining a nonblocking memory lock.
    pub fn append(&self, append: Append) -> Option<Append> {
        self.append_inner(append, false)
            .expect("blocking memory lock cannot be busy")
    }
    pub fn try_append(&self, append: Append) -> Result<Option<Append>> {
        self.append_inner(append, true)
    }
    fn append_inner(&self, append: Append, synchronous: bool) -> Result<Option<Append>> {
        let mut registry = self.admission_lock(synchronous)?;
        let Some(source) = registry.sources.get_mut(&append.content) else {
            return Ok(Some(append));
        };
        let result = if let Err(error) = source.scope.check(append.scope) {
            Err(error)
        } else if source.head.epoch != append.epoch {
            Err(KernelError::Ownership)
        } else if source.head.state != CaptureState::Open || source.abandoned {
            Err(KernelError::Conflict("content capture is closed".into()))
        } else if source.terminal_gate.is_some() || source.losses.len() == MAX_PENDING_LOSS_WINDOWS
        {
            Err(KernelError::Busy)
        } else if append.text.is_empty() {
            source.head.text_cursor()
        } else if source.charge + append.text.len() > MAX_SOURCE_CHARGE {
            Err(KernelError::Busy)
        } else if source
            .head
            .accepted
            .checked_add(append.text.len() as u64)
            .is_none_or(|n| n > i64::MAX as u64)
        {
            Err(invalid("content position exceeds SQLite integer range"))
        } else {
            let Buffer::Text { assembler, blocks } = &mut source.buffer else {
                let _ = append
                    .response
                    .send(Err(invalid("text append requires a text source")));
                return Ok(None);
            };
            let mut suffix = append.text.as_str();
            while !suffix.is_empty() {
                // Size/position were checked before any byte was admitted.
                let step = assembler.push(suffix).expect("validated content offset");
                suffix = &suffix[step.consumed_bytes..];
                if let Some(block) = step.block {
                    blocks.push_back(block);
                }
            }
            source.head.accepted = assembler.accepted_head();
            source.dirty_since.get_or_insert_with(Instant::now);
            source.charge += append.text.len();
            let mut admission = append.permit;
            let bytes = admission
                .split(append.text.len())
                .expect("admitted byte charge");
            if let Some(permit) = source.permit.as_mut() {
                permit.merge(bytes);
            } else {
                source.permit = Some(bytes);
            }
            // The packet allocation/header no longer exists after this step.
            // Merge its retained byte permits instead of retaining one object
            // per packet until the next checkpoint.
            drop(admission);
            source.sender.send_replace(source.head.clone());
            self.2.notify_one();
            source.head.text_cursor()
        };
        if result.is_ok() {
            metrics::add(&self.1.0.accepted_text_appends, 1);
            metrics::add(&self.1.0.accepted_text_bytes, append.text.len());
        }
        let _ = append.response.send(result);
        Ok(None)
    }

    fn stage_source(source: &mut LiveSource, state: CaptureState, full: bool) -> StagedSource {
        // A loss window references an accepted position. Commit its complete
        // preceding tail so no durable window can point beyond the byte head.
        let blocks = source.buffer.stage(full || !source.losses.is_empty());
        let mut head = source.head.clone();
        head.committed = blocks.last().map_or(head.committed, PendingBlock::end);
        head.state = if state == CaptureState::Complete && head.capture_loss.packets > 0 {
            CaptureState::Interrupted
        } else {
            state
        };
        head.capture_loss.committed_bytes = head.capture_loss.bytes;
        head.capture_loss.committed_packets = head.capture_loss.packets;
        StagedSource {
            head,
            blocks,
            losses: source.losses.iter().cloned().collect(),
        }
    }

    pub fn checkpoint(&self, timed: bool, closing: bool) -> Vec<StagedSource> {
        let mut registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut staged = Vec::new();
        let mut bytes = 0;
        let count = registry.order.len();
        for _ in 0..count {
            let id = registry.order.pop_front().expect("bounded source order");
            registry.order.push_back(id);
            let source = registry.sources.get_mut(&id).expect("live source order");
            let abandoned =
                source.abandoned || (!source.claimed && source.created.elapsed() >= ORPHAN_GRACE);
            let dirty = source.head.accepted > source.head.committed || !source.losses.is_empty();
            let due = timed
                && source
                    .dirty_since
                    .is_some_and(|since| since.elapsed() >= CHECKPOINT_INTERVAL);
            if !closing && !abandoned && (!dirty || (!due && source.charge < CHECKPOINT_BYTES)) {
                continue;
            }
            let state = if closing || abandoned {
                CaptureState::Interrupted
            } else {
                CaptureState::Open
            };
            if state != CaptureState::Open {
                source.terminal_gate.get_or_insert_with(RequestId::new);
            }
            bytes += source.charge;
            staged.push(Self::stage_source(
                source,
                state,
                due || closing || abandoned,
            ));
            if !closing && (bytes >= MAX_CHECKPOINT_BYTES || staged.len() >= 8) {
                break;
            }
        }
        staged
    }

    pub fn next_checkpoint(&self) -> tokio::time::Instant {
        let registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let mut next = now + CHECKPOINT_INTERVAL;
        for source in registry.sources.values() {
            if source.abandoned || source.charge >= CHECKPOINT_BYTES {
                return tokio::time::Instant::now();
            }
            if !source.claimed {
                next = next.min(source.created + ORPHAN_GRACE);
            }
            if let Some(since) = source.dirty_since {
                next = next.min(since + CHECKPOINT_INTERVAL);
            }
        }
        tokio::time::Instant::from_std(next)
    }

    /// Terminal scopes are selected from the bounded live registry. An
    /// inherited source stays owned by its origin; membership grants reads.
    pub fn terminal(&self, request: RequestId, command: &Command) -> Vec<StagedSource> {
        let mut registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        registry
            .sources
            .values_mut()
            .filter_map(|source| {
                let mut diagnostic = None;
                let state = match command {
                    Command::DeleteSession{session} if source.scope.session()==*session => CaptureState::Interrupted,
                    Command::SealContent {
                        scope,
                        content,
                        epoch,
                        ending,
                    } if source.scope == *scope
                        && source.head.id == *content
                        && source.head.epoch == *epoch =>
                    {
                        diagnostic = ending.diagnostic().map(str::to_owned);
                        ending.state()
                    }
                    Command::CompleteTool { scope, tool, .. }
                        if source.scope == OutputScope::Attempt(*scope) && source.tool == Some(*tool) =>
                    {
                        CaptureState::Complete
                    }
                    Command::FinishRound {
                        scope,
                        round,
                        succeeded,
                        ..
                    } if source.scope == OutputScope::Attempt(*scope)
                        && source.round == Some(*round)
                        && source.tool.is_none() =>
                    {
                        if *succeeded {
                            CaptureState::Complete
                        } else {
                            CaptureState::Interrupted
                        }
                    }
                    Command::CommitCompaction { scope, round, .. }
                        if source.scope == OutputScope::Attempt(*scope)
                            && source.round == Some(*round)
                            && source.tool.is_none() =>
                    {
                        CaptureState::Complete
                    }
                    Command::Cancel { session, execution }
                        if matches!(source.scope, OutputScope::Attempt(scope) if scope.session == *session && scope.execution == *execution) =>
                    {
                        CaptureState::Interrupted
                    }
                    Command::Finish { scope, phase } if source.scope == OutputScope::Attempt(*scope) => {
                        if *phase == ExecutionPhase::Succeeded {
                            CaptureState::Complete
                        } else {
                            CaptureState::Interrupted
                        }
                    }
                    Command::EndActivity { scope, outcome }
                        if source.scope == OutputScope::Activity(*scope) =>
                    {
                        let ending = outcome.capture_end();
                        diagnostic = ending.diagnostic().map(str::to_owned);
                        ending.state()
                    }
                    _ => return None,
                };
                source.terminal_gate.get_or_insert(request);
                let mut staged = Self::stage_source(source, state, true);
                if diagnostic.is_some() { staged.head.capture_error=diagnostic; }
                Some(staged)
            })
            .collect()
    }

    /// Install only after COMMIT. Failed transactions leave immutable blocks,
    /// accepted bytes and their admission permits intact for the next attempt.
    pub fn acknowledge(&self, staged: &[StagedSource]) {
        let mut registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for update in staged {
            let Some(source) = registry.sources.get_mut(&update.head.id) else {
                continue;
            };
            metrics::add(
                &self.1.0.text_blocks,
                update
                    .blocks
                    .iter()
                    .filter(|block| matches!(block, PendingBlock::Text(_)))
                    .count(),
            );
            metrics::add(
                &self.1.0.event_blocks,
                update
                    .blocks
                    .iter()
                    .filter(|block| matches!(block, PendingBlock::Events(_)))
                    .count(),
            );
            let accepted = source.head.accepted;
            let observed_loss = source.head.capture_loss;
            let capture_error = source.head.capture_error.clone();
            source.head = update.head.clone();
            if source.head.state == CaptureState::Open {
                // Synchronous capture may have admitted a suffix while this
                // ordinary checkpoint was on disk. Its head/projection and
                // permits remain live; acknowledgment must not rewind it.
                source.head.accepted = source.head.accepted.max(accepted);
                source.head.capture_loss.bytes =
                    source.head.capture_loss.bytes.max(observed_loss.bytes);
                source.head.capture_loss.packets =
                    source.head.capture_loss.packets.max(observed_loss.packets);
                if source.head.capture_error.is_none() {
                    source.head.capture_error = capture_error;
                }
            }
            for loss in &update.losses {
                let Some(pending) = source.losses.front_mut() else {
                    break;
                };
                assert_eq!(
                    pending.first_packet, loss.first_packet,
                    "loss acknowledgment follows FIFO"
                );
                if pending.last_packet == loss.last_packet {
                    source.losses.pop_front();
                } else {
                    pending.first_packet = loss.last_packet + 1;
                    pending.bytes -= loss.bytes;
                }
            }
            if source.head.accepted == source.head.committed && source.losses.is_empty() {
                source.dirty_since = None;
            }
            source.buffer.acknowledge(update.head.committed);
            let release = update
                .blocks
                .iter()
                .map(PendingBlock::charge)
                .sum::<usize>()
                .min(source.charge);
            source.charge -= release;
            if let Some(permit) = source.permit.as_mut() {
                drop(
                    permit
                        .split(release)
                        .expect("committed source byte reservation"),
                );
                if permit.num_permits() == 0 {
                    source.permit = None;
                }
            }
            source.sender.send_replace(source.head.clone());
            if update.head.state != CaptureState::Open {
                registry.sources.remove(&update.head.id);
                registry.order.retain(|id| *id != update.head.id);
            }
        }
    }

    fn snapshot(
        &self,
        content: ContentId,
        start: u64,
        limit: usize,
    ) -> Option<(ContentStreamView, Vec<u8>)> {
        let registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let source = registry.sources.get(&content)?;
        let Buffer::Text { assembler, blocks } = &source.buffer else {
            return None;
        };
        let end = source.head.accepted.min(start.saturating_add(limit as u64));
        let mut bytes = Vec::new();
        let live_start = start.max(source.head.committed);
        if live_start < end {
            bytes.reserve((end - live_start) as usize);
            for block in blocks {
                let from = live_start.max(block.start());
                let to = end.min(block.end());
                if from < to {
                    bytes.extend_from_slice(block.range(from, to).expect("intersection"));
                }
            }
            let pending = assembler.pending_text().as_bytes();
            let base = source.head.accepted - pending.len() as u64;
            let from = live_start.max(base);
            if from < end {
                bytes.extend_from_slice(&pending[(from - base) as usize..(end - base) as usize]);
            }
        }
        Some((source.head.clone(), bytes))
    }

    fn subscribe(&self, content: ContentId) -> Option<ContentWatch> {
        let registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        registry
            .sources
            .get(&content)
            .map(|source| ContentWatch::from_receiver(source.sender.subscribe()))
    }

    pub fn fail_closed(&self, diagnostic: &str) {
        let mut registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for source in registry.sources.values_mut() {
            source.head.epoch = ContentEpoch::new();
            source.head.accepted = source.head.committed;
            source.head.state = CaptureState::Interrupted;
            source.head.capture_error = Some(diagnostic.to_owned());
            source.sender.send_replace(source.head.clone());
        }
        registry.sources.clear();
        registry.order.clear();
    }
}

pub(super) async fn assign_ordinal(
    connection: &mut SqliteConnection,
    content: ContentId,
) -> Result<()> {
    sqlx::query("UPDATE agena_content_streams AS c SET ordinal=(SELECT count(*)-1 FROM agena_content_streams p WHERE p.entry_id=c.entry_id) WHERE c.id=?")
        .bind(content.to_string()).execute(connection).await.map_err(sql_error)?;
    Ok(())
}

pub(super) async fn stored_head(
    connection: &mut SqliteConnection,
    content: ContentId,
) -> Result<ContentStreamView> {
    let row = sqlx::query("SELECT entry_id,epoch,kind,activity_id,ordinal,committed_head,lost_bytes,lost_packets,retained_start,capture_state,capture_error FROM agena_content_streams WHERE id=?")
        .bind(content.to_string()).fetch_optional(connection).await.map_err(sql_error)?
        .ok_or_else(|| KernelError::NotFound("text stream".into()))?;
    let committed: i64 = row.try_get("committed_head").map_err(storage)?;
    Ok(ContentStreamView {
        id: content,
        entry: EntryId::try_from(row.try_get::<i64, _>("entry_id").map_err(storage)?)
            .map_err(storage)?,
        epoch: row
            .try_get::<String, _>("epoch")
            .map_err(storage)?
            .parse()
            .map_err(storage)?,
        kind: decode(&encode(
            &row.try_get::<String, _>("kind").map_err(storage)?,
        )?)?,
        activity_id: row
            .try_get::<Option<String>, _>("activity_id")
            .map_err(storage)?
            .map(|id| id.parse().map_err(storage))
            .transpose()?,
        ordinal: row.try_get::<i64, _>("ordinal").map_err(storage)? as u32,
        accepted: committed as u64,
        committed: committed as u64,
        capture_loss: CaptureLossCounters {
            bytes: row.try_get::<i64, _>("lost_bytes").map_err(storage)? as u64,
            packets: row.try_get::<i64, _>("lost_packets").map_err(storage)? as u64,
            committed_bytes: row.try_get::<i64, _>("lost_bytes").map_err(storage)? as u64,
            committed_packets: row.try_get::<i64, _>("lost_packets").map_err(storage)? as u64,
        },
        retained_start: row.try_get::<i64, _>("retained_start").map_err(storage)? as u64,
        state: decode(&encode(
            &row.try_get::<String, _>("capture_state").map_err(storage)?,
        )?)?,
        capture_error: row.try_get("capture_error").map_err(storage)?,
    })
}

pub(super) async fn bind_tool_entry(connection: &mut SqliteConnection, entry: i64) -> Result<()> {
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM agena_content_streams WHERE entry_id=?")
            .bind(entry)
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    if count >= 128 {
        return Err(invalid("tool output source limit exceeded"));
    }
    sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE id=?")
        .bind(entry)
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(())
}

enum TerminalTarget {
    Session(SessionId),
    Content(ContentId, ContentEpoch),
    Tool(port::Scope, ToolId),
    Round(port::Scope, RoundId),
    Attempt(SessionId, ExecutionId, Option<AttemptGeneration>),
    Activity(port::ActivityScope),
}

impl TerminalTarget {
    fn filter(&self, query: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>) {
        query.push("c.capture_state='open' AND ");
        match *self {
            Self::Session(session) => {
                query.push("c.session_id=").push_bind(session.get());
            }
            Self::Content(id, epoch) => {
                query
                    .push("c.id=")
                    .push_bind(id.to_string())
                    .push(" AND c.epoch=")
                    .push_bind(epoch.to_string());
            }
            Self::Tool(scope, tool) => {
                query.push("c.execution_id=").push_bind(scope.execution.to_string()).push(" AND c.generation=").push_bind(scope.generation.get()).push(" AND c.activity_id IS NULL AND c.entry_id IN (SELECT id FROM agena_entries WHERE tool_id=").push_bind(tool.get()).push(" AND kind='tool')");
            }
            Self::Round(scope, round) => {
                query.push("c.execution_id=").push_bind(scope.execution.to_string()).push(" AND c.generation=").push_bind(scope.generation.get()).push(" AND c.activity_id IS NULL AND c.entry_id IN (SELECT id FROM agena_entries WHERE round_id=").push_bind(round.get()).push(" AND tool_id IS NULL)");
            }
            Self::Attempt(session, execution, generation) => {
                query
                    .push("c.session_id=")
                    .push_bind(session.get())
                    .push(" AND c.execution_id=")
                    .push_bind(execution.to_string())
                    .push(" AND c.activity_id IS NULL");
                if let Some(generation) = generation {
                    query.push(" AND c.generation=").push_bind(generation.get());
                }
            }
            Self::Activity(scope) => {
                query
                    .push("c.activity_id=")
                    .push_bind(scope.id.to_string())
                    .push(" AND c.session_id=")
                    .push_bind(scope.session.get());
            }
        }
    }
}

/// A source opened earlier in this same transaction is not yet in the live
/// registry and cannot have an accepted in-memory tail. Seal those durable
/// rows too. Relying solely on live sources would resurrect a writer when an
/// open and its owner's completion/cancellation are batched together.
pub(super) async fn seal_stored(
    connection: &mut SqliteConnection,
    command: &Command,
) -> Result<()> {
    let (target, ending) = match command {
        Command::DeleteSession { session } => (
            TerminalTarget::Session(*session),
            CaptureEnd::Interrupted {
                diagnostic: "origin session deleted".into(),
            },
        ),
        Command::SealContent {
            content,
            epoch,
            ending,
            ..
        } => (TerminalTarget::Content(*content, *epoch), ending.clone()),
        Command::CompleteTool { scope, tool, .. } => {
            (TerminalTarget::Tool(*scope, *tool), CaptureEnd::Complete)
        }
        Command::FinishRound {
            scope,
            round,
            succeeded,
            ..
        } => (
            TerminalTarget::Round(*scope, *round),
            if *succeeded {
                CaptureEnd::Complete
            } else {
                CaptureEnd::Interrupted {
                    diagnostic: "provider round did not complete".into(),
                }
            },
        ),
        Command::CommitCompaction { scope, round, .. } => {
            (TerminalTarget::Round(*scope, *round), CaptureEnd::Complete)
        }
        Command::Cancel { session, execution } => (
            TerminalTarget::Attempt(*session, *execution, None),
            CaptureEnd::Interrupted {
                diagnostic: "execution cancelled".into(),
            },
        ),
        Command::Finish { scope, phase } => (
            TerminalTarget::Attempt(scope.session, scope.execution, Some(scope.generation)),
            if *phase == ExecutionPhase::Succeeded {
                CaptureEnd::Complete
            } else {
                CaptureEnd::Interrupted {
                    diagnostic: "execution ended before capture completed".into(),
                }
            },
        ),
        Command::EndActivity { scope, outcome } => {
            (TerminalTarget::Activity(*scope), outcome.capture_end())
        }
        _ => return Ok(()),
    };
    let mut revisions = sqlx::QueryBuilder::new(
        "UPDATE agena_entries SET revision=revision+1 WHERE id IN (SELECT c.entry_id FROM agena_content_streams c WHERE ",
    );
    target.filter(&mut revisions);
    revisions
        .push(")")
        .build()
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    let mut streams =
        sqlx::QueryBuilder::new("UPDATE agena_content_streams AS c SET capture_state=");
    streams
        .push("CASE WHEN lost_packets>0 AND ")
        .push_bind(commands::enum_text(&ending.state())?)
        .push("='complete' THEN 'interrupted' ELSE ")
        .push_bind(commands::enum_text(&ending.state())?)
        .push(" END,capture_error=COALESCE(")
        .push_bind(ending.diagnostic())
        .push(",capture_error)")
        .push(" WHERE ");
    target.filter(&mut streams);
    streams
        .build()
        .execute(connection)
        .await
        .map_err(sql_error)?;
    Ok(())
}

pub(super) async fn persist(
    connection: &mut SqliteConnection,
    sources: &mut [StagedSource],
) -> Result<()> {
    for source in sources {
        let current = stored_head(connection, source.head.id).await?;
        if current.epoch != source.head.epoch {
            return Err(KernelError::ContentReset);
        }
        if current.state != CaptureState::Open {
            // A preceding command in this same batch already sealed capture.
            // Its exact outcome remains authoritative even if a later cancel
            // changes the independent execution outcome.
            source.head = current;
            source.blocks.clear();
            source.losses.clear();
            continue;
        }
        let mut position = current.committed;
        for block in &source.blocks {
            if block.start() != position {
                return Err(storage("non-contiguous staged content"));
            }
            sqlx::query("INSERT INTO agena_content_blocks (content_id,logical_start,logical_end,codec,digest,body) VALUES (?,?,?,?,?,?)")
                .bind(source.head.id.to_string()).bind(block.start() as i64).bind(block.end() as i64).bind(block.codec()).bind(block.digest().as_slice()).bind(block.bytes())
                .execute(&mut *connection).await.map_err(sql_error)?;
            position = block.end();
            if let PendingBlock::Events(block) = block {
                for record in block.iter() {
                    let record = record.map_err(storage)?;
                    if record.payload.is_checkpoint() {
                        sqlx::query("INSERT INTO agena_content_checkpoints (content_id,logical_sequence,block_start) VALUES (?,?,?)")
                            .bind(source.head.id.to_string()).bind(record.sequence as i64).bind(block.start() as i64).execute(&mut *connection).await.map_err(sql_error)?;
                    }
                }
            }
        }
        if position != source.head.committed {
            return Err(storage("staged content head has missing bytes"));
        }
        loss::persist(connection, source, &current).await?;
        sqlx::query("UPDATE agena_content_streams SET committed_head=?,lost_bytes=?,lost_packets=?,capture_state=?,capture_error=? WHERE id=?")
            .bind(source.head.committed as i64).bind(source.head.capture_loss.committed_bytes as i64).bind(source.head.capture_loss.committed_packets as i64).bind(commands::enum_text(&source.head.state)?).bind(&source.head.capture_error).bind(source.head.id.to_string())
            .execute(&mut *connection).await.map_err(sql_error)?;
        if source.head.state != CaptureState::Open {
            sqlx::query("UPDATE agena_entries SET revision=revision+1 WHERE id=?")
                .bind(source.head.entry.get())
                .execute(&mut *connection)
                .await
                .map_err(sql_error)?;
        }
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub(super) enum CheckpointError {
    #[error(transparent)]
    Uncommitted(#[from] KernelError),
    #[error("content commit outcome is unknown: {0}")]
    Unknown(String),
}

impl CheckpointError {
    pub fn requires_reopen(&self) -> bool {
        matches!(
            self,
            Self::Unknown(_) | Self::Uncommitted(KernelError::Closed)
        )
    }
}

pub(super) async fn checkpoint(
    database: &KernelDatabase,
    live: &LiveContent,
    timed: bool,
    closing: bool,
) -> std::result::Result<(), CheckpointError> {
    let _gates = live.admission_gates();
    let mut sources = live.checkpoint(timed, closing);
    if sources.is_empty() {
        return Ok(());
    }
    let mut connection = database
        .writer
        .get_sqlite_connection_pool()
        .acquire()
        .await
        .map_err(storage)?;
    let mut transaction = connection
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(storage)?;
    if let Err(error) = persist(&mut transaction, &mut sources).await {
        abort_transaction(transaction).await?;
        return Err(error.into());
    }
    // Ordinary checkpoints do not change entry views. Abandoned/shutdown
    // captures do advance revisions and must fence warm metadata/history reads.
    let fence = sources
        .iter()
        .any(|source| source.head.state != CaptureState::Open)
        .then(|| live.diagnostics().1.pause());

    if let Err(error) = transaction.commit().await {
        if !rejected_commit(&error) {
            live.diagnostics().1.close();
        }
        return Err(if rejected_commit(&error) {
            CheckpointError::Uncommitted(storage(error))
        } else {
            CheckpointError::Unknown(error.to_string())
        });
    }

    if let Some(fence) = &fence {
        fence.committed(None, &[]);
    }
    live.acknowledge(&sources);

    metrics::add(&live.diagnostics().0.checkpoint_transactions, 1);
    Ok(())
}

async fn authorize(
    connection: &mut SqliteConnection,
    session: SessionId,
    content: ContentId,
) -> Result<()> {
    let member: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agena_content_streams c JOIN agena_segment_entries m ON m.entry_id=c.entry_id JOIN agena_session_segments s ON s.segment_id=m.segment_id JOIN agena_sessions owner ON owner.id=s.session_id WHERE c.id=? AND s.session_id=? AND owner.deleted_at_ms IS NULL AND m.ordinal>=s.first_ordinal AND m.ordinal<s.end_ordinal)")
        .bind(content.to_string()).bind(session.get()).fetch_one(connection).await.map_err(sql_error)?;
    if !member {
        return Err(KernelError::Ownership);
    }
    Ok(())
}

async fn validate_boundary(
    connection: &mut SqliteConnection,
    boundary: &ContentStreamView,
) -> Result<ContentStreamView> {
    let current = stored_head(connection, boundary.id).await?;
    if boundary.epoch != current.epoch
        || boundary.entry != current.entry
        || boundary.kind != current.kind
        || boundary.accepted != boundary.committed
        || boundary.committed > current.committed
        || boundary.retained_start > boundary.committed
    {
        return Err(KernelError::ContentReset);
    }
    let mut frozen = boundary.clone();
    // A fixed input head does not prevent explicit retention. Report its new
    // floor rather than implying that an already pruned prefix is readable.
    frozen.retained_start = current.retained_start.max(frozen.retained_start);
    Ok(frozen)
}

pub(super) async fn range(
    backend: &Backend,
    session: SessionId,
    content: ContentId,
    after: Option<TextCursor>,
    limit: usize,
) -> Result<TextRange> {
    range_inner(backend, session, content, after, limit, None).await
}

pub(super) async fn range_at(
    backend: &Backend,
    session: SessionId,
    boundary: &ContentStreamView,
    after: Option<TextCursor>,
    limit: usize,
) -> Result<TextRange> {
    range_inner(backend, session, boundary.id, after, limit, Some(boundary)).await
}

async fn range_inner(
    backend: &Backend,
    session: SessionId,
    content: ContentId,
    after: Option<TextCursor>,
    limit: usize,
    boundary: Option<&ContentStreamView>,
) -> Result<TextRange> {
    let _read = backend
        .content_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| KernelError::Busy)?;
    let limit = limit.clamp(1, MAX_RANGE_BYTES);
    let start = after.map_or(0, |cursor| cursor.offset);
    // Capture live heads/tail BEFORE acquiring a SQLite snapshot. A checkpoint
    // that races this read can only add an already captured suffix to SQLite.
    let snapshot = boundary
        .is_none()
        .then(|| backend.live.snapshot(content, start, limit))
        .flatten();
    let mut transaction = backend
        .reader
        .get_sqlite_connection_pool()
        .begin()
        .await
        .map_err(storage)?;
    authorize(&mut transaction, session, content).await?;
    let (head, tail) = match (boundary, snapshot) {
        (Some(boundary), _) => (
            validate_boundary(&mut transaction, boundary).await?,
            Vec::new(),
        ),
        (_, Some(snapshot)) => snapshot,
        (_, None) => (stored_head(&mut transaction, content).await?, Vec::new()),
    };
    if head.kind != agena_domain::ContentKind::Text {
        return Err(invalid("byte text ranges require a text stream"));
    }
    if after.is_some_and(|cursor| {
        cursor.content != content || cursor.epoch != head.epoch || cursor.offset > head.accepted
    }) {
        return Err(KernelError::ContentReset);
    }
    if start < head.retained_start {
        return Err(KernelError::ContentGap {
            content,
            requested: start,
            retained_start: head.retained_start,
        });
    }
    let mut end = head.accepted.min(start.saturating_add(limit as u64));
    let committed_end = end.min(head.committed);
    let mut bytes = Vec::with_capacity((end - start) as usize);
    if start < committed_end {
        // One predecessor block plus forward indexed ranges; SQL never scans
        // all earlier blocks to answer a range near the end of a long stream.
        let first: Option<i64> = sqlx::query_scalar("SELECT logical_start FROM agena_content_blocks WHERE content_id=? AND logical_start<=? ORDER BY logical_start DESC LIMIT 1")
            .bind(content.to_string()).bind(start as i64).fetch_optional(&mut *transaction).await.map_err(sql_error)?;
        let first = first.ok_or_else(|| storage("missing committed text block"))?;
        let rows = sqlx::query("SELECT logical_start,logical_end,codec,digest,body FROM agena_content_blocks WHERE content_id=? AND logical_start>=? AND logical_start<? ORDER BY logical_start LIMIT 128")
            .bind(content.to_string()).bind(first).bind(committed_end as i64).fetch_all(&mut *transaction).await.map_err(sql_error)?;
        let mut position = start;
        let count = rows.len();
        for row in rows {
            let from: i64 = row.try_get("logical_start").map_err(storage)?;
            let to: i64 = row.try_get("logical_end").map_err(storage)?;
            let payload: Vec<u8> = row.try_get("body").map_err(storage)?;
            let digest: Vec<u8> = row.try_get("digest").map_err(storage)?;
            if payload.len() > DEFAULT_TEXT_BLOCK_BYTES
                || row.try_get::<String, _>("codec").map_err(storage)? != "utf8"
            {
                return Err(storage("invalid text block encoding/length"));
            }
            let block = ImmutableTextBlock::from_stored(
                from as u64,
                to as u64,
                payload,
                digest
                    .try_into()
                    .map_err(|_| storage("invalid text checksum length"))?,
            )
            .map_err(storage)?;
            if block.start() > position || block.end() <= position {
                return Err(storage("non-contiguous text blocks"));
            }
            let take = committed_end.min(block.end());
            bytes.extend_from_slice(block.range(position, take).map_err(storage)?);
            position = take;
        }
        if position != committed_end {
            if count == 128 {
                end = position;
            } else {
                return Err(storage("committed text range has a gap"));
            }
        }
    }
    if end > head.committed {
        bytes.extend_from_slice(&tail);
    }
    if bytes.len() != (end - start) as usize {
        return Err(storage("live text range has a gap"));
    }
    Ok(TextRange {
        next: TextCursor {
            content,
            epoch: head.epoch,
            offset: end,
        },
        has_more: end < head.accepted,
        stream: head,
        start,
        bytes,
    })
}

pub(super) async fn watch(
    database: &sea_orm::DatabaseConnection,
    live: &LiveContent,
    session: SessionId,
    content: ContentId,
) -> Result<ContentWatch> {
    let mut connection = database
        .get_sqlite_connection_pool()
        .acquire()
        .await
        .map_err(storage)?;
    authorize(&mut connection, session, content).await?;
    if let Some(watch) = live.subscribe(content) {
        return Ok(watch);
    }
    let head = stored_head(&mut connection, content).await?;
    let (sender, receiver) = watch::channel(head);
    drop(sender);
    Ok(ContentWatch::from_receiver(receiver))
}
