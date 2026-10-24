//! Byte-bounded shared views. The exclusive database owner makes warm hits
//! authoritative; generation fencing prevents a racing snapshot refilling old
//! data after a write. COMMIT is bracketed so no stale warm hit escapes between
//! SQLite acknowledging a commit and the writer installing invalidations.

use std::{
    any::Any,
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use serde::Serialize;

use super::*;

const CACHE_BYTES: usize = 16 * 1024 * 1024;
const CACHE_ENTRIES: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum Key {
    Prompt(SessionId, RoundId),
    Summaries(i64, Option<SessionSummaryCursor>, usize),
    Summary(SessionId),
    ControlSnapshot(SessionId, usize),
    Entry(SessionId, EntryId),
    OwnedTools(SessionId, Option<OwnedToolCursor>, usize),
    ActiveExecutions(SessionId, Option<ActiveExecutionCursor>, usize),
    PendingInteractions(SessionId, Option<PendingInteractionCursor>, usize),
    Metadata(SessionId),
    Configuration(SessionId),
    Execution(SessionId, ExecutionId),
    History(SessionId, Option<HistoryCursor>, usize),
    Tool(SessionId, ToolId),
    Interaction(SessionId, InteractionId),
    Round(SessionId, RoundId),
    ContentReadContext(SessionId, ContentId),
    EventBlock(SessionId, ContentId, ContentEpoch, u64),
}
impl Key {
    pub fn owner(&self) -> ReadOwner {
        match self {
            Self::Summaries(workspace, ..) => ReadOwner::Workspace(*workspace),
            Self::Prompt(s, _)
            | Self::Summary(s)
            | Self::ControlSnapshot(s, _)
            | Self::Entry(s, _)
            | Self::OwnedTools(s, ..)
            | Self::PendingInteractions(s, ..)
            | Self::ActiveExecutions(s, ..)
            | Self::Configuration(s)
            | Self::Metadata(s)
            | Self::Execution(s, _)
            | Self::History(s, ..)
            | Self::Tool(s, _)
            | Self::Interaction(s, _)
            | Self::Round(s, _)
            | Self::ContentReadContext(s, _)
            | Self::EventBlock(s, ..) => ReadOwner::Session(*s),
        }
    }
}

struct Cached {
    value: Box<dyn Any + Send + Sync>,
    bytes: usize,
    touched: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReadCacheSnapshot {
    pub bytes: usize,
    pub entries: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub rejected_fills: u64,
}

#[derive(Default)]
struct State {
    generation: u64,
    clock: u64,
    paused: bool,
    closed: bool,
    entries: HashMap<Key, Cached>,
    // One small index node per cached block, included in the existing 256-byte
    // entry overhead. Bodies and their leases remain owned only by entries.
    event_blocks: BTreeMap<(SessionId, ContentId, ContentEpoch, u64), ()>,
    stats: ReadCacheSnapshot,
}

impl State {
    fn evict(&mut self) {
        self.entries.clear();
        self.event_blocks.clear();
        self.stats.bytes = 0;
        self.stats.entries = 0;
    }

    fn remove(&mut self, key: &Key) -> Option<Cached> {
        if let Key::EventBlock(session, content, epoch, start) = key {
            self.event_blocks
                .remove(&(*session, *content, *epoch, *start));
        }
        self.entries.remove(key)
    }

    fn rebuild_event_index(&mut self) {
        self.event_blocks.clear();
        for key in self.entries.keys() {
            if let Key::EventBlock(session, content, epoch, start) = key {
                self.event_blocks
                    .insert((*session, *content, *epoch, *start), ());
            }
        }
    }
}

#[derive(Default)]
pub(super) struct ReadCache(Mutex<State>);

pub(super) enum Lookup<T> {
    Hit(ReadView<T>),
    Miss(Fill),
}
#[derive(Clone, Copy)]
pub(super) struct Fill(Option<u64>);

impl ReadCache {
    /// Short generation stamps never hold the cache mutex while accessing live
    /// content. Live admission already uses the opposite (live -> cache) order.
    pub fn stamp(&self) -> Result<Fill> {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(KernelError::Closed);
        }
        Ok(Fill((!state.paused).then_some(state.generation)))
    }

    pub fn is_current(&self, stamp: Fill) -> Result<bool> {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(KernelError::Closed);
        }
        Ok(!state.paused && stamp.0 == Some(state.generation))
    }

    pub fn event_block_containing(
        &self,
        session: SessionId,
        content: ContentId,
        epoch: ContentEpoch,
        after: u64,
    ) -> Result<Option<ReadView<agena_storage::backend::content_events::ImmutableEventBlock>>> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(KernelError::Closed);
        }
        if state.paused {
            state.stats.misses += 1;
            return Ok(None);
        }
        let candidate = state
            .event_blocks
            .range(..=(session, content, epoch, after))
            .next_back()
            .map(|(key, ())| *key);
        let Some((owner, source, incarnation, start)) = candidate else {
            state.stats.misses += 1;
            return Ok(None);
        };
        if (owner, source, incarnation) != (session, content, epoch) {
            state.stats.misses += 1;
            return Ok(None);
        }
        state.clock = state.clock.wrapping_add(1);
        let touched = state.clock;
        let cached = state
            .entries
            .get_mut(&Key::EventBlock(session, content, epoch, start))
            .expect("event block index tracks cache ownership");
        let view = cached
            .value
            .downcast_ref::<ReadView<agena_storage::backend::content_events::ImmutableEventBlock>>()
            .expect("event block cache type");
        if after >= view.end() {
            state.stats.misses += 1;
            return Ok(None);
        }
        let view = view.clone();
        cached.touched = touched;
        state.stats.hits += 1;
        Ok(Some(view))
    }

    pub fn ensure_open(&self) -> Result<()> {
        if self.0.lock().unwrap_or_else(|e| e.into_inner()).closed {
            Err(KernelError::Closed)
        } else {
            Ok(())
        }
    }
    pub fn snapshot(&self) -> ReadCacheSnapshot {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stats
            .clone()
    }
    pub fn lookup<T: Send + Sync + 'static>(&self, key: &Key) -> Result<Lookup<T>> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(KernelError::Closed);
        }
        state.clock = state.clock.wrapping_add(1);
        let clock = state.clock;
        if !state.paused
            && let Some(cached) = state.entries.get_mut(key)
        {
            let view = cached
                .value
                .downcast_ref::<ReadView<T>>()
                .expect("typed read cache key")
                .clone();
            cached.touched = clock;
            state.stats.hits += 1;
            return Ok(Lookup::Hit(view));
        }
        state.stats.misses += 1;
        Ok(Lookup::Miss(Fill(
            (!state.paused).then_some(state.generation),
        )))
    }
    pub fn install<T: Send + Sync + 'static>(&self, key: Key, fill: Fill, view: &ReadView<T>) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || state.paused || fill.0 != Some(state.generation) {
            state.stats.rejected_fills += 1;
            return;
        }
        let bytes = view.modeled_bytes().saturating_add(256);
        if bytes > CACHE_BYTES {
            return;
        }
        if let Some(old) = state.remove(&key) {
            state.stats.bytes -= old.bytes;
        }
        while state.stats.bytes + bytes > CACHE_BYTES || state.entries.len() >= CACHE_ENTRIES {
            let oldest = state
                .entries
                .iter()
                .min_by_key(|(_, v)| v.touched)
                .map(|(k, _)| k.clone())
                .expect("nonempty cache at capacity");
            let old = state.remove(&oldest).expect("oldest cache entry");
            state.stats.bytes -= old.bytes;
            state.stats.evictions += 1;
        }
        state.clock = state.clock.wrapping_add(1);
        let touched = state.clock;
        if let Key::EventBlock(session, content, epoch, start) = &key {
            state
                .event_blocks
                .insert((*session, *content, *epoch, *start), ());
        }
        state.entries.insert(
            key,
            Cached {
                value: Box::new(view.clone()),
                bytes,
                touched,
            },
        );
        state.stats.bytes += bytes;
        state.stats.entries = state.entries.len();
    }
    /// Capacity reclamation changes ownership, not authorized source facts.
    /// Existing stamps and in-flight fills remain valid until a real fence.
    pub fn evict(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).evict();
    }
    pub fn clear(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.generation = state.generation.wrapping_add(1);
        state.evict();
    }
    pub fn close(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        state.generation = state.generation.wrapping_add(1);
        state.evict();
    }
    pub fn pause(self: &Arc<Self>) -> CommitFence {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!state.paused, "single kernel writer");
        state.generation = state.generation.wrapping_add(1);
        state.paused = true;
        CommitFence(self.clone())
    }
}

pub(super) struct CommitFence(Arc<ReadCache>);
impl CommitFence {
    /// None invalidates all cached sessions (e.g. abandoned capture revision).
    pub fn committed(&self, sessions: Option<&[SessionId]>, workspaces: &[i64]) {
        let mut state = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.retain(|key, _| {
            sessions.is_some_and(|sessions| match key.owner() {
                ReadOwner::Session(session) => !sessions.contains(&session),
                ReadOwner::Workspace(workspace) => !workspaces.contains(&workspace),
            })
        });
        state.rebuild_event_index();
        state.stats.bytes = state.entries.values().map(|v| v.bytes).sum();
        state.stats.entries = state.entries.len();
    }
}
impl Drop for CommitFence {
    fn drop(&mut self) {
        self.0.0.lock().unwrap_or_else(|e| e.into_inner()).paused = false;
    }
}
