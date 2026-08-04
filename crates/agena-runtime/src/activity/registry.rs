//! Unified background-activity registry.
//!
//! Every long-running piece of work Agena tools create (background shell
//! processes, delegated subagent tasks, runtime maintenance tasks) is
//! projected into one bounded in-memory store. Sources push [`BackgroundActivity`]
//! records in; every mutation publishes a [`BackgroundActivityChangedEvent`] on
//! the runtime event bus so TUI/Web can react live.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use agena_domain::{
    BackgroundActivity, BackgroundActivityChangedEvent, BackgroundActivityEventReason,
    BackgroundActivityFilter,
};
use chrono::Utc;
use parking_lot::Mutex;
use tokio::sync::mpsc;

const DEFAULT_ACTIVITY_HISTORY_LIMIT: usize = 256;

/// Bounded ordered store for activity records. Newest first.
#[derive(Debug)]
struct ActivityStore {
    order: VecDeque<String>,
    activities: BTreeMap<String, BackgroundActivity>,
    history_limit: usize,
}

impl ActivityStore {
    fn new(history_limit: usize) -> Self {
        Self {
            order: VecDeque::new(),
            activities: BTreeMap::new(),
            history_limit,
        }
    }

    fn upsert(&mut self, activity: BackgroundActivity) -> Option<BackgroundActivity> {
        let previous = self.activities.insert(activity.id.clone(), activity.clone());
        if previous.is_none() {
            self.order.push_front(activity.id.clone());
        }
        self.trim_history();
        previous
    }

    fn list(&self, filter: &BackgroundActivityFilter) -> Vec<BackgroundActivity> {
        self.order
            .iter()
            .filter_map(|id| self.activities.get(id))
            .filter(|activity| filter.matches(activity))
            .cloned()
            .collect()
    }

    fn get(&self, id: &str) -> Option<BackgroundActivity> {
        self.activities.get(id).cloned()
    }

    fn remove(&mut self, id: &str) -> Option<BackgroundActivity> {
        let removed = self.activities.remove(id);
        if removed.is_some() {
            self.order.retain(|candidate| candidate != id);
        }
        removed
    }

    fn clear_finished(&mut self) -> Vec<String> {
        let active = self
            .order
            .iter()
            .filter_map(|id| self.activities.get(id))
            .filter(|activity| activity.is_active())
            .map(|activity| activity.id.clone())
            .collect::<BTreeSet<_>>();
        let finished = self
            .order
            .iter()
            .filter(|id| !active.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        for id in &finished {
            self.activities.remove(id);
        }
        self.order.retain(|id| !finished.contains(id));
        finished
    }

    fn trim_history(&mut self) {
        if self.order.len() <= self.history_limit {
            return;
        }
        // Keep the newest `history_limit` entries; drop only terminal ones so
        // running work is never silently evicted from the UI.
        let mut index = self.order.len();
        while self.order.len() > self.history_limit && index > 0 {
            index -= 1;
                        let Some(id) = self.order.get(index).cloned() else {
                break;
            };
            let active = self
                .activities
                .get(id.as_str())
                .map(|activity| activity.is_active())
                .unwrap_or(false);
            // Never evict running work from the UI; only trim terminal records.
            if active {
                continue;
            }
            let _ = self.order.remove(index);
            self.activities.remove(id.as_str());
        }
    }
}

/// Shared handle over the activity store plus the publish channel.
#[derive(Debug, Clone)]
pub(crate) struct ActivityRegistry {
    store: Arc<Mutex<ActivityStore>>,
    tx: mpsc::UnboundedSender<BackgroundActivityChangedEvent>,
}

impl ActivityRegistry {
    pub(crate) fn new(tx: mpsc::UnboundedSender<BackgroundActivityChangedEvent>) -> Self {
        Self {
            store: Arc::new(Mutex::new(ActivityStore::new(DEFAULT_ACTIVITY_HISTORY_LIMIT))),
            tx,
        }
    }

    /// Insert or replace an activity and publish the corresponding event. The
    /// reason is derived from the transition: brand-new records are `Started`,
    /// transitions into a terminal status are `Finished`, everything else is
    /// `Updated`.
    pub(crate) fn upsert(&self, activity: BackgroundActivity) {
        let reason = {
            let mut store = self.store.lock();
            let previous = store.upsert(activity.clone());
            match previous {
                None => BackgroundActivityEventReason::Started,
                Some(previous) if previous.is_active() && !activity.is_active() => {
                    BackgroundActivityEventReason::Finished
                }
                Some(_) => BackgroundActivityEventReason::Updated,
            }
        };
        self.publish(activity, reason);
    }

    /// Dismiss a record from the store (does not stop the underlying work).
    pub(crate) fn dismiss(&self, id: &str) -> Option<BackgroundActivity> {
        let removed = self.store.lock().remove(id);
        if let Some(activity) = &removed {
            self.publish(activity.clone(), BackgroundActivityEventReason::Dismissed);
        }
        removed
    }

    /// Remove every finished record; returns the ids that were removed.
    pub(crate) fn clear_finished(&self) -> Vec<String> {
        self.store.lock().clear_finished()
    }

    pub(crate) fn list(&self, filter: &BackgroundActivityFilter) -> Vec<BackgroundActivity> {
        self.store.lock().list(filter)
    }

    pub(crate) fn get(&self, id: &str) -> Option<BackgroundActivity> {
        self.store.lock().get(id)
    }

    /// Push the record and event onto the bus-facing channel. The publisher
    /// task drains this channel and persists/broadcasts each event.
    fn publish(&self, activity: BackgroundActivity, reason: BackgroundActivityEventReason) {
        let event = BackgroundActivityChangedEvent {
            activity_id: activity.id.clone(),
            reason,
            activity,
            ts_ms: Utc::now().timestamp_millis(),
        };
        let _ = self.tx.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::{BackgroundActivityKind, BackgroundActivityStatus};

    fn activity(id: &str, status: BackgroundActivityStatus) -> BackgroundActivity {
        BackgroundActivity {
            id: id.to_string(),
            kind: BackgroundActivityKind::Shell,
            status,
            title: format!("Run process · {id}"),
            description: id.to_string(),
            command: Some(id.to_string()),
            workdir: None,
            session_id: None,
            parent_session_id: None,
            created_at_ms: 1,
            started_at_ms: 1,
            finished_at_ms: status.is_terminal().then_some(2),
            exit_code: None,
            message: None,
            failure: None,
            last_seq: 0,
            has_more: false,
            dropped_lines: 0,
            cancellable: true,
            dismissible: true,
        }
    }

        #[test]
    fn store_orders_newest_first_and_trims_terminal_oldest() {
        let mut store = ActivityStore::new(2);
        store.upsert(activity("a", BackgroundActivityStatus::Succeeded));
        store.upsert(activity("b", BackgroundActivityStatus::Running));
        store.upsert(activity("c", BackgroundActivityStatus::Running));
        assert_eq!(store.list(&BackgroundActivityFilter::default())[0].id, "c");
        // `a` is trimmed (oldest terminal); active `b` and `c` survive.
        let ids = store
            .list(&BackgroundActivityFilter::default())
            .into_iter()
            .map(|a| a.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["c", "b"]);
    }

    #[test]
    fn clear_finished_removes_only_terminal_records() {
        let mut store = ActivityStore::new(10);
        store.upsert(activity("a", BackgroundActivityStatus::Running));
        store.upsert(activity("b", BackgroundActivityStatus::Failed));
        let removed = store.clear_finished();
        assert_eq!(removed, vec!["b"]);
        assert!(store.get("a").is_some());
        assert!(store.get("b").is_none());
    }

    #[test]
    fn registry_publishes_reason_by_transition() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let registry = ActivityRegistry::new(tx);
        registry.upsert(activity("a", BackgroundActivityStatus::Running));
        assert_eq!(
            rx.try_recv().unwrap().reason,
            BackgroundActivityEventReason::Started
        );
        let mut finished = activity("a", BackgroundActivityStatus::Succeeded);
        finished.finished_at_ms = Some(3);
        registry.upsert(finished);
        assert_eq!(
            rx.try_recv().unwrap().reason,
            BackgroundActivityEventReason::Finished
        );
        let mut updated = activity("a", BackgroundActivityStatus::Succeeded);
        updated.message = Some("still succeeded".into());
        registry.upsert(updated);
        assert_eq!(
            rx.try_recv().unwrap().reason,
            BackgroundActivityEventReason::Updated
        );
        registry.dismiss("a");
        assert_eq!(
            rx.try_recv().unwrap().reason,
            BackgroundActivityEventReason::Dismissed
        );
    }
}
