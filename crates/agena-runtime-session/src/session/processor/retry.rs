//! A retry belongs to a single provider stream and is cleared on progress,
//! cancellation, failure, or a dropped future. Old streams cannot clear new ones.
use agena_domain::ProviderRetryStatus;
use std::{
    cell::Cell,
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Default)]
pub(crate) struct RetryRegistry(Arc<Mutex<HashMap<i64, (uuid::Uuid, ProviderRetryStatus)>>>);

impl RetryRegistry {
    pub(crate) fn snapshot(&self, session_id: i64) -> Option<ProviderRetryStatus> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&session_id)
            .map(|(_, state)| state.clone())
    }

    pub(super) fn track(&self, session_id: i64) -> RetryGuard {
        RetryGuard {
            registry: self.clone(),
            session_id,
            owner: uuid::Uuid::new_v4(),
            active: Cell::new(false),
        }
    }
}

pub(super) struct RetryGuard {
    registry: RetryRegistry,
    session_id: i64,
    owner: uuid::Uuid,
    active: Cell<bool>,
}
impl RetryGuard {
    pub(super) fn update(&self, attempt: u32, max_retries: u32, delay_ms: u64, reason: &str) {
        // Diagnostics remain in provider logs. Only reviewed reasons cross the
        // user-facing API boundary; error chains can contain credentials/URLs.
        let reason = reason.to_ascii_lowercase();
        let message = if reason.contains("429") || reason.contains("rate limit") {
            "The provider is rate limiting requests. Retrying automatically."
        } else if reason.contains("timeout") || reason.contains("timed out") {
            "The provider request timed out. Retrying automatically."
        } else {
            "The provider request failed temporarily. Retrying automatically."
        };
        let state = ProviderRetryStatus {
            attempt,
            max_retries,
            next_at_ms: chrono::Utc::now()
                .timestamp_millis()
                .saturating_add(delay_ms.min(i64::MAX as u64) as i64),
            message: message.to_owned(),
        };
        self.registry
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(self.session_id, (self.owner, state));
        self.active.set(true);
    }

    pub(super) fn clear(&self) {
        // The normal token stream never takes the shared retry registry lock.
        if !self.active.replace(false) {
            return;
        }
        let mut states = self.registry.0.lock().unwrap_or_else(|p| p.into_inner());
        if states
            .get(&self.session_id)
            .is_some_and(|(owner, _)| *owner == self.owner)
        {
            states.remove(&self.session_id);
        }
    }
}
impl Drop for RetryGuard {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retries_are_isolated_and_cleared_on_progress_and_drop() {
        let registry = RetryRegistry::default();
        let first = registry.track(7);
        first.update(1, 5, 2000, "429 secret diagnostic");
        let snapshot = registry.snapshot(7).unwrap();
        assert!(snapshot.next_at_ms > chrono::Utc::now().timestamp_millis());
        assert!(!snapshot.message.contains("secret"));
        assert!(registry.snapshot(8).is_none());
        let second = registry.track(7);
        second.update(2, 5, 1000, "timeout");
        drop(first);
        assert_eq!(registry.snapshot(7).unwrap().attempt, 2);
        second.clear();
        assert!(registry.snapshot(7).is_none());
        second.update(3, 5, 0, "connection");
        drop(second);
        assert!(registry.snapshot(7).is_none());
    }
}
