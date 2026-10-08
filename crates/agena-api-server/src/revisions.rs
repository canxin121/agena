//! Cheap invalidation tokens, maintained at mutation time rather than by
//! loading/serializing snapshots. Tokens are scoped to this server lifetime.
use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agena_api::live::{RuntimeSignalResource, SessionChangeResource};
use agena_domain::{
    BackgroundActivity, BackgroundActivityEventReason, SessionLifecycleState, SessionStateKind,
};
use agena_runtime::{RuntimeLiveSignal, RuntimeLiveSignalItem};
use agena_storage::store::PartState;
use agena_storage::store::{GlobalSubscription, SessionChange};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lru::LruCache;
use serde::Serialize;

use crate::{error::ServerError, state::AppState};

const MAX_RESOURCE_REVISIONS: usize = 8192;

/// These are comparison memos, not authoritative facts. Forget one old
/// observation at capacity; never erase all hot observations together.
struct RevisionMemo<K: Hash + Eq, V, const CAPACITY: usize>(LruCache<K, V>);

impl<K: Hash + Eq, V, const CAPACITY: usize> Default for RevisionMemo<K, V, CAPACITY> {
    fn default() -> Self {
        Self(LruCache::new(NonZeroUsize::new(CAPACITY).unwrap()))
    }
}

impl<K: Hash + Eq + Clone, V, const CAPACITY: usize> RevisionMemo<K, V, CAPACITY> {
    fn get<Q: Hash + Eq + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        self.0.peek(key)
    }
    fn get_mut<Q: Hash + Eq + ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
    {
        self.0.get_mut(key)
    }
    fn contains_key<Q: Hash + Eq + ?Sized>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.0.contains(key)
    }
    fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.0.put(key, value)
    }
    fn remove<Q: Hash + Eq + ?Sized>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
    {
        self.0.pop(key)
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.0.iter()
    }
    fn keys(&self) -> impl Iterator<Item = &K> {
        self.0.iter().map(|(key, _)| key)
    }
    fn retain(&mut self, mut keep: impl FnMut(&K, &V) -> bool) {
        let removed: Vec<_> = self
            .0
            .iter()
            .filter(|(key, value)| !keep(key, value))
            .map(|(key, _)| key.clone())
            .collect();
        for key in removed {
            self.0.pop(&key);
        }
    }
}

fn tool_section_hashes(part: &agena_storage::store::Part) -> [u64; 3] {
    fn hash<T: Hash>(value: T) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }
    [
        hash(part.content.get("input")),
        hash(part.content.get("metadata")),
        hash((
            part.state.as_str(),
            part.content.get("output"),
            part.content.get("error"),
        )),
    ]
}

struct PreparedPartChange {
    source: u64,
    fact: Option<u64>,
    sections: Option<[u64; 3]>,
}

fn observe_clock_change(clock: &Arc<Mutex<Clock>>, change: &SessionChange) {
    let prepared = if let SessionChange::PartAdded { part, .. }
    | SessionChange::PartUpdated { part, .. } = change
    {
        let id = change.session_id();
        let previous = {
            let clock = clock.lock().unwrap_or_else(|error| error.into_inner());
            if clock
                .parts
                .get(&(id, part.part_id))
                .is_some_and(|(revision, updated, _, _)| {
                    (*revision, *updated) >= (part.revision, part.updated_at_ms)
                })
            {
                return;
            }
            clock
                .file_facts
                .get(&(id, part.part_id))
                .map(|previous| (previous.source, previous.fact))
        };
        let source = if part.origin_session_id == id {
            crate::rest::recorded_file_source(part)
        } else {
            0
        };
        let fact = if source == 0 {
            None
        } else if let Some((previous_source, previous_fact)) =
            previous.filter(|(previous_source, _)| *previous_source == source)
        {
            let _ = previous_source;
            previous_fact
        } else {
            crate::rest::recorded_file_revision(part)
        };
        Some(PreparedPartChange {
            source,
            fact,
            sections: (part.kind == "tool_call" && part.visibility.visible_to_user())
                .then(|| tool_section_hashes(part)),
        })
    } else {
        None
    };
    clock
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .changed_prepared(change, prepared.as_ref());
}

#[derive(Default)]
struct Clock {
    updated_at_ms: i64,
    baseline: i64,
    resources: BTreeMap<String, i64>,
    resource_order: RevisionMemo<String, (), MAX_RESOURCE_REVISIONS>,
    resource_floor: i64,
    workspaces: RevisionMemo<i64, i64, 8192>,
    part_sessions: RevisionMemo<i64, i64, 8192>,
    sessions: RevisionMemo<i64, SessionRevision, 4096>,
    parts: RevisionMemo<(i64, i64), (i64, i64, PartState, serde_json::Value), 8192>,
    tool_sections: RevisionMemo<i64, [u64; 3], 8192>,
    file_facts: RevisionMemo<(i64, i64), FileRevision, 8192>,
    activities: RevisionMemo<String, (i64, BackgroundActivity, bool), 4096>,
    plans: RevisionMemo<i64, serde_json::Value, 4096>,
    deleted_sessions: RevisionMemo<i64, (), 4096>,
}

// Invalidation retains only navigation fields, never session configs or
// provider continuation bodies. The content version alone is not a reason
// to reload a navigation list.
struct SessionRevision {
    version: i64,
    updated_at_ms: i64,
    title: String,
    subtask_status: Option<String>,
    workspace_id: i64,
    previous_workspace_id: Option<i64>,
    parent_id: Option<i64>,
    pinned: bool,
    favorite: bool,
    lifecycle_state: SessionLifecycleState,
    state: Option<SessionStateKind>,
}

fn awaiting_interaction(state: PartState, user_input: &serde_json::Value) -> bool {
    state.is_in_flight()
        && user_input
            .get("requests")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|requests| {
                requests
                    .iter()
                    .any(|record| record.get("reply").is_none_or(serde_json::Value::is_null))
            })
}

struct FileRevision {
    stamp: (i64, i64),
    source: u64,
    fact: Option<u64>,
}

impl Clock {
    fn tick(&mut self) -> i64 {
        self.updated_at_ms = chrono::Utc::now()
            .timestamp_millis()
            .max(self.updated_at_ms.saturating_add(1));
        self.updated_at_ms
    }

    fn bump(&mut self, key: impl Into<String>) {
        let key = key.into();
        let updated_at = self.tick();
        self.remember_resource(key, updated_at);
    }

    fn remember_resource(&mut self, key: String, revision: i64) -> i64 {
        let retained = self.resources.contains_key(&key);
        if let Some((evicted, ())) = self.resource_order.0.push(key.clone(), ())
            && evicted != key
        {
            // A baseline can affect several retained list clocks. Materialize
            // those effective clocks before forgetting that baseline.
            if evicted.ends_with("-baseline") {
                let revisions: Vec<_> = self
                    .resources
                    .keys()
                    .map(|key| (key.clone(), self.revision(key)))
                    .collect();
                for (key, revision) in revisions {
                    self.resources.insert(key, revision);
                }
            }
            self.resources.remove(&evicted);
            // A forgotten key must never reuse its original token and make
            // an old HTTP representation appear current again. Retained keys
            // keep their own revision and do not share this eviction floor.
            self.resource_floor = self.tick();
        }
        let revision = if retained {
            revision
        } else {
            revision.max(self.resource_floor)
        };
        self.resources.insert(key, revision);
        revision
    }

    fn observe_resource(&mut self, key: &str) -> i64 {
        let revision = self.revision(key);
        self.remember_resource(key.to_owned(), revision)
    }

    fn reset(&mut self) {
        self.baseline = self.tick();
        self.resource_floor = self.baseline;
        self.resources.clear();
        self.resource_order = RevisionMemo::default();
    }

    fn lists_changed(&mut self, id: i64) {
        self.bump("sessions");
        if let Some(workspace) = self.workspaces.get(&id).copied() {
            self.bump(format!("workspace:{workspace}:sessions"));
            if let Some(meta) = self.sessions.get(&id) {
                let suffix = meta
                    .parent_id
                    .map_or_else(|| "roots".into(), |parent| format!("parent:{parent}"));
                self.bump(format!("workspace:{workspace}:sessions:{suffix}"));
            } else {
                self.bump(format!("workspace:{workspace}:sessions-baseline"));
            }
        } else {
            self.bump("workspace-sessions-baseline");
        }
    }

    fn parent_row_changed(&mut self, parent: i64) {
        // The parent's child count changed, so validate the list containing
        // that parent too. A child title/status change doesn't call this.
        self.lists_changed(parent);
        let previous = self
            .sessions
            .get(&parent)
            .map(|meta| (meta.pinned, meta.favorite, meta.state));
        self.buckets_changed(parent, previous);
    }

    fn bucket_changed(&mut self, id: i64, bucket: &str, count_changed: bool) {
        self.bump(format!("sessions:bucket:{bucket}"));
        if count_changed {
            self.bump(format!("sessions:bucket:{bucket}:count"));
        }
        if let Some(meta) = self.sessions.get(&id) {
            let workspace = meta.workspace_id;
            let old = meta.previous_workspace_id;
            self.bump(format!("workspace:{workspace}:sessions:bucket:{bucket}"));
            if let Some(old) = old {
                self.bump(format!("workspace:{old}:sessions:bucket:{bucket}"));
            }
        }
    }

    fn buckets_changed(
        &mut self,
        id: i64,
        previous: Option<(bool, bool, Option<SessionStateKind>)>,
    ) {
        let current = self
            .sessions
            .get(&id)
            .map(|meta| (meta.pinned, meta.favorite, meta.state));
        // State is derived from potentially several runs/interactions. Without
        // that projection, conservatively validate the three state buckets.
        for bucket in ["recent", "running", "attention"] {
            let member = |flags: (bool, bool, Option<SessionStateKind>)| {
                flags.2.is_none_or(|state| match bucket {
                    "recent" => state == SessionStateKind::Ready,
                    "running" => matches!(
                        state,
                        SessionStateKind::Running | SessionStateKind::Creating
                    ),
                    _ => state.is_attention(),
                })
            };
            if current.is_some_and(member)
                || previous.is_some_and(member)
                || (current.is_none() && previous.is_none())
            {
                let uncertain = current.is_none() && previous.is_none()
                    || current.is_some_and(|flags| flags.2.is_none())
                    || previous.is_some_and(|flags| flags.2.is_none());
                self.bucket_changed(
                    id,
                    bucket,
                    uncertain || current.is_some_and(member) != previous.is_some_and(member),
                );
            }
        }
        for (index, bucket) in ["pinned", "favorite"].into_iter().enumerate() {
            let member = |flags: (bool, bool, Option<SessionStateKind>)| {
                if index == 0 { flags.0 } else { flags.1 }
            };
            if current.is_some_and(member)
                || previous.is_some_and(member)
                || (current.is_none() && previous.is_none())
            {
                self.bucket_changed(
                    id,
                    bucket,
                    current.is_none() && previous.is_none()
                        || current.is_some_and(member) != previous.is_some_and(member),
                );
            }
        }
    }

    fn stats_changed(&mut self, id: i64) {
        self.bump("workspaces");
        if let Some(workspace) = self.workspaces.get(&id).copied() {
            self.bump(format!("workspace:{workspace}:stats"));
        } else {
            self.bump("workspace-stats-baseline");
        }
    }

    fn invalidate_part(&mut self, part_id: i64) {
        self.bump(format!("part:{part_id}"));
        for section in ["input", "metadata", "output"] {
            self.bump(format!("part:{part_id}:{section}"));
        }
    }

    fn record_file_fact(
        &mut self,
        id: i64,
        part: &agena_storage::store::Part,
        added: bool,
        prepared: Option<&PreparedPartChange>,
    ) {
        let key = (id, part.part_id);
        let stamp = (part.revision, part.updated_at_ms);
        let previous = self.file_facts.get(&key);
        if previous.is_some_and(|previous| previous.stamp >= stamp) {
            return;
        }
        let source = if let Some(prepared) = prepared {
            prepared.source
        } else if part.origin_session_id == id {
            crate::rest::recorded_file_source(part)
        } else {
            0
        };
        let fact = if let Some(prepared) = prepared {
            prepared.fact
        } else if source == 0 {
            None
        } else if let Some(previous) = previous.filter(|previous| previous.source == source) {
            previous.fact
        } else {
            crate::rest::recorded_file_revision(part)
        };
        // If a previously read record fell outside the bounded memo, a first
        // update that removes its evidence still needs a conservative check.
        let changed = previous.map_or(
            fact.is_some()
                || (!added
                    && source != 0
                    && self.resources.contains_key(&format!("session:{id}:files"))),
            |previous| previous.fact != fact,
        );
        self.file_facts.insert(
            key,
            FileRevision {
                stamp,
                source,
                fact,
            },
        );
        if changed {
            self.bump(format!("session:{id}:files"));
        }
    }

    fn changed_prepared(&mut self, change: &SessionChange, prepared: Option<&PreparedPartChange>) {
        let id = change.session_id();
        match change {
            SessionChange::SessionMetaUpdated { meta, .. } => {
                if meta.is_temporary() {
                    return;
                }
                if self
                    .sessions
                    .get(&id)
                    .is_some_and(|previous| previous.version >= meta.version)
                {
                    return;
                }
                let list_changed = self.sessions.get(&id).is_none_or(|previous| {
                    previous.updated_at_ms != meta.updated_at_ms
                        || previous.title != meta.title
                        || previous.subtask_status != meta.subtask_status
                        || previous.pinned != meta.pinned
                        || previous.favorite != meta.favorite
                        || previous.workspace_id != meta.workspace_id
                        || previous.parent_id != meta.parent_id
                        || previous.lifecycle_state != meta.lifecycle_state
                });
                let stats_changed = self.sessions.get(&id).is_none_or(|previous| {
                    previous.pinned != meta.pinned
                        || previous.workspace_id != meta.workspace_id
                        || previous.parent_id != meta.parent_id
                        || previous.lifecycle_state != meta.lifecycle_state
                });
                let previous_flags = self
                    .sessions
                    .get(&id)
                    .map(|previous| (previous.pinned, previous.favorite, previous.state))
                    .or_else(|| (meta.version > 1).then_some((true, true, None)));
                let state = match meta.lifecycle_state {
                    SessionLifecycleState::Creating => Some(SessionStateKind::Creating),
                    SessionLifecycleState::Failed => Some(SessionStateKind::Failed),
                    _ => self
                        .sessions
                        .get(&id)
                        .filter(|old| old.lifecycle_state == meta.lifecycle_state)
                        .and_then(|old| old.state)
                        .or_else(|| (meta.version == 1).then_some(SessionStateKind::Ready)),
                };
                let previous_workspace_id = self
                    .workspaces
                    .get(&id)
                    .copied()
                    .filter(|old| *old != meta.workspace_id);
                let previous_parent = self
                    .sessions
                    .get(&id)
                    .map(|old| (old.workspace_id, old.parent_id));
                self.sessions.insert(
                    id,
                    SessionRevision {
                        version: meta.version,
                        updated_at_ms: meta.updated_at_ms,
                        title: meta.title.clone(),
                        subtask_status: meta.subtask_status.clone(),
                        workspace_id: meta.workspace_id,
                        previous_workspace_id,
                        parent_id: meta.parent_id,
                        pinned: meta.pinned,
                        favorite: meta.favorite,
                        lifecycle_state: meta.lifecycle_state,
                        state,
                    },
                );
                self.deleted_sessions.remove(&id);
                match self.workspaces.insert(id, meta.workspace_id) {
                    Some(old) if old != meta.workspace_id => {
                        self.bump(format!("workspace:{old}:sessions"));
                        self.bump(format!("workspace:{old}:stats"));
                        if let Some((_, parent)) = previous_parent {
                            let suffix = parent.map_or_else(
                                || "roots".into(),
                                |parent| format!("parent:{parent}"),
                            );
                            self.bump(format!("workspace:{old}:sessions:{suffix}"));
                        }
                    }
                    None if meta.version > 1 => self.bump("workspace-sessions-baseline"),
                    _ => {}
                }
                if list_changed {
                    self.lists_changed(id);
                    self.buckets_changed(id, previous_flags);
                }
                if let Some((workspace, parent)) =
                    previous_parent.filter(|old| old.1 != meta.parent_id)
                {
                    let suffix =
                        parent.map_or_else(|| "roots".into(), |parent| format!("parent:{parent}"));
                    self.bump(format!("workspace:{workspace}:sessions:{suffix}"));
                }
                if previous_parent.is_none()
                    || previous_parent.is_some_and(|old| old.1 != meta.parent_id)
                {
                    if let Some(parent) = meta.parent_id {
                        self.parent_row_changed(parent);
                    }
                    if let Some((_, Some(parent))) = previous_parent {
                        self.parent_row_changed(parent);
                    }
                }
                if stats_changed {
                    self.stats_changed(id);
                }
                self.bump(format!("session:{id}:state"));
                self.bump(format!("session:{id}:parts"));
            }
            SessionChange::SessionDeleted {
                workspace_id,
                temporary,
                ..
            } => {
                if *temporary {
                    return;
                }
                // Invalidate the deleted row's precise list before dropping
                // the association, including the old bucket's workspace.
                self.lists_changed(id);
                let parent = self.sessions.get(&id).and_then(|meta| meta.parent_id);
                let previous_buckets = self
                    .sessions
                    .get(&id)
                    .map(|meta| (meta.pinned, meta.favorite, meta.state));
                self.buckets_changed(id, previous_buckets);
                self.workspaces.remove(&id);
                let previous_flags = self
                    .sessions
                    .remove(&id)
                    .map(|meta| (meta.pinned, meta.favorite, meta.state));
                self.buckets_changed(id, previous_flags);
                if let Some(parent) = parent {
                    self.parent_row_changed(parent);
                }
                self.deleted_sessions.insert(id, ());
                let parts: std::collections::HashSet<_> = self
                    .part_sessions
                    .iter()
                    .filter(|(_, session)| **session == id)
                    .map(|(part, _)| *part)
                    .chain(
                        self.parts
                            .keys()
                            .filter(|(session, _)| *session == id)
                            .map(|(_, part)| *part),
                    )
                    .collect();
                for part in parts {
                    self.invalidate_part(part);
                    self.part_sessions.remove(&part);
                    self.tool_sections.remove(&part);
                }
                self.parts.retain(|(session, _), _| *session != id);
                self.file_facts.retain(|(session, _), _| *session != id);
                self.plans.remove(&id);
                self.bump(format!("workspace:{workspace_id}:sessions"));
                self.bump("sessions");
                self.bump("workspaces");
                self.bump(format!("workspace:{workspace_id}:stats"));
                self.bump(format!("session:{id}:state"));
                self.bump(format!("session:{id}:files"));
                self.bump(format!("session:{id}:plan"));
                self.bump(format!("session:{id}:parts"));
            }
            SessionChange::PartAdded { part, .. } | SessionChange::PartUpdated { part, .. } => {
                self.record_file_fact(
                    id,
                    part,
                    matches!(change, SessionChange::PartAdded { .. }),
                    prepared,
                );
                let key = (id, part.part_id);
                let interaction = part.content.get("user_input").cloned().unwrap_or_default();
                let previous = self.parts.get(&key);
                if previous.is_some_and(|(revision, updated_at, _, _)| {
                    (*revision, *updated_at) >= (part.revision, part.updated_at_ms)
                }) {
                    return;
                }
                let state_changed = previous.is_none_or(|(_, _, state, user_input)| {
                    *state != part.state || *user_input != interaction
                });
                // Ordinary tool progress cannot change the derived session
                // state. Only run liveness and entering/leaving a user-input
                // gate affect navigation; pending -> in_progress is still
                // the same live message.
                let navigation_changed = if part.origin_session_id != id {
                    false
                } else if part.is_run_marker() {
                    match previous {
                        Some((_, _, state, _)) => state.is_in_flight() != part.state.is_in_flight(),
                        None => {
                            part.role != agena_storage::store::PartRole::Runtime
                                || part.state.is_in_flight()
                        }
                    }
                } else if part.kind == "tool_call" {
                    let pending = awaiting_interaction(part.state, &interaction);
                    match previous {
                        Some((_, _, state, user_input)) => {
                            pending != awaiting_interaction(*state, user_input)
                        }
                        // A gate can predate this server's first observation.
                        // Its retained request/reply history still identifies
                        // its first observed settlement as a navigation change.
                        None => interaction
                            .get("requests")
                            .and_then(serde_json::Value::as_array)
                            .is_some_and(|requests| !requests.is_empty()),
                    }
                } else {
                    false
                };
                self.parts.insert(
                    key,
                    (part.revision, part.updated_at_ms, part.state, interaction),
                );
                // The transcript carries the session version as well as its
                // user-visible projection, including hidden-part mutations.
                self.bump(format!("session:{id}:parts"));
                if !part.visibility.visible_to_user() {
                    return;
                }
                self.bump(format!("part:{}", part.part_id));
                if part.kind == "tool_call" {
                    let next = prepared
                        .and_then(|prepared| prepared.sections)
                        .unwrap_or_else(|| tool_section_hashes(part));
                    let previous = self.tool_sections.get(&part.part_id).copied();
                    self.tool_sections.insert(part.part_id, next);
                    for (index, section) in ["input", "metadata", "output"].into_iter().enumerate()
                    {
                        if previous.is_none_or(|previous| previous[index] != next[index]) {
                            self.bump(format!("part:{}:{section}", part.part_id));
                        }
                    }
                }
                self.part_sessions
                    .insert(part.part_id, part.origin_session_id);
                if state_changed && matches!(part.kind.as_str(), "run" | "tool_call") {
                    self.bump(format!("session:{id}:state"));
                }
                if navigation_changed {
                    self.lists_changed(id);
                    let previous_flags = self
                        .sessions
                        .get(&id)
                        .map(|meta| (meta.pinned, meta.favorite, meta.state));
                    // A submitted user message is already terminal: it changes
                    // row counts/recency, but cannot change running/attention
                    // membership or workspace statistics.
                    let state_membership_changed =
                        !(matches!(change, SessionChange::PartAdded { .. })
                            && part.is_run_marker()
                            && part.state.is_terminal());
                    if state_membership_changed && let Some(meta) = self.sessions.get_mut(&id) {
                        meta.state = None;
                    }
                    self.buckets_changed(id, previous_flags);
                    if state_membership_changed {
                        self.stats_changed(id);
                    }
                }
            }
            SessionChange::PartRemoved { part_id, .. } => {
                self.bump(format!("session:{id}:parts"));
                self.invalidate_part(*part_id);
                self.parts.remove(&(id, *part_id));
                self.tool_sections.remove(part_id);
                self.part_sessions.remove(part_id);
                if self
                    .file_facts
                    .remove(&(id, *part_id))
                    .is_none_or(|previous| previous.fact.is_some())
                {
                    self.bump(format!("session:{id}:files"));
                }
                self.bump(format!("session:{id}:state"));
                self.lists_changed(id);
                if let Some(meta) = self.sessions.get_mut(&id) {
                    meta.state = None;
                }
                self.buckets_changed(id, None);
                self.stats_changed(id);
            }
        }
    }

    fn signal(&mut self, signal: &RuntimeLiveSignal) {
        match signal {
            RuntimeLiveSignal::Activity(event) => {
                let id = &event.activity_id;
                let dismissed = event.reason == BackgroundActivityEventReason::Dismissed;
                if self
                    .activities
                    .get(id)
                    .is_some_and(|(time, previous, removed)| {
                        *time > event.ts_ms
                            || (*time == event.ts_ms
                                && previous == &event.activity
                                && *removed == dismissed)
                    })
                {
                    return;
                }
                let representation_changed =
                    self.activities
                        .get(id)
                        .is_none_or(|(_, previous, removed)| {
                            previous != &event.activity || *removed != dismissed
                        });
                let logs_changed = event.reason == BackgroundActivityEventReason::LogsChanged
                    || self
                        .activities
                        .get(id)
                        .is_none_or(|(_, previous, removed)| {
                            previous.last_seq != event.activity.last_seq
                                || previous.has_more != event.activity.has_more
                                || previous.dropped_lines != event.activity.dropped_lines
                                || previous.status != event.activity.status
                                || previous.exit_code != event.activity.exit_code
                                || *removed != dismissed
                        });
                if logs_changed {
                    self.bump(format!("activity:{id}:logs"));
                }
                if representation_changed {
                    self.bump(format!("activity:{id}"));
                }
                // The JSON representations contain log cursors too, so their
                // ETags must advance. Web consumers apply these complete SSE
                // descriptors directly and do not reload list/state bodies.
                self.activities
                    .insert(id.clone(), (event.ts_ms, event.activity.clone(), dismissed));
                if representation_changed {
                    self.bump("activities");
                    if let Some(id) = event
                        .activity
                        .parent_session_id
                        .or(event.activity.session_id)
                    {
                        self.bump(format!("session:{id}:state"));
                    }
                }
            }
            RuntimeLiveSignal::Plugin {
                session_id: Some(id),
                kind_label,
                payload,
                ..
            } if kind_label == "plan.changed" => {
                if self.plans.get(id) == Some(payload) {
                    return;
                }
                self.plans.insert(*id, payload.clone());
                self.bump(format!("session:{id}:plan"));
            }
            _ => {}
        }
    }

    fn revision(&self, key: &str) -> i64 {
        let revision = self
            .resources
            .get(key)
            .copied()
            .unwrap_or(self.baseline.max(self.resource_floor));
        if key.starts_with("workspace:") {
            let workspace_baseline = key
                .split(':')
                .nth(1)
                .and_then(|id| {
                    self.resources
                        .get(&format!("workspace:{id}:sessions-baseline"))
                })
                .copied()
                .unwrap_or(self.baseline);
            revision
                .max(if key.ends_with(":stats") {
                    self.baseline
                } else {
                    workspace_baseline
                })
                .max(
                    self.resources
                        .get(if key.ends_with(":stats") {
                            "workspace-stats-baseline"
                        } else {
                            "workspace-sessions-baseline"
                        })
                        .copied()
                        .unwrap_or(self.baseline),
                )
        } else {
            if key.starts_with("sessions:bucket:") {
                revision.max(
                    self.resources
                        .get("session-buckets-baseline")
                        .copied()
                        .unwrap_or(self.baseline),
                )
            } else {
                revision
            }
        }
    }
}

pub(crate) struct ResourceRevisions {
    epoch: String,
    clock: Arc<Mutex<Clock>>,
    _subscription: GlobalSubscription,
    signal_task: Option<tokio::task::JoinHandle<()>>,
    _signal_observation: Option<agena_runtime::RuntimeLiveSignalObservation>,
    durable: tokio::sync::Mutex<DurableSnapshot>,
}

#[derive(Default)]
struct DurableSnapshot {
    checked_at: Option<Instant>,
    last_attempt: Option<Instant>,
    sessions: HashMap<i64, (i64, i64)>,
    workspaces: Vec<(i64, i64, String)>,
}

impl ResourceRevisions {
    pub(crate) fn new(state: &AppState) -> Result<Self, ServerError> {
        let baseline = state.server().started_at.timestamp_millis();
        let clock = Arc::new(Mutex::new(Clock {
            updated_at_ms: baseline,
            baseline,
            ..Clock::default()
        }));
        let observer = clock.clone();
        let subscription = state
            .session_store()?
            .subscribe_all(Arc::new(move |change| {
                observe_clock_change(&observer, &change);
            }));
        let observer = clock.clone();
        let signal_observation = state.live_signals()?.observe(Arc::new(move |signal| {
            observer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .signal(signal);
        }));
        let signal_task = if signal_observation.is_none() {
            let mut signals = state.live_signals()?.subscribe();
            let observer = clock.clone();
            Some(tokio::spawn(async move {
                while let Some(signal) = signals.recv().await {
                    let mut clock = observer.lock().unwrap_or_else(|e| e.into_inner());
                    match signal {
                        RuntimeLiveSignalItem::Signal(signal) => clock.signal(&signal),
                        RuntimeLiveSignalItem::Lagged(_) => clock.reset(),
                    }
                }
            }))
        } else {
            None
        };
        Ok(Self {
            epoch: state.server().id.to_string(),
            clock,
            _subscription: subscription,
            signal_task,
            _signal_observation: signal_observation,
            durable: tokio::sync::Mutex::new(DurableSnapshot::default()),
        })
    }

    pub(crate) fn bump(&self, key: &str) {
        self.clock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bump(key);
    }

    pub(crate) fn register_part(&self, part: &agena_storage::store::Part) {
        let part_id = part.part_id;
        let sections = (part.kind == "tool_call").then(|| tool_section_hashes(part));
        let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
        clock.part_sessions.insert(part_id, part.origin_session_id);
        if part.kind == "tool_call" && !clock.tool_sections.contains_key(&part_id) {
            clock.tool_sections.insert(part_id, sections.unwrap());
        }
    }

    pub(crate) fn register_file_facts(&self, id: i64, parts: &[agena_storage::store::Part]) {
        let resource = format!("session:{id}:files");
        let (revision, candidates) = {
            let clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
            let available = 8192_usize.saturating_sub(clock.file_facts.len());
            let candidates = parts
                .iter()
                .rev()
                .take(8192)
                .filter(|part| !clock.file_facts.contains_key(&(id, part.part_id)))
                .take(available)
                .collect::<Vec<_>>();
            (clock.revision(&resource), candidates)
        };
        // Decoding durable edit results can be expensive. Do it outside the
        // synchronous mutation observer's clock lock, retaining no part clone.
        let seeds = candidates
            .into_iter()
            .map(|part| {
                let source = if part.origin_session_id == id {
                    crate::rest::recorded_file_source(part)
                } else {
                    0
                };
                let fact = (source != 0)
                    .then(|| crate::rest::recorded_file_revision(part))
                    .flatten();
                (
                    (id, part.part_id),
                    FileRevision {
                        stamp: (part.revision, part.updated_at_ms),
                        source,
                        fact,
                    },
                )
            })
            .collect::<Vec<_>>();
        let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
        // A mutation or deletion during decoding owns the newer baseline.
        if clock.revision(&resource) != revision || clock.deleted_sessions.contains_key(&id) {
            return;
        }
        for (key, seed) in seeds {
            if clock.file_facts.len() >= 8192 {
                break;
            }
            if !clock.file_facts.contains_key(&key) {
                clock.file_facts.insert(key, seed);
            }
        }
    }

    pub(crate) async fn refresh_workspaces(&self, state: &AppState) -> Result<(), ServerError> {
        let mut durable = self.durable.lock().await;
        let mut rows = state
            .service()
            .workspace_revision_rows()
            .await
            .map_err(crate::rest::server_error_from_application)?;
        rows.sort_unstable();
        if durable.workspaces != rows {
            self.bump("workspaces");
            self.bump("workspaces:catalog");
            durable.workspaces = rows;
        }
        Ok(())
    }

    /// Shared, metadata-only safety net for writers in other processes. One
    /// check per server per 30s, regardless of browser count; no part/history
    /// reads or representation serialization occur here.
    pub(crate) async fn refresh_durable(&self, state: &AppState) -> Result<(), ServerError> {
        // Local commit observers already update the clock synchronously.
        // The cross-process safety check must not queue every browser behind
        // a slow metadata read; the in-flight refresh owns the next baseline.
        let Ok(mut durable) = self.durable.try_lock() else {
            return Ok(());
        };
        if durable
            .checked_at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(30))
            || durable
                .last_attempt
                .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
        {
            return Ok(());
        }
        // Retain a short cooldown after errors or caller cancellation as well,
        // so failing reads cannot turn a revision poll into a retry storm.
        durable.last_attempt = Some(Instant::now());
        let (sessions, mut workspaces) = tokio::try_join!(
            async {
                state
                    .session_store()?
                    .session_revision_rows()
                    .await
                    .map_err(crate::rest::server_error_from_store)
            },
            async {
                state
                    .service()
                    .workspace_revision_rows()
                    .await
                    .map_err(crate::rest::server_error_from_application)
            }
        )?;
        workspaces.sort_unstable();
        let next: HashMap<i64, (i64, i64)> = sessions
            .into_iter()
            .map(|(id, workspace, version)| (id, (workspace, version)))
            .collect();
        {
            let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
            if durable.checked_at.is_some() {
                let changed: std::collections::HashSet<i64> = next
                    .iter()
                    .filter(|(id, row)| {
                        durable.sessions.get(id) != Some(row)
                            && clock
                                .sessions
                                .get(id)
                                .is_none_or(|meta| meta.version < row.1)
                    })
                    .map(|(id, _)| *id)
                    .chain(
                        durable
                            .sessions
                            .keys()
                            .filter(|id| {
                                !next.contains_key(id) && !clock.deleted_sessions.contains_key(id)
                            })
                            .copied(),
                    )
                    .collect();
                if !changed.is_empty() {
                    clock.bump("sessions");
                    clock.bump("workspaces");
                    clock.bump("session-buckets-baseline");
                }
                for id in &changed {
                    // Plan storage has a separate revision from transcripts.
                    for suffix in ["state", "files", "parts"] {
                        clock.bump(format!("session:{id}:{suffix}"));
                    }
                    if let Some((workspace, _)) = durable.sessions.get(id) {
                        clock.bump(format!("workspace:{workspace}:sessions"));
                        clock.bump(format!("workspace:{workspace}:sessions-baseline"));
                        clock.bump(format!("workspace:{workspace}:stats"));
                    }
                    if let Some((workspace, _)) = next.get(id) {
                        clock.bump(format!("workspace:{workspace}:sessions"));
                        clock.bump(format!("workspace:{workspace}:sessions-baseline"));
                        clock.bump(format!("workspace:{workspace}:stats"));
                    }
                }
                let parts = clock
                    .part_sessions
                    .iter()
                    .filter(|(_, sid)| changed.contains(sid))
                    .map(|(pid, _)| *pid)
                    .collect::<Vec<_>>();
                for part in parts {
                    clock.invalidate_part(part);
                }
                if durable.workspaces != workspaces {
                    clock.bump("workspaces");
                    clock.bump("workspaces:catalog");
                }
            }
            // The association is a bounded optimization. Unknown sessions
            // invalidate the workspace baseline instead of risking staleness.
            for (id, (workspace, version)) in next.iter().take(8192) {
                if clock
                    .sessions
                    .get(id)
                    .is_none_or(|meta| meta.version <= *version)
                {
                    clock.workspaces.insert(*id, *workspace);
                }
            }
        }
        durable.sessions = next;
        durable.workspaces = workspaces;
        durable.checked_at = Some(Instant::now());
        Ok(())
    }

    pub(crate) fn token(&self, key: &str) -> String {
        let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
        let revision = clock.observe_resource(key);
        format!("{}:{revision}", self.epoch)
    }

    /// Reuse the state projection from an already-required list read. This
    /// gives exact bucket membership without adding mutation-time DB reads.
    pub(crate) fn seed_session_list(
        &self,
        rows: &[agena_api::resource::SessionResource],
        observed: &str,
    ) {
        let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
        if observed != format!("{}:{}", self.epoch, clock.revision("sessions")) {
            return;
        }
        for row in rows {
            if clock
                .sessions
                .get(&row.id)
                .is_some_and(|old| old.version > row.version)
            {
                continue;
            }
            let previous_workspace_id = clock
                .sessions
                .get(&row.id)
                .and_then(|old| old.previous_workspace_id);
            clock.sessions.insert(
                row.id,
                SessionRevision {
                    version: row.version,
                    updated_at_ms: row.updated_at.timestamp_millis(),
                    title: row.title.clone(),
                    subtask_status: row.subtask_status.map(|status| status.as_ref().to_owned()),
                    workspace_id: row.workspace_id,
                    previous_workspace_id,
                    parent_id: row.parent_id,
                    pinned: row.pinned,
                    favorite: row.favorite,
                    lifecycle_state: row.lifecycle_state,
                    state: Some(row.state.kind()),
                },
            );
            clock.workspaces.insert(row.id, row.workspace_id);
        }
    }

    pub(crate) fn observe_change(&self, change: &SessionChange) {
        observe_clock_change(&self.clock, change);
    }

    pub(crate) fn observe_signal(&self, signal: &RuntimeLiveSignal) {
        self.clock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .signal(signal);
    }

    pub(crate) fn live_tokens(
        &self,
        change: Option<&SessionChangeResource>,
        signal: Option<&RuntimeSignalResource>,
    ) -> BTreeMap<String, String> {
        let clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
        let mut keys = Vec::new();
        if let Some(change) = change {
            let id = change.session_id();
            keys.extend([
                "sessions".to_owned(),
                "workspaces".to_owned(),
                "workspaces:catalog".to_owned(),
                format!("session:{id}:state"),
                format!("session:{id}:files"),
                format!("session:{id}:parts"),
            ]);
            keys.extend(
                ["pinned", "favorite", "recent", "running", "attention"]
                    .map(|bucket| format!("sessions:bucket:{bucket}")),
            );
            keys.extend(
                ["pinned", "favorite", "recent", "running", "attention"]
                    .map(|bucket| format!("sessions:bucket:{bucket}:count")),
            );
            match change {
                SessionChangeResource::PartAdded { part, .. }
                | SessionChangeResource::PartUpdated { part, .. } => {
                    keys.push(format!("part:{}", part.part_id));
                    if part.kind == "tool_call" {
                        for section in ["input", "metadata", "output"] {
                            keys.push(format!("part:{}:{section}", part.part_id));
                        }
                    }
                }
                SessionChangeResource::PartRemoved { part_id, .. } => {
                    keys.push(format!("part:{part_id}"));
                    for section in ["input", "metadata", "output"] {
                        keys.push(format!("part:{part_id}:{section}"));
                    }
                }
                _ => {}
            }
            if let Some(workspace) = clock.workspaces.get(&id) {
                keys.push(format!("workspace:{workspace}:sessions"));
                keys.push(format!("workspace:{workspace}:stats"));
                if let Some(old) = clock
                    .sessions
                    .get(&id)
                    .and_then(|meta| meta.previous_workspace_id)
                {
                    keys.push(format!("workspace:{old}:sessions"));
                    keys.push(format!("workspace:{old}:stats"));
                }
            } else if let SessionChangeResource::SessionDeleted { workspace_id, .. } = change {
                keys.push(format!("workspace:{workspace_id}:sessions"));
                keys.push(format!("workspace:{workspace_id}:stats"));
            } else {
                keys.extend(
                    clock
                        .resources
                        .range("workspace:".to_owned()..)
                        .take_while(|(key, _)| key.starts_with("workspace:"))
                        .map(|(key, _)| key.clone()),
                );
            }
        }
        // Fine list clocks are registered by the first read. Sending their
        // unchanged tokens is cheap and only changed clocks wake consumers.
        if change.is_some() {
            let prefixes = keys
                .iter()
                .filter(|key| key.starts_with("workspace:"))
                .filter_map(|key| key.split(':').nth(1))
                .map(|id| format!("workspace:{id}:sessions:"))
                .collect::<std::collections::HashSet<_>>();
            for prefix in prefixes {
                keys.extend(
                    clock
                        .resources
                        .range(prefix.clone()..)
                        .take_while(|(key, _)| key.starts_with(&prefix))
                        .map(|(key, _)| key.clone()),
                );
            }
        }
        if let Some(signal) = signal {
            if signal.kind == "activity" {
                keys.push("activities".to_owned());
                if let Some(id) = signal
                    .payload
                    .get("activity_id")
                    .and_then(serde_json::Value::as_str)
                {
                    keys.push(format!("activity:{id}"));
                    keys.push(format!("activity:{id}:logs"));
                }
                if let Some(id) = signal.session_id {
                    keys.push(format!("session:{id}:state"));
                }
            } else if signal.kind == "plugin"
                && signal
                    .payload
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("plan.changed")
                && let Some(id) = signal.session_id
            {
                keys.push(format!("session:{id}:plan"));
            }
        }
        keys.into_iter()
            .map(|key| {
                let token = format!("{}:{}", self.epoch, clock.revision(&key));
                (key, token)
            })
            .collect()
    }
}

impl Drop for ResourceRevisions {
    fn drop(&mut self) {
        if let Some(task) = &self.signal_task {
            task.abort();
        }
    }
}

/// Capture the token BEFORE reading data. A mutation during the read then
/// forces another read instead of attaching a newer token to an older body.
pub(crate) struct ConditionalRead {
    token: String,
}

impl ConditionalRead {
    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    pub(crate) async fn new(state: &AppState, key: &str) -> Result<Self, ServerError> {
        // Seed the durable checkpoint before the first body can be cached.
        // All later reads share one metadata recovery check per 30 seconds.
        state.revisions()?.refresh_durable(state).await?;
        Ok(Self {
            token: state.revisions()?.token(key),
        })
    }
    pub(crate) fn not_modified(&self, headers: &HeaderMap) -> Option<Response> {
        let etag = format!("W/\"{}\"", self.token);
        let matches = headers
            .get(header::IF_NONE_MATCH)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| {
                h.split(',')
                    .any(|tag| tag.trim().trim_start_matches("W/") == etag.trim_start_matches("W/"))
            });
        matches.then(|| self.headers(StatusCode::NOT_MODIFIED.into_response()))
    }
    pub(crate) async fn json<T: Serialize + Send + 'static>(
        &self,
        data: T,
    ) -> Result<Response, ServerError> {
        Ok(self.headers(crate::json_codec::response(data).await?))
    }
    pub(crate) fn headers(&self, mut response: Response) -> Response {
        if let Some((_, updated_at)) = self.token.rsplit_once(':') {
            response.headers_mut().insert(
                "x-resource-updated-at-ms",
                HeaderValue::from_str(updated_at).expect("resource timestamp"),
            );
        }
        response.headers_mut().insert(
            header::ETAG,
            HeaderValue::from_str(&format!("W/\"{}\"", self.token)).expect("UUID revision tag"),
        );
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-cache"),
        );
        response.headers_mut().insert(
            header::VARY,
            HeaderValue::from_static("Authorization, Cookie"),
        );
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_evicts_one_revision_without_invalidating_retained_resources() {
        let mut clock = Clock::default();
        let forgotten = clock.observe_resource("part:old");
        clock.bump("workspace:7:sessions:roots");
        let retained = clock.observe_resource("workspace:7:sessions:roots");
        for id in 0..MAX_RESOURCE_REVISIONS + 32 {
            clock.bump(format!("part:{id}"));
            assert_eq!(
                clock.observe_resource("workspace:7:sessions:roots"),
                retained
            );
        }
        assert_eq!(clock.resources.len(), MAX_RESOURCE_REVISIONS);
        assert_eq!(clock.resource_order.len(), MAX_RESOURCE_REVISIONS);
        assert!(clock.observe_resource("part:old") > forgotten);
        assert_eq!(clock.revision("workspace:7:sessions:roots"), retained);
        assert_eq!(clock.baseline, 0);
    }

    #[test]
    fn evicted_baselines_keep_their_effect_on_retained_list_revisions() {
        let mut clock = Clock::default();
        clock.observe_resource("workspace:7:sessions:roots");
        clock.bump("workspace:7:sessions-baseline");
        let baseline = clock.revision("workspace:7:sessions:roots");
        for id in 0..MAX_RESOURCE_REVISIONS + 32 {
            clock.bump(format!("part:{id}"));
            assert_eq!(
                clock.observe_resource("workspace:7:sessions:roots"),
                baseline
            );
        }
        assert!(
            !clock
                .resources
                .contains_key("workspace:7:sessions-baseline")
        );
        assert_eq!(clock.revision("workspace:7:sessions:roots"), baseline);
        assert_eq!(clock.baseline, 0);
    }

    #[test]
    fn revision_memos_evict_individual_old_observations_and_keep_updated_entries() {
        let mut memo = RevisionMemo::<i64, i64, 2>::default();
        memo.insert(1, 1);
        memo.insert(2, 2);
        memo.insert(1, 3);
        memo.insert(4, 4);
        assert_eq!(memo.get(&1), Some(&3));
        assert_eq!(memo.get(&2), None);
        assert_eq!(memo.get(&4), Some(&4));
        assert_eq!(memo.len(), 2);
    }

    #[test]
    fn resource_time_keeps_advancing_when_wall_time_is_behind() {
        let future = chrono::Utc::now().timestamp_millis() + 60_000;
        let mut clock = Clock {
            updated_at_ms: future,
            ..Clock::default()
        };
        clock.bump("sessions");
        let first = clock.revision("sessions");
        clock.bump("sessions");
        assert!(first > future);
        assert!(clock.revision("sessions") > first);
    }

    #[test]
    fn equal_millisecond_activity_progress_and_dismissal_are_distinct_revisions() {
        let activity: BackgroundActivity = serde_json::from_value(serde_json::json!({
            "id":"proc_test", "kind":"shell", "status":"running", "title":"test", "description":"test",
            "session_id":7, "created_at_ms":1, "started_at_ms":1, "last_seq":1,
            "has_more":false, "dropped_lines":0, "cancellable":true, "dismissible":true
        })).unwrap();
        let mut event = agena_domain::BackgroundActivityChangedEvent {
            activity_id: activity.id.clone(),
            reason: BackgroundActivityEventReason::Updated,
            activity,
            ts_ms: 1000,
        };
        let mut clock = Clock::default();
        clock.signal(&RuntimeLiveSignal::Activity(Box::new(event.clone())));
        let initial = clock.revision("activity:proc_test");
        event.activity.last_seq = 2;
        clock.signal(&RuntimeLiveSignal::Activity(Box::new(event.clone())));
        let progress = clock.revision("activity:proc_test");
        assert!(progress > initial);
        let descriptor = clock.revision("activities");
        clock.signal(&RuntimeLiveSignal::Activity(Box::new(event.clone())));
        assert_eq!(
            clock.revision("activity:proc_test"),
            progress,
            "duplicate observers do not advance tokens"
        );
        event.reason = BackgroundActivityEventReason::Dismissed;
        clock.signal(&RuntimeLiveSignal::Activity(Box::new(event)));
        assert!(clock.revision("activity:proc_test") > progress);
        assert!(clock.revision("activities") > descriptor);
    }
}

#[cfg(test)]
mod log_granularity_tests {
    use super::*;

    #[test]
    fn activity_progress_keeps_log_clock_and_same_cursor_streaming_keeps_descriptor_clock() {
        let activity: BackgroundActivity = serde_json::from_value(serde_json::json!({
            "id":"task_test", "kind":"task", "status":"running", "title":"test", "description":"test",
            "session_id":7, "created_at_ms":1, "started_at_ms":1, "last_seq":1,
            "has_more":false, "dropped_lines":0, "cancellable":true, "dismissible":true
        })).unwrap();
        let mut event = agena_domain::BackgroundActivityChangedEvent {
            activity_id: activity.id.clone(),
            reason: BackgroundActivityEventReason::Started,
            activity,
            ts_ms: 1,
        };
        let mut clock = Clock::default();
        clock.signal(&RuntimeLiveSignal::Activity(Box::new(event.clone())));
        let logs = clock.revision("activity:task_test:logs");
        event.ts_ms = 2;
        event.reason = BackgroundActivityEventReason::Updated;
        event.activity.message = Some("installing 2/3".into());
        clock.signal(&RuntimeLiveSignal::Activity(Box::new(event.clone())));
        assert_eq!(clock.revision("activity:task_test:logs"), logs);
        let descriptor = clock.revision("activity:task_test");
        let list = clock.revision("activities");
        let state = clock.revision("session:7:state");
        event.ts_ms = 3;
        event.reason = BackgroundActivityEventReason::LogsChanged;
        clock.signal(&RuntimeLiveSignal::Activity(Box::new(event.clone())));
        assert!(clock.revision("activity:task_test:logs") > logs);
        assert_eq!(clock.revision("activity:task_test"), descriptor);
        assert_eq!(clock.revision("activities"), list);
        assert_eq!(clock.revision("session:7:state"), state);
        let logs = clock.revision("activity:task_test:logs");
        event.ts_ms = 4;
        event.reason = BackgroundActivityEventReason::Finished;
        event.activity.status = agena_domain::BackgroundActivityStatus::Succeeded;
        clock.signal(&RuntimeLiveSignal::Activity(Box::new(event)));
        assert!(clock.revision("activity:task_test:logs") > logs);
    }
}
