use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::model::Session;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SessionCachePolicy {
    pub(crate) max_sessions: usize,
    pub(crate) ttl: Duration,
    pub(crate) max_bytes: usize,
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
}

impl SessionCache {
    pub(crate) fn get(
        &mut self,
        session_id: i64,
        cache_policy: SessionCachePolicy,
    ) -> Option<Session> {
        self.prune(cache_policy);
        let session = {
            let entry = self.entries.get_mut(&session_id)?;
            entry.last_accessed = Instant::now();
            entry.session.clone()
        };
        self.bump(session_id);
        Some(session)
    }

    pub(crate) fn insert(&mut self, mut session: Session, cache_policy: SessionCachePolicy) {
        self.prune(cache_policy);
        session.refresh_derived();
        let session_id = session.id;
        self.remove(session_id);
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
            self.remove(session_id);
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
            }
        }
    }

    fn bump(&mut self, session_id: i64) {
        self.access_order.retain(|item| *item != session_id);
        self.access_order.push_back(session_id);
    }

    fn remove(&mut self, session_id: i64) {
        if let Some(entry) = self.entries.remove(&session_id) {
            self.total_bytes = self
                .total_bytes
                .saturating_sub(entry.session.approx_bytes());
        }
        self.access_order.retain(|item| *item != session_id);
    }
}
