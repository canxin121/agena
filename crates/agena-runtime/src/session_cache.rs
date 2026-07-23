use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use agena_domain::SessionCacheStats;

use crate::SessionCachePolicy;

#[derive(Debug, Clone)]
struct CachedSessionRecord<T> {
    session: T,
    last_accessed: Instant,
}

#[derive(Debug)]
pub struct SessionCache<T: CacheEntry> {
    sessions: HashMap<i64, CachedSessionRecord<T>>,
    access_order: VecDeque<i64>,
    total_bytes: usize,
    stats: agena_domain::SessionCacheStats,
}

pub trait CacheEntry: Clone {
    fn cache_key(&self) -> i64;
    fn approx_cache_bytes(&self) -> usize;
}

impl<T: CacheEntry> Default for SessionCache<T> {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            access_order: VecDeque::new(),
            total_bytes: 0,
            stats: SessionCacheStats::default(),
        }
    }
}

impl<T: CacheEntry> SessionCache<T> {
    pub fn discard(&mut self, session_id: i64) {
        self.remove(session_id, false);
    }

    pub fn update(&mut self, session_id: i64, update: impl FnOnce(&mut T)) -> Option<T> {
        let cached = self.sessions.get_mut(&session_id)?;
        let previous_bytes = cached.session.approx_cache_bytes();
        update(&mut cached.session);
        let current_bytes = cached.session.approx_cache_bytes();
        self.total_bytes = self
            .total_bytes
            .saturating_sub(previous_bytes)
            .saturating_add(current_bytes);
        Some(cached.session.clone())
    }

    pub fn get(&mut self, session_id: i64, cache_policy: SessionCachePolicy) -> Option<T> {
        self.prune(cache_policy);
        let session = match self.sessions.get_mut(&session_id) {
            Some(cached) => {
                self.stats.hits = self.stats.hits.saturating_add(1);
                cached.last_accessed = Instant::now();
                cached.session.clone()
            }
            None => {
                self.stats.misses = self.stats.misses.saturating_add(1);
                return None;
            }
        };
        self.bump(session_id);
        Some(session)
    }

    pub fn insert(&mut self, session: T, cache_policy: SessionCachePolicy) {
        self.prune(cache_policy);
        let session_id = session.cache_key();
        self.remove(session_id, false);
        let approx_bytes = session.approx_cache_bytes();
        if approx_bytes > cache_policy.max_bytes.max(1) {
            return;
        }

        self.sessions.insert(
            session_id,
            CachedSessionRecord {
                session,
                last_accessed: Instant::now(),
            },
        );
        self.total_bytes = self.total_bytes.saturating_add(approx_bytes);
        self.stats.inserts = self.stats.inserts.saturating_add(1);
        self.bump(session_id);
        self.enforce_limit(cache_policy);
    }

    pub fn prune(&mut self, cache_policy: SessionCachePolicy) {
        let now = Instant::now();
        let expired = self
            .sessions
            .iter()
            .filter_map(|(session_id, cached)| {
                (now.saturating_duration_since(cached.last_accessed) > cache_policy.ttl)
                    .then_some(*session_id)
            })
            .collect::<Vec<_>>();
        for session_id in expired {
            self.remove(session_id, true);
        }
    }

    pub fn stats(&self) -> SessionCacheStats {
        SessionCacheStats {
            session_count: self.sessions.len(),
            total_bytes: self.total_bytes,
            ..self.stats
        }
    }

    fn enforce_limit(&mut self, cache_policy: SessionCachePolicy) {
        while self.sessions.len() > cache_policy.max_sessions.max(1)
            || self.total_bytes > cache_policy.max_bytes.max(1)
        {
            let Some(session_id) = self.access_order.pop_front() else {
                break;
            };
            if let Some(cached) = self.sessions.remove(&session_id) {
                self.total_bytes = self
                    .total_bytes
                    .saturating_sub(cached.session.approx_cache_bytes());
                self.stats.evictions = self.stats.evictions.saturating_add(1);
            }
        }
    }

    fn bump(&mut self, session_id: i64) {
        self.access_order.retain(|item| *item != session_id);
        self.access_order.push_back(session_id);
    }

    fn remove(&mut self, session_id: i64, count_eviction: bool) {
        if let Some(cached) = self.sessions.remove(&session_id) {
            self.total_bytes = self
                .total_bytes
                .saturating_sub(cached.session.approx_cache_bytes());
            if count_eviction {
                self.stats.evictions = self.stats.evictions.saturating_add(1);
            }
        }
        self.access_order.retain(|item| *item != session_id);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{CacheEntry, SessionCache};
    use crate::SessionCachePolicy;

    #[derive(Clone)]
    struct Entry {
        id: i64,
        bytes: usize,
    }

    impl CacheEntry for Entry {
        fn cache_key(&self) -> i64 {
            self.id
        }

        fn approx_cache_bytes(&self) -> usize {
            self.bytes
        }
    }

    #[test]
    fn evicts_least_recent_entry_when_the_byte_limit_is_exceeded() {
        let policy = SessionCachePolicy {
            max_sessions: 3,
            max_bytes: 10,
            ttl: Duration::from_secs(60),
        };
        let mut cache = SessionCache::default();
        cache.insert(Entry { id: 1, bytes: 6 }, policy);
        cache.insert(Entry { id: 2, bytes: 6 }, policy);

        assert!(cache.get(1, policy).is_none());
        assert_eq!(cache.get(2, policy).map(|entry| entry.id), Some(2));
        let stats = cache.stats();
        assert_eq!(stats.session_count, 1);
        assert_eq!(stats.total_bytes, 6);
        assert_eq!(stats.evictions, 1);
    }
}
