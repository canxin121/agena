use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::model::Session;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SessionCachePolicy {
    pub(crate) max_sessions: usize,
    pub(crate) ttl: Duration,
    pub(crate) max_bytes: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
    pub entry_count: usize,
    pub total_bytes: usize,
}

#[derive(Debug, Clone)]
struct CachedSessionEntry {
    session: Session,
    last_accessed: Instant,
}

#[derive(Debug, Default)]
pub(crate) struct SessionCache {
    entries: HashMap<i64, CachedSessionEntry>,
    access_order: VecDeque<i64>,
    total_bytes: usize,
    stats: SessionCacheStats,
}

impl SessionCache {
    pub(crate) fn get(
        &mut self,
        session_id: i64,
        cache_policy: SessionCachePolicy,
    ) -> Option<Session> {
        self.prune(cache_policy);
        let session = match self.entries.get_mut(&session_id) {
            Some(entry) => {
                self.stats.hits = self.stats.hits.saturating_add(1);
                entry.last_accessed = Instant::now();
                entry.session.clone()
            }
            None => {
                self.stats.misses = self.stats.misses.saturating_add(1);
                return None;
            }
        };
        self.bump(session_id);
        Some(session)
    }

    pub(crate) fn insert(&mut self, mut session: Session, cache_policy: SessionCachePolicy) {
        self.prune(cache_policy);
        session.refresh_derived();
        let session_id = session.id;
        self.remove(session_id, false);
        let approx_bytes = session.approx_bytes();
        if approx_bytes > cache_policy.max_bytes.max(1) {
            return;
        }

        self.entries.insert(
            session_id,
            CachedSessionEntry {
                session,
                last_accessed: Instant::now(),
            },
        );
        self.total_bytes = self.total_bytes.saturating_add(approx_bytes);
        self.stats.inserts = self.stats.inserts.saturating_add(1);
        self.bump(session_id);
        self.enforce_limit(cache_policy);
    }

    pub(crate) fn prune(&mut self, cache_policy: SessionCachePolicy) {
        let now = Instant::now();
        let expired = self
            .entries
            .iter()
            .filter_map(|(session_id, entry)| {
                (now.saturating_duration_since(entry.last_accessed) > cache_policy.ttl)
                    .then_some(*session_id)
            })
            .collect::<Vec<_>>();
        for session_id in expired {
            self.remove(session_id, true);
        }
    }

    pub(crate) fn stats(&self) -> SessionCacheStats {
        SessionCacheStats {
            entry_count: self.entries.len(),
            total_bytes: self.total_bytes,
            ..self.stats
        }
    }

    #[cfg(test)]
    pub(crate) fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    fn enforce_limit(&mut self, cache_policy: SessionCachePolicy) {
        while self.entries.len() > cache_policy.max_sessions.max(1)
            || self.total_bytes > cache_policy.max_bytes.max(1)
        {
            let Some(session_id) = self.access_order.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&session_id) {
                self.total_bytes = self
                    .total_bytes
                    .saturating_sub(entry.session.approx_bytes());
                self.stats.evictions = self.stats.evictions.saturating_add(1);
            }
        }
    }

    fn bump(&mut self, session_id: i64) {
        self.access_order.retain(|item| *item != session_id);
        self.access_order.push_back(session_id);
    }

    fn remove(&mut self, session_id: i64, count_eviction: bool) {
        if let Some(entry) = self.entries.remove(&session_id) {
            self.total_bytes = self
                .total_bytes
                .saturating_sub(entry.session.approx_bytes());
            if count_eviction {
                self.stats.evictions = self.stats.evictions.saturating_add(1);
            }
        }
        self.access_order.retain(|item| *item != session_id);
    }
}
