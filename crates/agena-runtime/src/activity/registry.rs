//! Unified background-activity registry.
//!
//! Every long-running piece of work Agena tools create (background shell
//! processes, agents, runtime maintenance tasks) is
//! projected into one bounded in-memory store. Sources push [`BackgroundActivity`]
//! records in; every mutation publishes a [`BackgroundActivityChangedEvent`] on
//! the runtime event bus so TUI/Web can react live. Log-only presentation
//! signals are coalesced; authoritative snapshots and resource clocks still
//! change synchronously, and lifecycle transitions are delivered immediately.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};
use std::time::Duration;

use agena_domain::{
    BackgroundActivity, BackgroundActivityChangedEvent, BackgroundActivityEventReason,
    BackgroundActivityFilter,
};
use chrono::Utc;
use parking_lot::Mutex;
use tokio::sync::mpsc;

const DEFAULT_ACTIVITY_HISTORY_LIMIT: usize = 256;
const LOG_SIGNAL_INTERVAL: Duration = Duration::from_millis(200);
/// Bounded ordered store for activity records. Newest first.
#[derive(Debug)]
struct ActivityStore {
    order: VecDeque<String>,
    activities: BTreeMap<String, BackgroundActivity>,
    history_limit: usize,
    pending_log_signals: BTreeMap<String, BackgroundActivityChangedEvent>,
    log_signal_worker_running: bool,
}

impl ActivityStore {
    fn new(history_limit: usize) -> Self {
        Self {
            order: VecDeque::new(),
            activities: BTreeMap::new(),
            history_limit,
            pending_log_signals: BTreeMap::new(),
            log_signal_worker_running: false,
        }
    }

    fn get(&self, id: &str) -> Option<BackgroundActivity> {
        self.activities.get(id).cloned()
    }

    fn upsert(
        &mut self,
        activity: BackgroundActivity,
    ) -> (Option<BackgroundActivity>, Vec<BackgroundActivity>) {
        let previous = self
            .activities
            .insert(activity.id.clone(), activity.clone());
        if previous.is_none() {
            self.order.push_front(activity.id.clone());
        }
        (previous, self.trim_history())
    }

    fn list(&self, filter: &BackgroundActivityFilter) -> Vec<BackgroundActivity> {
        self.order
            .iter()
            .filter_map(|id| self.activities.get(id))
            .filter(|activity| filter.matches(activity))
            .cloned()
            .collect()
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

    fn trim_history(&mut self) -> Vec<BackgroundActivity> {
        let mut removed = Vec::new();
        if self.order.len() <= self.history_limit {
            return removed;
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
            if let Some(activity) = self.activities.remove(id.as_str()) {
                removed.push(activity);
            }
        }
        removed
    }
}

/// Shared handle over the activity store plus the publish channel.
#[derive(Debug, Clone)]
pub(crate) struct ActivityRegistry {
    store: Arc<Mutex<ActivityStore>>,
    tx: Option<mpsc::Sender<BackgroundActivityChangedEvent>>,
    signals: Option<Arc<crate::live_signal::LiveSignalHub>>,
    published_at_ms: Arc<AtomicI64>,
}

impl ActivityRegistry {
    pub(crate) fn with_signals(signals: Arc<crate::live_signal::LiveSignalHub>) -> Self {
        Self {
            store: Arc::new(Mutex::new(ActivityStore::new(
                DEFAULT_ACTIVITY_HISTORY_LIMIT,
            ))),
            tx: None,
            signals: Some(signals),
            published_at_ms: Arc::new(AtomicI64::new(0)),
        }
    }

    /// Read one record without changing it. A source that projects a live
    /// process needs the durable provenance the operation already stored, and
    /// upsert replaces the whole record.
    pub(crate) fn get(&self, id: &str) -> Option<BackgroundActivity> {
        self.store.lock().get(id)
    }

    /// Insert or replace an activity and publish the corresponding event. The
    /// reason is derived from the transition: brand-new records are `Started`,
    /// transitions into a terminal status are `Finished`, everything else is
    /// `Updated`.
    pub(crate) fn upsert(&self, activity: BackgroundActivity) {
        let mut store = self.store.lock();
        if store.activities.get(&activity.id) == Some(&activity) {
            return;
        }
        let (previous, removed) = store.upsert(activity.clone());
        let reason = match previous.as_ref() {
            None => BackgroundActivityEventReason::Started,
            Some(previous) if previous.is_active() && !activity.is_active() => {
                BackgroundActivityEventReason::Finished
            }
            Some(_) => BackgroundActivityEventReason::Updated,
        };
        let log_only = previous.is_some_and(|mut previous| {
            previous.last_seq = activity.last_seq;
            previous.has_more = activity.has_more;
            previous.dropped_lines = activity.dropped_lines;
            previous == activity
        });
        self.publish(&mut store, activity, reason, log_only);
        for activity in removed {
            self.publish(
                &mut store,
                activity,
                BackgroundActivityEventReason::Dismissed,
                false,
            );
        }
    }

    /// Dismiss a record from the store (does not stop the underlying work).
    pub(crate) fn dismiss(&self, id: &str) -> Option<BackgroundActivity> {
        let mut store = self.store.lock();
        let removed = store.remove(id);
        if let Some(activity) = &removed {
            self.publish(
                &mut store,
                activity.clone(),
                BackgroundActivityEventReason::Dismissed,
                false,
            );
        }
        removed
    }

    /// A durable source can delete a row that was only projected by a list
    /// read (for example after restart), and never entered this live cache.
    /// Its committed snapshot still supplies the exact owner for the event.
    pub(crate) fn dismiss_projected(&self, activity: BackgroundActivity) {
        let mut store = self.store.lock();
        let activity = store.remove(&activity.id).unwrap_or(activity);
        self.publish(
            &mut store,
            activity,
            BackgroundActivityEventReason::Dismissed,
            false,
        );
    }

    /// Remove every finished record; returns the ids that were removed.
    pub(crate) fn clear_finished(&self) -> Vec<String> {
        let mut store = self.store.lock();
        let finished = store
            .activities
            .values()
            .filter(|activity| !activity.is_active())
            .cloned()
            .collect::<Vec<_>>();
        store.clear_finished();
        for activity in &finished {
            self.publish(
                &mut store,
                activity.clone(),
                BackgroundActivityEventReason::Dismissed,
                false,
            );
        }
        finished.into_iter().map(|activity| activity.id).collect()
    }

    pub(crate) fn list(&self, filter: &BackgroundActivityFilter) -> Vec<BackgroundActivity> {
        self.store.lock().list(filter)
    }

    /// Observe every mutation immediately, then limit log-only fan-out to one
    /// latest descriptor per activity per window. The store lock serializes
    /// the delayed publisher with immediate lifecycle events and dismissals.
    fn publish(
        &self,
        store: &mut ActivityStore,
        activity: BackgroundActivity,
        reason: BackgroundActivityEventReason,
        coalesce: bool,
    ) {
        let now = Utc::now().timestamp_millis();
        let previous = self
            .published_at_ms
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |previous| {
                Some(now.max(previous.saturating_add(1)))
            })
            .expect("activity clock update");
        let event = BackgroundActivityChangedEvent {
            activity_id: activity.id.clone(),
            reason,
            activity,
            ts_ms: now.max(previous.saturating_add(1)),
        };
        if let Some(signals) = &self.signals {
            let signal = crate::RuntimeLiveSignal::Activity(Box::new(event));
            signals.observe_mutation(&signal);
            if coalesce && let Ok(handle) = tokio::runtime::Handle::try_current() {
                let crate::RuntimeLiveSignal::Activity(event) = signal else {
                    unreachable!("activity publisher only creates activity signals");
                };
                store
                    .pending_log_signals
                    .insert(event.activity_id.clone(), *event);
                if !store.log_signal_worker_running {
                    store.log_signal_worker_running = true;
                    let store = Arc::downgrade(&self.store);
                    let signals = Arc::clone(signals);
                    handle.spawn(async move {
                        loop {
                            tokio::time::sleep(LOG_SIGNAL_INTERVAL).await;
                            let Some(store) = store.upgrade() else {
                                return;
                            };
                            let mut store = store.lock();
                            if store.pending_log_signals.is_empty() {
                                store.log_signal_worker_running = false;
                                return;
                            }
                            for event in
                                std::mem::take(&mut store.pending_log_signals).into_values()
                            {
                                signals
                                    .broadcast(crate::RuntimeLiveSignal::Activity(Box::new(event)));
                            }
                        }
                    });
                }
            } else {
                // A queued tail must not follow a newer state or resurrect a
                // dismissed activity. The immediate descriptor includes the
                // latest cursor, and every activity signal wakes log readers.
                store.pending_log_signals.remove(match &signal {
                    crate::RuntimeLiveSignal::Activity(event) => &event.activity_id,
                    _ => unreachable!("activity publisher only creates activity signals"),
                });
                signals.broadcast(signal);
            }
            return;
        }
        let Some(tx) = &self.tx else {
            return;
        };
        if let Err(error) = tx.try_send(event) {
            match error {
                mpsc::error::TrySendError::Full(event) => tracing::debug!(
                    activity_id = %event.activity_id,
                    reason = ?event.reason,
                    "background activity signal queue is full; the authoritative registry snapshot remains available"
                ),
                mpsc::error::TrySendError::Closed(_) => {
                    tracing::debug!("background activity signal publisher is closed")
                }
            }
        }
    }
}
