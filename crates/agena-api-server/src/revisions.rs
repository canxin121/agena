//! Single-owner kernel commit cursors invalidate bounded client resources.
use crate::{error::ServerError, state::AppState};
use agena_api::live::{RuntimeSignalResource, SessionChangeResource};
use agena_domain::SessionStateKind;
use agena_runtime::{RuntimeLiveSignal, RuntimeLiveSignalObservation};
use agena_storage::kernel::{KernelChange, SessionListChange, SessionListState, WatchCursor};
use axum::{
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
struct Clock {
    position: Option<WatchCursor>,
    revision: i64,
    reset_revision: i64,
    local: lru::LruCache<String, i64>,
}
impl Clock {
    fn new() -> Self {
        Self {
            position: None,
            revision: chrono::Utc::now().timestamp_millis(),
            reset_revision: chrono::Utc::now().timestamp_millis(),
            local: lru::LruCache::new(std::num::NonZeroUsize::new(8192).unwrap()),
        }
    }
    fn tick(&mut self) -> i64 {
        self.revision = chrono::Utc::now()
            .timestamp_millis()
            .max(self.revision.saturating_add(1));
        self.revision
    }
    fn observe(&mut self, cursor: &WatchCursor) {
        if self
            .position
            .as_ref()
            .is_none_or(|old| old.epoch != cursor.epoch || old.position < cursor.position)
        {
            self.position = Some(cursor.clone());
            self.tick();
        }
    }
    fn set(&mut self, key: String, revision: i64) -> i64 {
        // A forgotten key must never regain a token issued before eviction.
        if !self.local.contains(&key) && self.local.len() == self.local.cap().get() {
            self.reset_revision = self.tick();
            self.local.clear();
        }
        let revision = revision.max(self.reset_revision);
        self.local.put(key, revision);
        revision
    }
    fn reset(&mut self, position: Option<WatchCursor>) {
        self.reset_revision = self.tick();
        self.position = position;
        self.local.clear();
    }
    fn observe_signal(&mut self, signal: &RuntimeLiveSignal) {
        let RuntimeLiveSignal::Activity(activity) = signal else {
            return;
        };
        let revision = self.tick();
        self.set("activities".into(), revision);
        self.set(format!("activity:{}", activity.activity_id), revision);
        self.set(format!("activity:{}:logs", activity.activity_id), revision);
        if let Some(id) = activity
            .activity
            .parent_session_id
            .or(activity.activity.session_id)
        {
            self.set(format!("session:{id}:state"), revision);
        }
    }
}

const BUCKETS: [&str; 5] = ["pinned", "favorite", "running", "attention", "recent"];
fn bucket_membership(state: Option<&SessionListState>, bucket: &str) -> bool {
    let Some(state) = state.filter(|state| state.include_in_lists) else {
        return false;
    };
    match bucket {
        "pinned" => state.pinned,
        "favorite" => state.favorite,
        "running" => matches!(
            state.processing_state,
            SessionStateKind::Running | SessionStateKind::Creating
        ),
        "attention" => state.processing_state.is_attention(),
        "recent" => state.processing_state == SessionStateKind::Ready,
        _ => false,
    }
}

fn list_row_keys(state: &SessionListState, keys: &mut BTreeSet<String>) {
    if !state.include_in_lists {
        return;
    }
    keys.insert("sessions".into());
    let workspace = format!("workspace:{}:sessions", state.workspace_id);
    keys.insert(workspace.clone());
    keys.insert(match state.parent_session_id {
        Some(parent) => format!("{workspace}:parent:{}", parent.get()),
        None => format!("{workspace}:roots"),
    });
    for bucket in BUCKETS {
        if bucket_membership(Some(state), bucket) {
            keys.insert(format!("sessions:bucket:{bucket}"));
            keys.insert(format!("{workspace}:bucket:{bucket}"));
        }
    }
}

fn statistics_contribution(state: Option<&SessionListState>, workspace: i64) -> [bool; 5] {
    let Some(state) =
        state.filter(|state| state.include_in_lists && state.workspace_id == workspace)
    else {
        return [false; 5];
    };
    [
        true,
        state.parent_session_id.is_none(),
        state.pinned,
        state.queued_executions > 0 || state.running_executions > 0,
        state.processing_state.is_attention(),
    ]
}

fn list_change_keys(change: &SessionListChange) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let before = change.before.as_ref();
    let after = change.after.as_ref();
    if before != after {
        for state in [before, after].into_iter().flatten() {
            list_row_keys(state, &mut keys);
        }
        for bucket in BUCKETS {
            if bucket_membership(before, bucket) != bucket_membership(after, bucket) {
                keys.insert(format!("sessions:bucket:{bucket}:count"));
            }
        }
        let workspaces = [before, after]
            .into_iter()
            .flatten()
            .map(|state| state.workspace_id)
            .collect::<BTreeSet<_>>();
        for workspace in workspaces {
            if statistics_contribution(before, workspace)
                != statistics_contribution(after, workspace)
            {
                keys.insert(format!("workspace:{workspace}:stats"));
                // The directory resource includes both session counts and stats.
                keys.insert("workspaces".into());
            }
        }
    }
    // A child's creation/removal changes its parents' displayed child counts,
    // without changing those parent rows' own bucket membership or statistics.
    for parent in &change.parents {
        list_row_keys(parent, &mut keys);
    }
    keys
}

pub(crate) struct ResourceRevisions {
    epoch: String,
    clock: Arc<Mutex<Clock>>,
    _signal_observation: Option<RuntimeLiveSignalObservation>,
}
impl ResourceRevisions {
    pub(crate) fn new(state: &AppState) -> Result<Self, ServerError> {
        let clock = Arc::new(Mutex::new(Clock::new()));
        let weak = Arc::downgrade(&clock);
        let observation = state.live_signals()?.observe(Arc::new(move |signal| {
            if let Some(clock) = weak.upgrade() {
                clock
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .observe_signal(signal);
            }
        }));
        Ok(Self {
            epoch: state.server().id.to_string(),
            clock,
            _signal_observation: observation,
        })
    }
    pub(crate) fn token(&self, key: &str) -> String {
        let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
        let revision = clock
            .local
            .peek(key)
            .copied()
            .unwrap_or(clock.reset_revision)
            .max(clock.reset_revision);
        let revision = clock.set(key.to_owned(), revision);
        format!("{}:{revision}", self.epoch)
    }
    pub(crate) fn observe_kernel_change(&self, change: &KernelChange) {
        let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
        if clock.position.as_ref().is_some_and(|old| {
            old.epoch == change.cursor.epoch && old.position >= change.cursor.position
        }) {
            return;
        }
        if clock
            .position
            .as_ref()
            .is_some_and(|old| old.epoch != change.cursor.epoch)
        {
            clock.reset(Some(change.cursor.clone()));
            return;
        }
        clock.observe(&change.cursor);
        let Some(list_change) = &change.list_change else {
            // Missing scope facts cannot safely reuse any previously issued token.
            clock.reset(Some(change.cursor.clone()));
            return;
        };
        let revision = clock.revision;
        let id = change.session.get();
        let session_prefix = format!("session:{id}:");
        let affected = change
            .affected_entries
            .iter()
            .map(|entry| format!("part:{}", entry.get()))
            .collect::<Vec<_>>();
        let mut keys = clock
            .local
            .iter()
            .filter(|(key, _)| {
                key.starts_with(&session_prefix)
                    || affected
                        .iter()
                        .any(|prefix| key == &prefix || key.starts_with(&format!("{prefix}:")))
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        keys.extend([
            format!("session:{id}:state"),
            format!("session:{id}:parts"),
            format!("session:{id}:files"),
            format!("session:{id}:plan"),
        ]);
        keys.extend(list_change_keys(list_change));
        for key in keys {
            clock.set(key, revision);
        }
    }
    /// The kernel has one process owner. Observe the in-memory committed head;
    /// normal requests never rescan the database or every session's metadata.
    pub(crate) async fn refresh_durable(&self, state: &AppState) -> Result<(), ServerError> {
        let after = self
            .clock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .position
            .clone();
        let mut watch = state
            .application()
            .session_kernel()?
            .reader
            .watch_all(after)
            .map_err(|e| ServerError::internal_error(&e))?;
        if watch.needs_snapshot() {
            let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
            // An empty owner has no durable mutations to invalidate. Keep its
            // clocks stable across idle conditional reads before the first commit.
            if watch.head().is_some() || clock.position.is_some() {
                clock.reset(watch.head().cloned());
            }
            return Ok(());
        }
        loop {
            match watch.try_recv() {
                Ok(Some(change)) => self.observe_kernel_change(&change),
                Ok(None) => break,
                Err(_) => {
                    let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
                    clock.reset(watch.head().cloned());
                    break;
                }
            }
        }
        Ok(())
    }
    pub(crate) fn observe_workspace_change(&self) {
        let mut clock = self.clock.lock().unwrap_or_else(|e| e.into_inner());
        let revision = clock.tick();
        for key in ["workspaces", "workspaces:catalog"] {
            clock.set(key.into(), revision);
        }
    }
    pub(crate) fn live_tokens(
        &self,
        change: Option<&SessionChangeResource>,
        signal: Option<&RuntimeSignalResource>,
    ) -> BTreeMap<String, String> {
        let mut keys = vec!["sessions".to_owned(), "workspaces".to_owned()];
        if let Some(change) = change {
            let id = change.session_id();
            for suffix in ["state", "parts", "files"] {
                keys.push(format!("session:{id}:{suffix}"));
            }
        }
        if let Some(signal) = signal {
            keys.push("activities".to_owned());
            if let Some(id) = signal.session_id {
                keys.push(format!("session:{id}:state"));
            }
        }
        keys.into_iter()
            .map(|key| {
                let value = self.token(&key);
                (key, value)
            })
            .collect()
    }
}
/// Capture the token BEFORE reading data. A mutation during the read then
/// forces another read instead of attaching a newer token to an older body.
pub(crate) struct ConditionalRead {
    token: String,
}

impl ConditionalRead {
    pub(crate) fn for_value<T: Serialize>(
        state: &AppState,
        value: &T,
    ) -> Result<Self, ServerError> {
        let bytes =
            serde_json::to_vec(value).map_err(|error| ServerError::internal_error(&error))?;
        Ok(Self {
            token: format!("{}:{}", state.server().id, blake3::hash(&bytes).to_hex()),
        })
    }
    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    pub(crate) async fn new(state: &AppState, key: &str) -> Result<Self, ServerError> {
        // Seed the durable checkpoint before the first body can be cached.
        // Later reads drain the bounded in-memory committed feed.
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
        if let Some((_, updated_at)) = self.token.rsplit_once(':')
            && updated_at.parse::<i64>().is_ok()
        {
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
    fn row(id: i64, parent: Option<i64>) -> SessionListState {
        SessionListState {
            session: id.try_into().unwrap(),
            workspace_id: 10,
            parent_session_id: parent.map(|id| id.try_into().unwrap()),
            favorite: false,
            pinned: false,
            include_in_lists: true,
            is_subagent: false,
            processing_state: SessionStateKind::Ready,
            queued_executions: 0,
            running_executions: 0,
            metadata_version: 1,
            updated_at_ms: 100,
            user_message_count: 0,
            last_message_at_ms: None,
        }
    }
    fn changed(before: SessionListState, after: SessionListState) -> BTreeSet<String> {
        list_change_keys(&SessionListChange {
            before: Some(before),
            after: Some(after),
            parents: Vec::new(),
        })
    }

    #[test]
    fn list_row_changes_preserve_counts_stats_and_other_buckets() {
        let before = row(1, Some(2));
        let mut after = before.clone();
        after.metadata_version += 1;
        after.updated_at_ms += 1;
        let keys = changed(before, after);
        assert_eq!(
            keys,
            BTreeSet::from([
                "sessions".into(),
                "workspace:10:sessions".into(),
                "workspace:10:sessions:parent:2".into(),
                "sessions:bucket:recent".into(),
                "workspace:10:sessions:bucket:recent".into(),
            ])
        );
    }

    #[test]
    fn membership_counts_and_workspace_stats_change_independently() {
        let before = row(1, None);
        let mut after = before.clone();
        after.pinned = true;
        let keys = changed(before.clone(), after);
        assert!(keys.contains("sessions:bucket:pinned"));
        assert!(keys.contains("sessions:bucket:pinned:count"));
        assert!(keys.contains("workspace:10:stats"));
        assert!(keys.contains("workspaces"));
        assert!(!keys.contains("sessions:bucket:recent:count"));
        assert!(!keys.contains("sessions:bucket:favorite"));
        assert!(!keys.contains("workspace:11:stats"));
        assert!(!keys.contains("workspaces:catalog"));
        assert!(changed(before.clone(), before).is_empty());
    }

    #[test]
    fn parent_child_count_changes_update_rows_without_parent_bucket_counts() {
        let mut parent = row(2, None);
        parent.favorite = true;
        parent.pinned = true;
        let keys = list_change_keys(&SessionListChange {
            before: None,
            after: Some(row(3, Some(2))),
            parents: vec![parent],
        });
        assert!(keys.contains("workspace:10:sessions:parent:2"));
        assert!(keys.contains("workspace:10:sessions:roots"));
        assert!(keys.contains("sessions:bucket:favorite"));
        assert!(keys.contains("sessions:bucket:pinned"));
        assert!(!keys.contains("sessions:bucket:favorite:count"));
        assert!(!keys.contains("sessions:bucket:pinned:count"));
        assert!(keys.contains("sessions:bucket:recent:count"));
    }

    #[test]
    fn temporary_sessions_do_not_change_listing_clocks() {
        let mut before = row(1, None);
        before.include_in_lists = false;
        let mut after = before.clone();
        after.metadata_version += 1;
        after.pinned = true;
        assert!(changed(before, after).is_empty());
    }

    #[test]
    fn active_execution_statistics_do_not_imply_running_bucket_membership() {
        let mut before = row(1, None);
        before.processing_state = SessionStateKind::AwaitingInteraction;
        let mut after = before.clone();
        after.running_executions = 1;
        let keys = changed(before, after);
        assert!(keys.contains("workspace:10:stats"));
        assert!(keys.contains("sessions:bucket:attention"));
        assert!(!keys.contains("sessions:bucket:attention:count"));
        assert!(!keys.contains("sessions:bucket:running"));
    }

    #[test]
    fn bounded_revision_eviction_cannot_restore_an_old_token() {
        let mut clock = Clock::new();
        let initial = clock.reset_revision;
        clock.set("watched".into(), initial);
        let changed = clock.tick();
        clock.set("watched".into(), changed);
        for index in 0..clock.local.cap().get() {
            clock.set(format!("other:{index}"), initial);
        }
        assert!(!clock.local.contains("watched"));
        let restored = clock.set("watched".into(), initial);
        assert!(restored > changed);
    }
}
