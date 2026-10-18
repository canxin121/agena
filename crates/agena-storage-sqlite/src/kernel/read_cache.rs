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

#[cfg(test)]
mod tests {
    use super::*;
    use agena_storage::backend::content_events::{
        EncodedEvent, EventBlockAssembler, ImmutableEventBlock,
    };

    fn install_block(
        cache: &ReadCache,
        budget: &ReadBudget,
        session: SessionId,
        content: ContentId,
        epoch: ContentEpoch,
        start: u64,
    ) -> ReadView<ImmutableEventBlock> {
        let mut assembler = EventBlockAssembler::new(start, 32 * 1024).unwrap();
        for sequence in start..start + 64 {
            let pushed = assembler
                .push(
                    EncodedEvent::new(
                        sequence as i64,
                        &EventPayload::Log {
                            channel: agena_domain::CommandOutputStream::Stdout,
                            text: format!("{sequence}:界\n"),
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
            assert!(pushed.before.is_none() && pushed.after.is_none());
        }
        let block = assembler.checkpoint().unwrap();
        let bytes = block.retained_bytes();
        let view = budget.wrap_charged(session, block, bytes).unwrap();
        let fill = cache.stamp().unwrap();
        cache.install(Key::EventBlock(session, content, epoch, start), fill, &view);
        view
    }

    #[test]
    fn capacity_eviction_preserves_authority_stamps_and_consumer_leases() {
        let cache = Arc::new(ReadCache::default());
        let budget = ReadBudget::default();
        let session = SessionId::try_from(1).unwrap();
        let content = ContentId::new();
        let epoch = ContentEpoch::new();
        let held = install_block(&cache, &budget, session, content, epoch, 0);
        drop(install_block(&cache, &budget, session, content, epoch, 64));
        let stamp = cache.stamp().unwrap();
        assert_eq!(budget.snapshot().allocations, 2);

        cache.evict();
        assert!(cache.is_current(stamp).unwrap());
        assert_eq!(cache.snapshot().entries, 0);
        assert_eq!(cache.snapshot().bytes, 0);
        assert!(cache.0.lock().unwrap().event_blocks.is_empty());
        assert!(
            cache
                .event_block_containing(session, content, epoch, 1)
                .unwrap()
                .is_none()
        );
        assert_eq!(budget.snapshot().allocations, 1);
        assert_eq!(budget.snapshot().bytes, held.modeled_bytes());

        // A fill from the same authority generation may repopulate after an
        // ordinary capacity eviction. Explicit invalidation still rejects it.
        cache.install(Key::EventBlock(session, content, epoch, 0), stamp, &held);
        assert!(
            cache
                .event_block_containing(session, content, epoch, 1)
                .unwrap()
                .unwrap()
                .ptr_eq(&held)
        );
        cache.clear();
        assert!(!cache.is_current(stamp).unwrap());
        let before_commit = cache.stamp().unwrap();
        let fence = cache.pause();
        assert!(!cache.is_current(before_commit).unwrap());
        fence.committed(Some(&[session]), &[]);
        drop(fence);
        assert!(!cache.is_current(before_commit).unwrap());
        assert_eq!(budget.snapshot().bytes, held.modeled_bytes());
        drop(held);
        assert_eq!(budget.snapshot().bytes, 0);
    }

    #[test]
    fn content_stamps_reject_cross_commit_clear_and_closed_observations() {
        let cache = Arc::new(ReadCache::default());
        let before = cache.stamp().unwrap();
        assert!(cache.is_current(before).unwrap());
        let fence = cache.pause();
        assert!(!cache.is_current(before).unwrap());
        let paused = cache.stamp().unwrap();
        assert!(!cache.is_current(paused).unwrap());
        fence.committed(Some(&[]), &[]);
        drop(fence);
        assert!(!cache.is_current(before).unwrap());
        assert!(!cache.is_current(paused).unwrap());
        let after = cache.stamp().unwrap();
        assert!(cache.is_current(after).unwrap());
        cache.clear();
        assert!(!cache.is_current(after).unwrap());
        let closing = cache.stamp().unwrap();
        cache.close();
        assert_eq!(cache.is_current(closing), Err(KernelError::Closed));
        assert!(matches!(cache.stamp(), Err(KernelError::Closed)));
    }

    #[test]
    fn containing_blocks_share_misaligned_positions_and_follow_every_invalidation() {
        let cache = Arc::new(ReadCache::default());
        let budget = ReadBudget::default();
        let session = SessionId::try_from(1).unwrap();
        let other = SessionId::try_from(2).unwrap();
        let content = ContentId::new();
        let epoch = ContentEpoch::new();
        let first = install_block(&cache, &budget, session, content, epoch, 0);
        let second = install_block(&cache, &budget, session, content, epoch, 64);
        for after in [0, 1, 7, 31, 32, 33, 63] {
            assert!(
                cache
                    .event_block_containing(session, content, epoch, after)
                    .unwrap()
                    .unwrap()
                    .ptr_eq(&first)
            );
        }
        assert!(
            cache
                .event_block_containing(session, content, epoch, 64)
                .unwrap()
                .unwrap()
                .ptr_eq(&second)
        );
        assert!(
            cache
                .event_block_containing(session, content, epoch, 128)
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .event_block_containing(other, content, epoch, 1)
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .event_block_containing(session, ContentId::new(), epoch, 1)
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .event_block_containing(session, content, ContentEpoch::new(), 1)
                .unwrap()
                .is_none()
        );

        let replacement = install_block(&cache, &budget, session, content, epoch, 0);
        assert!(
            cache
                .event_block_containing(session, content, epoch, 17)
                .unwrap()
                .unwrap()
                .ptr_eq(&replacement)
        );
        assert_eq!(cache.0.lock().unwrap().event_blocks.len(), 2);
        let fence = cache.pause();
        assert!(
            cache
                .event_block_containing(session, content, epoch, 1)
                .unwrap()
                .is_none()
        );
        fence.committed(Some(&[other]), &[]);
        drop(fence);
        assert!(
            cache
                .event_block_containing(session, content, epoch, 1)
                .unwrap()
                .is_some()
        );
        let fence = cache.pause();
        fence.committed(Some(&[session]), &[]);
        drop(fence);
        assert!(
            cache
                .event_block_containing(session, content, epoch, 1)
                .unwrap()
                .is_none()
        );
        assert!(cache.0.lock().unwrap().event_blocks.is_empty());

        drop(install_block(&cache, &budget, session, content, epoch, 0));
        cache.clear();
        assert!(cache.0.lock().unwrap().event_blocks.is_empty());
        drop(install_block(&cache, &budget, session, content, epoch, 0));
        cache.close();
        assert!(cache.0.lock().unwrap().event_blocks.is_empty());
        assert!(matches!(
            cache.event_block_containing(session, content, epoch, 1),
            Err(KernelError::Closed)
        ));
        drop((first, second, replacement));
        assert_eq!(budget.snapshot().bytes, 0);
    }

    #[test]
    fn lru_eviction_removes_containing_index_without_releasing_consumer_lease() {
        let cache = ReadCache::default();
        let budget = ReadBudget::default();
        let session = SessionId::try_from(1).unwrap();
        let content = ContentId::new();
        let epoch = ContentEpoch::new();
        let held = install_block(&cache, &budget, session, content, epoch, 0);
        for index in 1..80 {
            let view = budget.wrap(session, "z".repeat(200_000)).unwrap();
            cache.install(
                Key::History(session, None, index),
                cache.stamp().unwrap(),
                &view,
            );
        }
        assert!(cache.snapshot().evictions > 0);
        assert!(
            cache
                .event_block_containing(session, content, epoch, 1)
                .unwrap()
                .is_none()
        );
        assert!(cache.0.lock().unwrap().event_blocks.is_empty());
        cache.clear();
        assert_eq!(budget.snapshot().bytes, held.modeled_bytes());
        drop(held);
        assert_eq!(budget.snapshot().bytes, 0);
    }

    fn miss(cache: &ReadCache, key: &Key) -> Fill {
        match cache.lookup::<String>(key).unwrap() {
            Lookup::Miss(fill) => fill,
            Lookup::Hit(_) => panic!("expected cache miss"),
        }
    }

    #[test]
    fn commit_fences_old_and_midcommit_fills_without_invalidating_unaffected_sessions() {
        let cache = Arc::new(ReadCache::default());
        let budget = ReadBudget::default();
        let a = SessionId::try_from(1).unwrap();
        let b = SessionId::try_from(2).unwrap();
        let key = Key::Metadata(a);
        let other_key = Key::Metadata(b);
        let old = budget.wrap(a, "old".to_string()).unwrap();
        let other = budget.wrap(b, "other".to_string()).unwrap();
        let old_fill = miss(&cache, &key);
        cache.install(key.clone(), old_fill, &old);
        cache.install(other_key.clone(), miss(&cache, &other_key), &other);
        let fence = cache.pause();
        let during = miss(&cache, &key);
        fence.committed(Some(&[a]), &[]);
        drop(fence);
        cache.install(key.clone(), old_fill, &old);
        cache.install(key.clone(), during, &old);
        let fresh_fill = miss(&cache, &key);
        let fresh = budget.wrap(a, "new".to_string()).unwrap();
        cache.install(key.clone(), fresh_fill, &fresh);
        let Lookup::Hit(hit) = cache.lookup::<String>(&key).unwrap() else {
            panic!("warm value missing")
        };
        assert!(hit.ptr_eq(&fresh));
        let Lookup::Hit(hit) = cache.lookup::<String>(&other_key).unwrap() else {
            panic!("unaffected value evicted")
        };
        assert!(hit.ptr_eq(&other));
        assert_eq!(cache.snapshot().rejected_fills, 2);
        // A rejected COMMIT preserves the previously authoritative cache.
        drop(cache.pause());
        assert!(matches!(
            cache.lookup::<String>(&key).unwrap(),
            Lookup::Hit(_)
        ));
    }

    #[test]
    fn eviction_releases_cache_ownership_but_cannot_unaccount_a_returned_view() {
        let cache = Arc::new(ReadCache::default());
        let budget = ReadBudget::default();
        let session = SessionId::try_from(1).unwrap();
        let first = budget.wrap(session, "x".repeat(200_000)).unwrap();
        let key = Key::History(session, None, 1);
        let fill = match cache.lookup::<String>(&key).unwrap() {
            Lookup::Miss(fill) => fill,
            _ => unreachable!(),
        };
        cache.install(key, fill, &first);
        for index in 2..80 {
            let key = Key::History(session, None, index);
            let fill = match cache.lookup::<String>(&key).unwrap() {
                Lookup::Miss(fill) => fill,
                _ => unreachable!(),
            };
            let view = budget.wrap(session, "z".repeat(200_000)).unwrap();
            cache.install(key, fill, &view);
        }
        let snapshot = cache.snapshot();
        assert!(snapshot.bytes <= CACHE_BYTES);
        assert!(snapshot.evictions > 0);
        cache.clear();
        assert_eq!(budget.snapshot().allocations, 1);
        assert_eq!(budget.snapshot().bytes, first.modeled_bytes());
        drop(first);
        assert_eq!(budget.snapshot().bytes, 0);
    }
}
