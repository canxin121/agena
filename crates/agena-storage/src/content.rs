//! Bounded live content with independent, batched persistence.
//!
//! This module has no database, transport, client, or tool dependency. A
//! backend commits record batches; observers read by cursor and cannot block
//! the source writer. The registry retains active resources only.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use agena_domain::{
    ContentChunk, ContentCursor, ContentId, ContentInput, ContentKind, ContentPage, ContentPayload,
    ContentRange, ContentResource, ContentState,
};
use async_trait::async_trait;
use tokio::sync::{Notify, watch};

use crate::store::StoreError;

/// Backend commits must be idempotent at the same cursor. A failed commit
/// leaves its preceding manifest readable; retries cannot duplicate records.
#[async_trait]
pub trait ContentBackend: Send + Sync {
    async fn describe(&self, id: ContentId) -> Result<ContentResource, StoreError> {
        Ok(self.read(id, None, 1).await?.resource)
    }

    async fn commit(
        &self,
        resource: ContentResource,
        chunks: &[Arc<ContentChunk>],
    ) -> Result<ContentResource, StoreError>;

    async fn read(
        &self,
        id: ContentId,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, StoreError>;

    /// Restore a sealed snapshot under a fresh id, including retention gaps.
    async fn restore(&self, archive: &ContentArchive) -> Result<(), StoreError>;

    /// Active writer ids and every Part reference must be protected.
    async fn prune(
        &self,
        protected: &std::collections::HashSet<ContentId>,
    ) -> Result<usize, StoreError>;

    async fn delete(&self, id: ContentId) -> Result<(), StoreError>;
}

mod archive;
pub use archive::ContentArchive;
mod text;

#[derive(Debug, Clone)]
pub struct ContentConfig {
    pub max_active_sources: usize,
    pub max_resident_bytes: usize,
    pub memory_bytes: usize,
    pub pending_bytes: usize,
    pub chunk_bytes: usize,
    pub flush_bytes: usize,
    pub flush_interval: Duration,
}

impl Default for ContentConfig {
    fn default() -> Self {
        Self {
            max_active_sources: 128,
            max_resident_bytes: 128 * 1024 * 1024,
            memory_bytes: 1024 * 1024,
            pending_bytes: 512 * 1024,
            chunk_bytes: 64 * 1024,
            flush_bytes: 64 * 1024,
            flush_interval: Duration::from_millis(500),
        }
    }
}

#[derive(Clone)]
pub struct ContentHub(Arc<HubInner>);

/// A bounded textual projection for model, export and diagnostic consumers.
/// Loss and truncation stay explicit; facts are never hydrated in place.
pub struct ContentText {
    pub text: String,
    pub gap: bool,
    pub truncated: bool,
}

struct HubInner {
    backend: Arc<dyn ContentBackend>,
    config: ContentConfig,
    active: Mutex<HashMap<ContentId, Arc<ActiveContent>>>,
    slots: Arc<tokio::sync::Semaphore>,
    resident_bytes: std::sync::atomic::AtomicUsize,
    capacity: Notify,
    lifecycle: tokio::sync::RwLock<()>,
}

struct ActiveContent {
    state: Mutex<ContentBuffer>,
    commit_gate: tokio::sync::Mutex<()>,
    changes: watch::Sender<ContentResource>,
    flush_requested: Notify,
    hub: Weak<HubInner>,
}

struct ContentBuffer {
    slot: Option<tokio::sync::OwnedSemaphorePermit>,
    resource: ContentResource,
    records: VecDeque<Arc<ContentChunk>>,
    memory_bytes: usize,
    pending: Vec<Arc<ContentChunk>>,
    uncommitted_bytes: usize,
    closing: Option<ContentState>,
    owner_abandoned: bool,
    persistence_error: Option<String>,
    capture_failure: Option<String>,
    loss_version: u64,
    committed_loss_version: u64,
    terminal_frame: Option<(ContentCursor, u16, u16, bool)>,
    document: Option<agena_domain::ContentDocument>,
    document_cursor: Option<ContentCursor>,
    document_bytes: usize,
    document_events: u32,
    document_event_bytes: usize,
}

/// Clones share the same write authority. Dropping the last authority seals
/// an abandoned source as interrupted and leaves its flush task responsible
/// for draining the final batch.
#[derive(Clone)]
pub struct ContentWriter(Arc<WriterLease>);

impl std::fmt::Debug for ContentWriter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContentWriter")
            .field("resource", &self.resource())
            .finish()
    }
}

struct WriterLease(
    Arc<ActiveContent>,
    tokio::runtime::Handle,
    std::sync::atomic::AtomicBool,
);

impl Drop for WriterLease {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().expect("content buffer lock");
        if state.resource.state == ContentState::Active {
            state.owner_abandoned = true;
            state.closing.get_or_insert(ContentState::Interrupted);
            drop(state);
            self.0.flush_requested.notify_one();
        }
    }
}

impl ContentHub {
    /// Current accounting for the shared live replay and pending buffers.
    pub fn resident_bytes(&self) -> usize {
        self.0
            .resident_bytes
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub async fn read_text(
        &self,
        id: ContentId,
        max_bytes: usize,
    ) -> Result<ContentText, StoreError> {
        let resource = self.describe(id).await?;
        let mut cursor = ContentCursor {
            sequence: 0,
            ..resource.cursor
        };
        let mut output = ContentText {
            text: String::new(),
            gap: false,
            truncated: false,
        };
        let limit = max_bytes.min(8 * 1024 * 1024);
        loop {
            let remaining = limit.saturating_sub(output.text.len());
            if remaining == 0 {
                output.truncated = cursor.sequence < resource.cursor.sequence;
                break;
            }
            let page = self
                .read(id, Some(cursor), remaining.min(64 * 1024))
                .await?;
            output.gap |= page.gap || page.resource.dropped_bytes > 0;
            for chunk in page.chunks {
                let Some(text) = chunk.payload.text_content() else {
                    continue;
                };
                let mut end = text.len().min(limit.saturating_sub(output.text.len()));
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                output.text.push_str(&text[..end]);
                if end < text.len() {
                    output.truncated = true;
                    return Ok(output);
                }
            }
            if !page.has_more || page.next_cursor == cursor {
                break;
            }
            cursor = page.next_cursor;
        }
        Ok(output)
    }

    pub fn new(backend: Arc<dyn ContentBackend>, config: ContentConfig) -> Self {
        assert!(config.max_active_sources > 0);
        assert!(config.max_resident_bytes >= config.memory_bytes);
        assert!(config.chunk_bytes > 0);
        assert!(config.pending_bytes >= config.chunk_bytes + RECORD_OVERHEAD);
        assert!(config.memory_bytes >= config.pending_bytes);
        assert!(!config.flush_interval.is_zero());
        Self(Arc::new(HubInner {
            backend,
            slots: Arc::new(tokio::sync::Semaphore::new(config.max_active_sources)),
            resident_bytes: std::sync::atomic::AtomicUsize::new(0),
            capacity: Notify::new(),
            lifecycle: tokio::sync::RwLock::new(()),
            config,
            active: Mutex::new(HashMap::new()),
        }))
    }

    pub fn in_memory() -> Self {
        Self::new(
            Arc::new(MemoryContentBackend::default()),
            ContentConfig::default(),
        )
    }

    pub async fn open(
        &self,
        owner_session_id: i64,
        part_id: i64,
        kind: ContentKind,
    ) -> Result<ContentWriter, StoreError> {
        let _lifecycle = self.0.lifecycle.read().await;
        if owner_session_id <= 0 || part_id <= 0 {
            return Err(StoreError::InvalidState(
                "content requires durable ownership".into(),
            ));
        }
        let slot = self.0.slots.clone().try_acquire_owned().map_err(|_| {
            StoreError::InvalidState("active content source budget exhausted".into())
        })?;
        let id = ContentId::new();
        let cursor = ContentCursor {
            epoch: id.0,
            sequence: 0,
        };
        let resource = ContentResource {
            resource_id: id,
            owner_session_id,
            part_id,
            kind,
            state: ContentState::Active,
            cursor,
            committed_cursor: cursor,
            total_bytes: 0,
            dropped_bytes: 0,
            retained_ranges: Vec::new(),
            capture_error: None,
        };
        let resource = self.0.backend.commit(resource, &[]).await?;
        let (changes, _) = watch::channel(resource.clone());
        let source = Arc::new(ActiveContent {
            state: Mutex::new(ContentBuffer {
                slot: Some(slot),
                resource,
                records: VecDeque::new(),
                memory_bytes: 0,
                pending: Vec::new(),
                uncommitted_bytes: 0,
                closing: None,
                owner_abandoned: false,
                persistence_error: None,
                capture_failure: None,
                loss_version: 0,
                committed_loss_version: 0,
                terminal_frame: None,
                document: None,
                document_cursor: None,
                document_bytes: 0,
                document_events: 0,
                document_event_bytes: 0,
            }),
            commit_gate: tokio::sync::Mutex::new(()),
            changes,
            flush_requested: Notify::new(),
            hub: Arc::downgrade(&self.0),
        });
        self.0
            .active
            .lock()
            .expect("content registry lock")
            .insert(id, source.clone());
        let weak = Arc::downgrade(&source);
        let interval = self.0.config.flush_interval;
        tokio::spawn(async move {
            loop {
                let Some(source) = weak.upgrade() else { break };
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {},
                    _ = source.flush_requested.notified() => {},
                }
                if let Err(error) = source.flush().await {
                    tracing::error!(%error, "content batch commit failed");
                    let abandoned = source
                        .state
                        .lock()
                        .expect("content buffer lock")
                        .owner_abandoned;
                    if abandoned {
                        if let Err(error) = source.recover_or_discard_tail().await {
                            tracing::error!(%error, "abandoned content finalization failed");
                        }
                        break;
                    }
                    // Storage failure must not turn a notification permit
                    // into a tight retry loop.
                    tokio::time::sleep(interval).await;
                }
                if source
                    .state
                    .lock()
                    .expect("content buffer lock")
                    .resource
                    .state
                    != ContentState::Active
                {
                    source.retire();
                    break;
                }
            }
        });
        Ok(ContentWriter(Arc::new(WriterLease(
            source,
            tokio::runtime::Handle::current(),
            std::sync::atomic::AtomicBool::new(false),
        ))))
    }

    /// Register before reading the bootstrap snapshot. The receiver's latest
    /// cursor then covers every append that races with that read.
    pub fn subscribe(&self, id: ContentId) -> Option<watch::Receiver<ContentResource>> {
        self.0
            .active
            .lock()
            .expect("content registry lock")
            .get(&id)
            .map(|source| source.changes.subscribe())
    }

    pub async fn describe(&self, id: ContentId) -> Result<ContentResource, StoreError> {
        if let Some(source) = self
            .0
            .active
            .lock()
            .expect("content registry lock")
            .get(&id)
            .cloned()
        {
            return Ok(source
                .state
                .lock()
                .expect("content buffer lock")
                .resource
                .clone());
        }
        let mut resource = self.0.backend.describe(id).await?;
        if resource.state == ContentState::Active {
            resource.state = ContentState::Interrupted;
        }
        Ok(resource)
    }

    pub async fn read(
        &self,
        id: ContentId,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, StoreError> {
        let max_bytes = max_bytes.clamp(1, 1024 * 1024);
        let source = self
            .0
            .active
            .lock()
            .expect("content registry lock")
            .get(&id)
            .cloned();
        if let Some(source) = &source {
            let state = source.state.lock().expect("content buffer lock");
            validate_cursor(&state.resource, after)?;
            if after.is_none()
                && matches!(
                    state.resource.kind,
                    ContentKind::Terminal | ContentKind::Structured | ContentKind::Document
                )
                && let Some(snapshot) = state.records.iter().rev().find(|chunk| {
                    matches!(
                        chunk.payload,
                        ContentPayload::Terminal { .. } | ContentPayload::StructuredSnapshot { .. }
                    )
                })
            {
                let start = ContentCursor {
                    sequence: snapshot.cursor.sequence - 1,
                    ..snapshot.cursor
                };
                return Ok(page_from_records(
                    &state.resource,
                    state.records.iter(),
                    Some(start),
                    max_bytes,
                ));
            }
            if (after.is_none()
                && !matches!(
                    state.resource.kind,
                    ContentKind::Terminal | ContentKind::Structured | ContentKind::Document
                )
                && (!state.records.is_empty() || state.resource.cursor.sequence == 0))
                || after.is_some_and(|cursor| {
                    state
                        .records
                        .front()
                        .map_or(cursor.sequence == state.resource.cursor.sequence, |first| {
                            cursor.sequence >= first.cursor.sequence - 1
                        })
                })
            {
                return Ok(page_from_records(
                    &state.resource,
                    state.records.iter(),
                    after,
                    max_bytes,
                ));
            }
        }
        let mut page = self.0.backend.read(id, after, max_bytes).await?;
        if let Some(source) = source {
            let state = source.state.lock().expect("content buffer lock");
            if !page.has_more && page.next_cursor.sequence < state.resource.cursor.sequence {
                let remaining = max_bytes.saturating_sub(
                    page.chunks
                        .iter()
                        .map(|chunk| chunk.payload.byte_len())
                        .sum::<usize>(),
                );
                if remaining > 0 {
                    let tail = page_from_records(
                        &state.resource,
                        state.records.iter(),
                        Some(page.next_cursor),
                        remaining,
                    );
                    page.gap |= tail.gap;
                    page.chunks.extend(tail.chunks);
                    page.next_cursor = tail.next_cursor;
                }
            }
            page.resource = state.resource.clone();
            page.has_more = page.next_cursor.sequence < page.resource.cursor.sequence;
        } else if page.resource.state == ContentState::Active {
            // No current owner exists. Historical committed bytes remain
            // readable, but the preceding process cannot still be streaming.
            page.resource.state = ContentState::Interrupted;
        }
        Ok(page)
    }
}

const RECORD_OVERHEAD: usize = 64;

impl HubInner {
    /// Reclaim only committed replay data. The durable backend can satisfy a
    /// slow observer, while uncommitted records remain owned by the source.
    /// This runs only under global pressure and never waits on another writer.
    fn reclaim_committed(&self, required: usize) {
        use std::sync::atomic::Ordering;
        let sources = self
            .active
            .lock()
            .expect("content registry lock")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for source in sources {
            let Ok(mut state) = source.state.try_lock() else {
                continue;
            };
            let before = state.memory_bytes;
            while self
                .resident_bytes
                .load(Ordering::Acquire)
                .saturating_add(required)
                > self.config.max_resident_bytes
                && state.records.front().is_some_and(|chunk| {
                    chunk.cursor.sequence <= state.resource.committed_cursor.sequence
                })
            {
                let chunk = state.records.pop_front().expect("committed record exists");
                let cost = chunk.payload.byte_len() + RECORD_OVERHEAD;
                state.memory_bytes -= cost;
                self.resident_bytes.fetch_sub(cost, Ordering::AcqRel);
            }
            if before != state.memory_bytes {
                self.capacity.notify_waiters();
            }
            if self
                .resident_bytes
                .load(Ordering::Acquire)
                .saturating_add(required)
                <= self.config.max_resident_bytes
            {
                break;
            }
        }
    }
}

impl PartialEq for ContentWriter {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for ContentWriter {}

impl ContentWriter {
    /// An execution owner explicitly assumes responsibility for sealing this
    /// source after the invocation that launched it has returned.
    pub fn delegate_lifecycle(&self) -> Self {
        self.0.2.store(true, std::sync::atomic::Ordering::Release);
        self.clone()
    }

    pub fn lifecycle_is_delegated(&self) -> bool {
        self.0.2.load(std::sync::atomic::Ordering::Acquire)
    }

    /// For source producers running on dedicated OS threads, such as the PTY
    /// driver. Async producers always use append directly.
    pub fn append_blocking(&self, payload: ContentInput) -> Result<ContentCursor, StoreError> {
        self.0.1.block_on(self.append(payload))
    }

    pub fn finish_blocking(&self) -> Result<ContentResource, StoreError> {
        self.0.1.block_on(self.finish())
    }

    pub fn finalize_blocking(
        &self,
        final_state: ContentState,
    ) -> Result<ContentResource, StoreError> {
        self.0.1.block_on(self.finalize(final_state))
    }

    pub fn resource(&self) -> ContentResource {
        self.0
            .0
            .state
            .lock()
            .expect("content buffer lock")
            .resource
            .clone()
    }

    pub async fn append_text(&self, text: &str) -> Result<ContentCursor, StoreError> {
        let limit = self
            .0
            .0
            .hub
            .upgrade()
            .ok_or_else(|| StoreError::InvalidState("content hub stopped".into()))?
            .config
            .chunk_bytes;
        let mut remaining = text;
        let mut cursor = self.resource().cursor;
        while !remaining.is_empty() {
            let mut end = remaining.len().min(limit);
            while !remaining.is_char_boundary(end) {
                end -= 1;
            }
            if end == 0 {
                return Err(StoreError::InvalidState(
                    "chunk budget cannot hold a UTF-8 character".into(),
                ));
            }
            cursor = self
                .append(ContentInput::Text {
                    text: remaining[..end].to_owned(),
                })
                .await?;
            remaining = &remaining[end..];
        }
        Ok(cursor)
    }

    /// Lossless producers may await persistence capacity. Observers never
    /// participate in this wait. OS pipe/PTY readers use `capture` instead.
    pub async fn append(&self, payload: ContentInput) -> Result<ContentCursor, StoreError> {
        let source = &self.0.0;
        let hub = source
            .hub
            .upgrade()
            .ok_or_else(|| StoreError::InvalidState("content hub stopped".into()))?;
        let bytes = payload.byte_len();
        validate_input(&payload, hub.config.chunk_bytes)?;
        let mut payload = Some(payload);
        let mut required = bytes + RECORD_OVERHEAD;
        let mut changes = source.changes.subscribe();
        loop {
            let capacity = hub.capacity.notified();
            tokio::pin!(capacity);
            capacity.as_mut().enable();
            if let Some(cursor) = self.append_ready(&hub, &mut payload, bytes, &mut required)? {
                return Ok(cursor);
            }
            hub.reclaim_committed(required);
            if let Some(cursor) = self.append_ready(&hub, &mut payload, bytes, &mut required)? {
                return Ok(cursor);
            }
            source.flush_requested.notify_one();
            tokio::select! {
                result = changes.changed() => result.map_err(|_| StoreError::InvalidState("content source stopped".into()))?,
                _ = capacity => {},
            }
        }
    }

    /// Never wait on disk, a transport or an observer while draining an OS
    /// pipe. The producer records any rejected bytes as capture loss.
    pub fn capture(&self, payload: ContentInput) -> Result<ContentCursor, StoreError> {
        let hub = self
            .0
            .0
            .hub
            .upgrade()
            .ok_or_else(|| StoreError::InvalidState("content hub stopped".into()))?;
        let bytes = payload.byte_len();
        validate_input(&payload, hub.config.chunk_bytes)?;
        let mut payload = Some(payload);
        let mut required = bytes + RECORD_OVERHEAD;
        let mut result = self.append_ready(&hub, &mut payload, bytes, &mut required)?;
        if result.is_none() {
            hub.reclaim_committed(required);
            result = self.append_ready(&hub, &mut payload, bytes, &mut required)?;
        }
        if result.is_none() {
            self.0.0.flush_requested.notify_one();
        }
        result.ok_or_else(|| StoreError::InvalidState("content capture capacity exhausted".into()))
    }

    fn append_ready(
        &self,
        hub: &HubInner,
        payload: &mut Option<ContentInput>,
        mut bytes: usize,
        required: &mut usize,
    ) -> Result<Option<ContentCursor>, StoreError> {
        let source = &self.0.0;
        let mut state = source.state.lock().expect("content buffer lock");
        if state.closing.is_some() || state.resource.state != ContentState::Active {
            return Err(StoreError::InvalidState("content source is sealed".into()));
        }
        let value = payload.as_ref().expect("unconsumed payload");
        if !state.resource.kind.accepts(value.kind()) {
            return Err(StoreError::InvalidState(
                "content payload kind differs from source".into(),
            ));
        }
        let mut document = None;
        let mut checkpoint = false;
        let mut checkpoint_payload = None;
        if let ContentInput::Structured { event } = value {
            let empty = agena_domain::ContentDocument::default();
            let next = state
                .document
                .as_ref()
                .unwrap_or(&empty)
                .updated(event)
                .map_err(|error| StoreError::Constraint(error.into()))?;
            checkpoint = state.document.is_none()
                || state.document_events >= 31
                || state.document_event_bytes.saturating_add(bytes) >= 64 * 1024;
            if checkpoint {
                checkpoint_payload = Some(ContentPayload::StructuredSnapshot {
                    document: next.clone(),
                });
            }
            document = Some(next);
        }
        if let Some(prepared) = checkpoint_payload.as_ref() {
            validate_payload(prepared, hub.config.chunk_bytes)?;
            bytes = prepared.byte_len();
        }
        if let ContentInput::TerminalPatch {
            base_cursor,
            screen,
            ..
        } = value
            && state.terminal_frame
                != Some((
                    *base_cursor,
                    screen.rows,
                    screen.cols,
                    screen.alternate_screen,
                ))
        {
            return Err(StoreError::Constraint(
                "terminal patch does not extend the current screen".into(),
            ));
        }
        if let Some(error) = &state.persistence_error {
            return Err(StoreError::Io(error.clone()));
        }
        if bytes == 0 {
            return Ok(Some(state.resource.cursor));
        }
        let cost = bytes + RECORD_OVERHEAD;
        let document_bytes = document.as_ref().map_or(
            state.document_bytes,
            agena_domain::ContentDocument::byte_len,
        );
        let reservation = cost + document_bytes.saturating_sub(state.document_bytes);
        *required = reservation;
        if reservation > hub.config.max_resident_bytes {
            return Err(StoreError::Constraint(
                "content frame exceeds the global resident budget".into(),
            ));
        }
        // One valid screen is an atomic frame, with a separate bounded budget.
        // Ordinary text/log records retain their small source budget.
        let terminal = state.resource.kind == ContentKind::Terminal;
        let pending_limit = if terminal {
            hub.config
                .pending_bytes
                .max(agena_domain::MAX_TERMINAL_FRAME_BYTES + RECORD_OVERHEAD)
        } else if document.is_some() {
            hub.config
                .pending_bytes
                .max(agena_domain::MAX_DOCUMENT_BYTES + RECORD_OVERHEAD)
        } else {
            hub.config.pending_bytes
        };
        if state.uncommitted_bytes + cost > pending_limit {
            return Ok(None);
        }
        let sequence = state
            .resource
            .cursor
            .sequence
            .checked_add(1)
            .ok_or_else(|| StoreError::InvalidState("content sequence exhausted".into()))?;
        let total = state
            .resource
            .total_bytes
            .checked_add(bytes as u64)
            .ok_or_else(|| StoreError::InvalidState("content byte count exhausted".into()))?;
        use std::sync::atomic::Ordering;
        if hub
            .resident_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(reservation)
                    .filter(|next| *next <= hub.config.max_resident_bytes)
            })
            .is_err()
        {
            return Ok(None);
        }
        state.resource.cursor.sequence = sequence;
        state.resource.total_bytes = total;
        if let Some(document) = document {
            if document_bytes < state.document_bytes {
                hub.resident_bytes
                    .fetch_sub(state.document_bytes - document_bytes, Ordering::AcqRel);
            }
            state.document = Some(document);
            state.document_bytes = document_bytes;
            if checkpoint {
                state.document_events = 0;
                state.document_event_bytes = 0;
            } else {
                state.document_events += 1;
                state.document_event_bytes += bytes;
            }
        }
        extend_ranges(&mut state.resource.retained_ranges, sequence);
        let chunk = Arc::new(ContentChunk {
            cursor: state.resource.cursor,
            captured_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis() as i64)
                .unwrap_or(0),
            payload: {
                let event = payload.take().expect("unconsumed payload");
                match checkpoint_payload {
                    Some(prepared) => prepared,
                    None => event
                        .into_payload(state.document_cursor)
                        .map_err(|error| StoreError::Constraint(error.into()))?,
                }
            },
        });
        if matches!(
            chunk.payload,
            ContentPayload::Structured { .. } | ContentPayload::StructuredSnapshot { .. }
        ) {
            state.document_cursor = Some(chunk.cursor);
        }
        if let ContentPayload::Terminal { screen } | ContentPayload::TerminalPatch { screen, .. } =
            &chunk.payload
        {
            state.terminal_frame = Some((
                chunk.cursor,
                screen.rows,
                screen.cols,
                screen.alternate_screen,
            ));
        }
        state.records.push_back(chunk.clone());
        state.pending.push(chunk);
        state.memory_bytes += cost;
        state.uncommitted_bytes += cost;
        let memory_limit = if terminal {
            hub.config
                .memory_bytes
                .max(agena_domain::MAX_TERMINAL_FRAME_BYTES + RECORD_OVERHEAD)
        } else {
            hub.config.memory_bytes
        };
        let before = state.memory_bytes;
        trim_memory(&mut state, memory_limit);
        if before != state.memory_bytes {
            hub.resident_bytes.fetch_sub(
                before - state.memory_bytes,
                std::sync::atomic::Ordering::AcqRel,
            );
            hub.capacity.notify_waiters();
        }
        source.changes.send_replace(state.resource.clone());
        if state.uncommitted_bytes >= hub.config.flush_bytes {
            source.flush_requested.notify_one();
        }
        Ok(Some(state.resource.cursor))
    }

    /// Producers keep draining their input after a storage/capture error. The
    /// descriptor remembers loss even if a later commit succeeds.
    pub fn record_loss(&self, bytes: usize, error: &dyn std::fmt::Display) {
        let source = &self.0.0;
        let mut state = source.state.lock().expect("content buffer lock");
        if state.resource.state != ContentState::Active || state.closing.is_some() {
            return;
        }
        state.resource.dropped_bytes = state.resource.dropped_bytes.saturating_add(bytes as u64);
        state.resource.total_bytes = state.resource.total_bytes.saturating_add(bytes as u64);
        state
            .capture_failure
            .get_or_insert_with(|| capture_error_message(error));
        state.loss_version = state.loss_version.saturating_add(1);
        state.resource.capture_error = state.capture_failure.clone();
        source.changes.send_replace(state.resource.clone());
        // Loss descriptors follow the same timer as content batches. A failing
        // source can report thousands of rejected chunks without thousands of
        // manifest writes, including when no further bytes are accepted.
    }

    pub async fn finish(&self) -> Result<ContentResource, StoreError> {
        self.seal(ContentState::Complete).await
    }

    pub async fn interrupt(&self) -> Result<ContentResource, StoreError> {
        self.seal(ContentState::Interrupted).await
    }

    /// Execution owners call this after draining producers. Try to preserve
    /// every admitted record; if storage cannot accept the tail, publish an
    /// explicit interrupted descriptor and release the source's live budget.
    /// This never changes the independent tool/process outcome.
    pub async fn finalize(&self, final_state: ContentState) -> Result<ContentResource, StoreError> {
        if final_state == ContentState::Active {
            return Err(StoreError::InvalidState(
                "cannot finalize active content".into(),
            ));
        }
        match self.seal(final_state).await {
            Ok(resource) => Ok(resource),
            Err(_) => {
                let source = self.0.0.clone();
                tokio::spawn(async move { source.recover_or_discard_tail().await })
                    .await
                    .map_err(|error| {
                        StoreError::Io(format!("content finalization task failed: {error}"))
                    })?
            }
        }
    }

    async fn seal(&self, mut final_state: ContentState) -> Result<ContentResource, StoreError> {
        let source = &self.0.0;
        {
            let mut state = source.state.lock().expect("content buffer lock");
            if state.capture_failure.is_some() && final_state == ContentState::Complete {
                final_state = ContentState::Interrupted;
            }
            if state.resource.state == final_state {
                return Ok(state.resource.clone());
            }
            if state.resource.state != ContentState::Active
                || state.closing.is_some_and(|closing| closing != final_state)
            {
                return Err(StoreError::InvalidState(
                    "interrupted content cannot complete".into(),
                ));
            }
            state.closing = Some(final_state);
        }
        // If this future is cancelled, the registered worker still seals and
        // commits the tail. Completion never depends on another append.
        source.flush_requested.notify_one();
        source.flush().await?;
        source.retire();
        Ok(self.resource())
    }
}

impl ActiveContent {
    /// A publication may have succeeded before its final directory sync
    /// failed. Retry once under the same commit gate so the backend can
    /// reconcile that boundary before declaring any admitted records lost.
    /// Permanent failure is bounded once an execution owner has departed.
    async fn recover_or_discard_tail(self: &Arc<Self>) -> Result<ContentResource, StoreError> {
        match self.flush().await {
            Ok(()) => {
                let resource = self
                    .state
                    .lock()
                    .expect("content buffer lock")
                    .resource
                    .clone();
                self.retire();
                Ok(resource)
            }
            Err(error) => {
                self.discard_unpersistable_tail(capture_error_message(&error))
                    .await
            }
        }
    }

    async fn discard_unpersistable_tail(
        &self,
        error: String,
    ) -> Result<ContentResource, StoreError> {
        let _gate = self.commit_gate.lock().await;
        let hub = self
            .hub
            .upgrade()
            .ok_or_else(|| StoreError::InvalidState("content hub stopped".into()))?;
        let resource = {
            let mut state = self.state.lock().expect("content buffer lock");
            if state.resource.state != ContentState::Active {
                return Ok(state.resource.clone());
            }
            let lost = state
                .pending
                .iter()
                .map(|chunk| chunk.payload.byte_len() as u64)
                .sum::<u64>()
                .min(
                    state
                        .resource
                        .total_bytes
                        .saturating_sub(state.resource.dropped_bytes),
                );
            state.resource.dropped_bytes += lost;
            state.capture_failure.get_or_insert(error);
            state.resource.capture_error = state.capture_failure.clone();
            state.closing = Some(ContentState::Interrupted);
            state.pending.clear();
            state.uncommitted_bytes = 0;
            let mut resource = state.resource.clone();
            resource.state = ContentState::Interrupted;
            // This manifest accounts for the final position and its explicit
            // missing ranges. Interrupted capture never claims complete bytes.
            resource.committed_cursor = resource.cursor;
            resource
        };
        let result = hub.backend.commit(resource.clone(), &[]).await;
        {
            let mut state = self.state.lock().expect("content buffer lock");
            match &result {
                Ok(committed) => state.resource = committed.clone(),
                Err(_) => {
                    state.resource.state = ContentState::Interrupted;
                    state.resource.capture_error = resource.capture_error;
                }
            }
            self.changes.send_replace(state.resource.clone());
        }
        self.retire();
        result
    }

    async fn flush(self: &Arc<Self>) -> Result<(), StoreError> {
        // Commit ownership outlives an individual waiter. Cancelling finish
        // or a disconnected request cannot discard a batch already taken
        // from the live buffer.
        let source = self.clone();
        tokio::spawn(async move { source.commit_batch().await })
            .await
            .map_err(|error| StoreError::Io(format!("content commit task failed: {error}")))?
    }

    async fn commit_batch(&self) -> Result<(), StoreError> {
        let _gate = self.commit_gate.lock().await;
        let Some(hub) = self.hub.upgrade() else {
            return Ok(());
        };
        let (mut resource, batch, loss_version) = {
            let mut state = self.state.lock().expect("content buffer lock");
            if state.resource.state != ContentState::Active
                || (state.pending.is_empty()
                    && state.closing.is_none()
                    && state.loss_version == state.committed_loss_version)
            {
                return Ok(());
            }
            let mut resource = state.resource.clone();
            resource.committed_cursor = resource.cursor;
            resource.capture_error = state.capture_failure.clone();
            if let Some(closing) = state.closing {
                resource.state = if state.capture_failure.is_some() {
                    ContentState::Interrupted
                } else {
                    closing
                };
            }
            (
                resource,
                std::mem::take(&mut state.pending),
                state.loss_version,
            )
        };
        match hub.backend.commit(resource.clone(), &batch).await {
            Ok(committed) => {
                let mut state = self.state.lock().expect("content buffer lock");
                state.uncommitted_bytes -= batch
                    .iter()
                    .map(|chunk| chunk.payload.byte_len() + RECORD_OVERHEAD)
                    .sum::<usize>();
                state.resource.committed_cursor = committed.committed_cursor;
                state.resource.state = committed.state;
                state.persistence_error = None;
                state.committed_loss_version = loss_version;
                state.resource.capture_error =
                    state.capture_failure.clone().or(committed.capture_error);
                resource.retained_ranges = committed.retained_ranges;
                if committed.cursor.sequence < state.resource.cursor.sequence {
                    let first = committed.cursor.sequence + 1;
                    let last = state.resource.cursor.sequence;
                    merge_range(&mut resource.retained_ranges, ContentRange { first, last });
                }
                state.resource.retained_ranges = resource.retained_ranges;
                let memory_limit = if state.resource.kind == ContentKind::Terminal {
                    hub.config
                        .memory_bytes
                        .max(agena_domain::MAX_TERMINAL_FRAME_BYTES + RECORD_OVERHEAD)
                } else {
                    hub.config.memory_bytes
                };
                let before = state.memory_bytes;
                trim_memory(&mut state, memory_limit);
                hub.resident_bytes.fetch_sub(
                    before - state.memory_bytes,
                    std::sync::atomic::Ordering::AcqRel,
                );
                hub.capacity.notify_waiters();
                self.changes.send_replace(state.resource.clone());
                Ok(())
            }
            Err(error) => {
                let mut state = self.state.lock().expect("content buffer lock");
                let mut pending = batch;
                pending.append(&mut state.pending);
                state.pending = pending;
                state.persistence_error = Some(capture_error_message(&error));
                state.resource.capture_error = state
                    .capture_failure
                    .clone()
                    .or_else(|| state.persistence_error.clone());
                self.changes.send_replace(state.resource.clone());
                Err(error)
            }
        }
    }

    fn retire(&self) {
        if let Some(hub) = self.hub.upgrade() {
            let id = {
                let mut state = self.state.lock().expect("content buffer lock");
                // Completed sources are read through committed segments. The
                // process may retain its descriptor/writer without pinning
                // live buffers or occupying an active-source slot forever.
                hub.resident_bytes.fetch_sub(
                    state.memory_bytes + state.document_bytes,
                    std::sync::atomic::Ordering::AcqRel,
                );
                state.records.clear();
                state.memory_bytes = 0;
                state.document = None;
                state.document_bytes = 0;
                state.slot.take();
                hub.capacity.notify_waiters();
                state.resource.resource_id
            };
            hub.active
                .lock()
                .expect("content registry lock")
                .remove(&id);
        }
    }
}

fn trim_memory(state: &mut ContentBuffer, budget: usize) {
    while state.memory_bytes > budget
        && state
            .records
            .front()
            .is_some_and(|chunk| chunk.cursor.sequence <= state.resource.committed_cursor.sequence)
    {
        let removed = state.records.pop_front().expect("front exists");
        state.memory_bytes -= removed.payload.byte_len() + RECORD_OVERHEAD;
    }
}

pub fn validate_cursor(
    resource: &ContentResource,
    after: Option<ContentCursor>,
) -> Result<(), StoreError> {
    if let Some(cursor) = after {
        if cursor.epoch != resource.cursor.epoch {
            return Err(StoreError::Conflict(
                "content cursor belongs to a different epoch".into(),
            ));
        }
        if cursor.sequence > resource.cursor.sequence {
            return Err(StoreError::Conflict(
                "content cursor is ahead of the resource".into(),
            ));
        }
    }
    Ok(())
}

fn capture_error_message(error: &dyn std::fmt::Display) -> String {
    let mut message = error.to_string();
    if message.len() > 1024 {
        let mut end = 1024;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    message
}

/// A sealed loss descriptor can cover positions whose bytes were never
/// persisted. Its retained ranges and error make this different from success.
pub fn accounts_for_lost_tail(resource: &ContentResource, retained_cursor: u64) -> bool {
    resource.state == ContentState::Interrupted
        && resource.capture_error.is_some()
        && resource.dropped_bytes > 0
        && resource.dropped_bytes <= resource.total_bytes
        && resource.cursor.sequence > retained_cursor
}

impl Drop for ActiveContent {
    fn drop(&mut self) {
        if let Some(hub) = self.hub.upgrade() {
            let state = self
                .state
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            hub.resident_bytes.fetch_sub(
                state.memory_bytes + state.document_bytes,
                std::sync::atomic::Ordering::AcqRel,
            );
        }
    }
}

fn validate_payload(payload: &ContentPayload, ordinary_limit: usize) -> Result<(), StoreError> {
    payload
        .validate()
        .map_err(|error| StoreError::Constraint(error.to_owned()))?;
    let limit = if matches!(
        payload,
        ContentPayload::Terminal { .. } | ContentPayload::TerminalPatch { .. }
    ) {
        agena_domain::MAX_TERMINAL_FRAME_BYTES
    } else if matches!(payload, ContentPayload::StructuredSnapshot { .. }) {
        agena_domain::MAX_DOCUMENT_BYTES
    } else {
        ordinary_limit
    };
    if payload.byte_len() > limit {
        return Err(StoreError::InvalidState(
            "content chunk exceeds its byte budget".into(),
        ));
    }
    Ok(())
}

fn validate_input(payload: &ContentInput, ordinary_limit: usize) -> Result<(), StoreError> {
    payload
        .validate()
        .map_err(|error| StoreError::Constraint(error.into()))?;
    let limit = if payload.kind() == ContentKind::Terminal {
        agena_domain::MAX_TERMINAL_FRAME_BYTES
    } else {
        ordinary_limit
    };
    if payload.byte_len() > limit {
        return Err(StoreError::Constraint(
            "content input exceeds the record budget".into(),
        ));
    }
    Ok(())
}

fn extend_ranges(ranges: &mut Vec<ContentRange>, sequence: u64) {
    merge_range(
        ranges,
        ContentRange {
            first: sequence,
            last: sequence,
        },
    );
}

pub fn merge_range(ranges: &mut Vec<ContentRange>, range: ContentRange) {
    if let Some(last) = ranges.last_mut()
        && last.last.checked_add(1) == Some(range.first)
    {
        last.last = range.last;
        return;
    }
    ranges.push(range);
}

/// Shared bounded-page semantics for the memory and concrete file backend.
pub fn page_from_records<'a>(
    resource: &ContentResource,
    records: impl DoubleEndedIterator<Item = &'a Arc<ContentChunk>>,
    after: Option<ContentCursor>,
    max_bytes: usize,
) -> ContentPage {
    let mut chunks = Vec::new();
    let mut bytes = 0;
    if let Some(after) = after {
        for chunk in records.filter(|chunk| chunk.cursor.sequence > after.sequence) {
            let size = chunk.payload.byte_len();
            if !chunks.is_empty() && bytes + size > max_bytes {
                break;
            }
            chunks.push((**chunk).clone());
            bytes += size;
            if bytes >= max_bytes {
                break;
            }
        }
    } else {
        for chunk in records.rev() {
            let size = chunk.payload.byte_len();
            if !chunks.is_empty() && bytes + size > max_bytes {
                break;
            }
            chunks.push((**chunk).clone());
            bytes += size;
            if bytes >= max_bytes {
                break;
            }
        }
        chunks.reverse();
    }
    let next_cursor = chunks
        .last()
        .map(|chunk| chunk.cursor)
        .unwrap_or(resource.cursor);
    let gap = after.is_some_and(|after| {
        let mut expected = after.sequence;
        for chunk in &chunks {
            if expected.checked_add(1) != Some(chunk.cursor.sequence) {
                return true;
            }
            expected = chunk.cursor.sequence;
        }
        chunks.is_empty() && after.sequence < resource.cursor.sequence
    });
    ContentPage {
        resource: resource.clone(),
        chunks,
        next_cursor,
        has_more: next_cursor.sequence < resource.cursor.sequence,
        gap,
    }
}

/// Test/small-deployment backend with an explicit retained-byte ceiling.
pub struct MemoryContentBackend {
    sources: Mutex<HashMap<ContentId, (ContentResource, VecDeque<Arc<ContentChunk>>)>>,
    retained_bytes: usize,
}

impl Default for MemoryContentBackend {
    fn default() -> Self {
        Self {
            sources: Mutex::new(HashMap::new()),
            retained_bytes: 16 * 1024 * 1024,
        }
    }
}

#[async_trait]
impl ContentBackend for MemoryContentBackend {
    async fn restore(&self, archive: &ContentArchive) -> Result<(), StoreError> {
        archive.validate()?;
        let budget = if archive.resource.kind == ContentKind::Terminal {
            self.retained_bytes
                .max(agena_domain::MAX_TERMINAL_FRAME_BYTES + RECORD_OVERHEAD)
        } else {
            self.retained_bytes
        };
        if archive
            .chunks
            .iter()
            .map(|chunk| chunk.payload.byte_len() + RECORD_OVERHEAD)
            .sum::<usize>()
            > budget
        {
            return Err(StoreError::Constraint(
                "content archive exceeds the retained-byte budget".into(),
            ));
        }
        let mut sources = self.sources.lock().expect("memory content lock");
        if sources.contains_key(&archive.resource.resource_id) {
            return Err(StoreError::Conflict(
                "content resource already exists".into(),
            ));
        }
        sources.insert(
            archive.resource.resource_id,
            (
                archive.resource.clone(),
                archive.chunks.iter().cloned().map(Arc::new).collect(),
            ),
        );
        Ok(())
    }

    async fn prune(
        &self,
        protected: &std::collections::HashSet<ContentId>,
    ) -> Result<usize, StoreError> {
        let mut sources = self.sources.lock().expect("memory content lock");
        let before = sources.len();
        sources.retain(|id, _| protected.contains(id));
        Ok(before - sources.len())
    }

    async fn delete(&self, id: ContentId) -> Result<(), StoreError> {
        self.sources
            .lock()
            .expect("memory content lock")
            .remove(&id);
        Ok(())
    }

    async fn commit(
        &self,
        mut resource: ContentResource,
        chunks: &[Arc<ContentChunk>],
    ) -> Result<ContentResource, StoreError> {
        let mut sources = self.sources.lock().expect("memory content lock");
        let entry = sources.get(&resource.resource_id);
        if let Some((previous, _)) = entry {
            if previous.cursor.epoch != resource.cursor.epoch
                || previous.part_id != resource.part_id
                || previous.owner_session_id != resource.owner_session_id
                || previous.kind != resource.kind
            {
                return Err(StoreError::Constraint(
                    "content ownership or generation changed".into(),
                ));
            }
            if previous.state != ContentState::Active {
                // Retention belongs to the backend. A retry after publication
                // still carries the source's preceding retention snapshot.
                resource.retained_ranges = previous.retained_ranges.clone();
                if previous == &resource {
                    return Ok(resource);
                }
                return Err(StoreError::InvalidState(
                    "committed content is sealed".into(),
                ));
            }
        }
        let previous = entry.map_or(0, |(resource, _)| resource.committed_cursor.sequence);
        let mut records = entry.map_or_else(VecDeque::new, |(_, records)| records.clone());
        let mut expected = previous;
        let mut document_cursor = records
            .iter()
            .rev()
            .find(|chunk| {
                matches!(
                    chunk.payload,
                    ContentPayload::Structured { .. } | ContentPayload::StructuredSnapshot { .. }
                )
            })
            .map(|chunk| chunk.cursor);
        let mut terminal_cursor = records
            .iter()
            .rev()
            .find(|chunk| {
                matches!(
                    chunk.payload,
                    ContentPayload::Terminal { .. } | ContentPayload::TerminalPatch { .. }
                )
            })
            .map(|chunk| chunk.cursor);
        for chunk in chunks
            .iter()
            .filter(|chunk| chunk.cursor.sequence > previous)
        {
            chunk
                .payload
                .validate()
                .map_err(|error| StoreError::Constraint(error.into()))?;
            if chunk.cursor.epoch != resource.cursor.epoch
                || expected.checked_add(1) != Some(chunk.cursor.sequence)
                || !resource.kind.accepts(chunk.payload.kind())
            {
                return Err(StoreError::Constraint(
                    "content batch is not a contiguous resource stream".into(),
                ));
            }
            match &chunk.payload {
                ContentPayload::Structured { base_cursor, .. } => {
                    if document_cursor != Some(*base_cursor) {
                        return Err(StoreError::Constraint(
                            "document mutation is missing its base".into(),
                        ));
                    }
                    document_cursor = Some(chunk.cursor);
                }
                ContentPayload::StructuredSnapshot { .. } => document_cursor = Some(chunk.cursor),
                ContentPayload::TerminalPatch { base_cursor, .. } => {
                    if terminal_cursor != Some(*base_cursor) {
                        return Err(StoreError::Constraint(
                            "terminal patch is missing its base".into(),
                        ));
                    }
                    terminal_cursor = Some(chunk.cursor);
                }
                ContentPayload::Terminal { .. } => terminal_cursor = Some(chunk.cursor),
                _ => {}
            }
            expected = chunk.cursor.sequence;
            records.push_back(chunk.clone());
        }
        if (resource.cursor.sequence != expected && !accounts_for_lost_tail(&resource, expected))
            || resource.committed_cursor != resource.cursor
        {
            return Err(StoreError::Constraint(
                "content manifest cursor does not cover the batch".into(),
            ));
        }
        let document_snapshot = records
            .iter()
            .rev()
            .find(|chunk| matches!(chunk.payload, ContentPayload::StructuredSnapshot { .. }))
            .map(|chunk| chunk.cursor.sequence);
        let terminal_snapshot = records
            .iter()
            .rev()
            .find(|chunk| matches!(chunk.payload, ContentPayload::Terminal { .. }))
            .map(|chunk| chunk.cursor.sequence);
        let protected = |chunk: &ContentChunk| match chunk.payload {
            ContentPayload::Structured { .. } | ContentPayload::StructuredSnapshot { .. } => {
                document_snapshot.is_some_and(|snapshot| chunk.cursor.sequence >= snapshot)
            }
            ContentPayload::Terminal { .. } | ContentPayload::TerminalPatch { .. } => {
                terminal_snapshot.is_some_and(|snapshot| chunk.cursor.sequence >= snapshot)
            }
            _ => false,
        };
        let budget = if resource.kind == ContentKind::Terminal {
            self.retained_bytes
                .max(agena_domain::MAX_TERMINAL_FRAME_BYTES + RECORD_OVERHEAD)
        } else {
            self.retained_bytes
        };
        let mut bytes = records
            .iter()
            .map(|chunk| chunk.payload.byte_len() + RECORD_OVERHEAD)
            .sum::<usize>();
        while bytes > budget && records.len() > 1 {
            let index = (1..records.len() - 1)
                .find(|index| !protected(&records[*index]))
                .or_else(|| (!protected(&records[0])).then_some(0));
            let Some(index) = index else { break };
            let removed = records.remove(index).expect("memory record exists");
            bytes -= removed.payload.byte_len() + RECORD_OVERHEAD;
        }
        if bytes > budget {
            return Err(StoreError::Constraint(
                "content checkpoint and recent output exceed the retained-byte budget".into(),
            ));
        }
        resource.retained_ranges.clear();
        for chunk in &records {
            extend_ranges(&mut resource.retained_ranges, chunk.cursor.sequence);
        }
        sources.insert(resource.resource_id, (resource.clone(), records));
        Ok(resource)
    }

    async fn read(
        &self,
        id: ContentId,
        after: Option<ContentCursor>,
        max_bytes: usize,
    ) -> Result<ContentPage, StoreError> {
        let sources = self.sources.lock().expect("memory content lock");
        let (resource, records) = sources
            .get(&id)
            .ok_or_else(|| StoreError::not_found(format!("content {id}")))?;
        validate_cursor(resource, after)?;
        let after = after.or_else(|| {
            if !matches!(
                resource.kind,
                ContentKind::Terminal | ContentKind::Structured | ContentKind::Document
            ) {
                return None;
            }
            records
                .iter()
                .rev()
                .find(|chunk| {
                    matches!(
                        chunk.payload,
                        ContentPayload::Terminal { .. } | ContentPayload::StructuredSnapshot { .. }
                    )
                })
                .map(|chunk| ContentCursor {
                    sequence: chunk.cursor.sequence - 1,
                    ..chunk.cursor
                })
        });
        Ok(page_from_records(
            resource,
            records.iter(),
            after,
            max_bytes,
        ))
    }
}

#[cfg(test)]
mod tests;
