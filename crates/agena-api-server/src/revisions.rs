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
        // Plans live in plugin storage, so their mutations need not produce
        // a kernel session change. Invalidate their HTTP representation at
        // commit notification time, before any client can read it again.
        if let RuntimeLiveSignal::Plugin {
            session_id: Some(id),
            kind_label,
            ..
        } = signal
            && kind_label == "plan.changed"
        {
            let revision = self.tick();
            self.set(format!("session:{id}:plan"), revision);
            return;
        }
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
        let files_key = format!("session:{id}:files");
        let affected = change
            .affected_entries
            .iter()
            .map(|entry| format!("part:{}", entry.get()))
            .collect::<Vec<_>>();
        let mut keys = clock
            .local
            .iter()
            .filter(|(key, _)| {
                (key.starts_with(&session_prefix) && key.as_str() != files_key)
                    || affected
                        .iter()
                        .any(|prefix| key == &prefix || key.starts_with(&format!("{prefix}:")))
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        keys.extend([
            format!("session:{id}:state"),
            format!("session:{id}:parts"),
            format!("session:{id}:plan"),
        ]);
        // File evidence does not expire when a title, provider round, text
        // capture or activity changes. Tool/membership/control mutations can
        // change that evidence; the endpoint also validates each file body.
        if matches!(
            change.kind,
            agena_storage::kernel::ChangeKind::Tool
                | agena_storage::kernel::ChangeKind::Membership
                | agena_storage::kernel::ChangeKind::Execution
                | agena_storage::kernel::ChangeKind::Interaction
        ) {
            keys.push(files_key);
        }
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
                if signal.kind == "plugin"
                    && signal
                        .payload
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        == Some("plan.changed")
                {
                    keys.push(format!("session:{id}:plan"));
                }
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
    resource_revision: Option<String>,
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
            resource_revision: None,
        })
    }
    pub(crate) fn with_resource_revision(mut self, observed: &Self) -> Self {
        self.resource_revision = Some(observed.token.clone());
        self
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
            resource_revision: None,
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
        let resource_revision = self.resource_revision.as_deref().unwrap_or(&self.token);
        if let Some((_, updated_at)) = resource_revision.rsplit_once(':')
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
            "x-resource-revision",
            HeaderValue::from_str(resource_revision).expect("resource revision"),
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

    fn revisions() -> ResourceRevisions {
        ResourceRevisions {
            epoch: "test-server".into(),
            clock: Arc::new(Mutex::new(Clock::new())),
            _signal_observation: None,
        }
    }

    fn plugin_signal(session_id: Option<i64>, kind: &str) -> RuntimeLiveSignal {
        RuntimeLiveSignal::Plugin {
            session_id,
            plugin_id: "agena.plan".parse().unwrap(),
            kind_label: kind.into(),
            payload: serde_json::json!({"revision": "new-plan-revision"}),
        }
    }

    #[test]
    fn plan_mutations_invalidate_only_the_owning_sessions_cached_plan() {
        let revisions = revisions();
        let old_plan = revisions.token("session:7:plan");
        let other_plan = revisions.token("session:8:plan");
        let old_state = revisions.token("session:7:state");
        revisions
            .clock
            .lock()
            .unwrap()
            .observe_signal(&plugin_signal(Some(7), "plan.changed"));
        let new_plan = revisions.token("session:7:plan");
        assert_ne!(new_plan, old_plan);
        assert_eq!(revisions.token("session:8:plan"), other_plan);
        assert_eq!(revisions.token("session:7:state"), old_state);

        // The same version must travel on SSE so live clients cannot reuse
        // the old body before even issuing an If-None-Match request.
        let tokens = revisions.live_tokens(
            None,
            Some(&RuntimeSignalResource {
                kind: "plugin".into(),
                session_id: Some(7),
                payload: serde_json::json!({"kind": "plan.changed"}),
            }),
        );
        assert_eq!(tokens.get("session:7:plan"), Some(&new_plan));
        assert!(!tokens.contains_key("session:8:plan"));
    }

    #[test]
    fn unrelated_plugin_signals_do_not_invalidate_cached_plans() {
        let revisions = revisions();
        let old_plan = revisions.token("session:7:plan");
        for signal in [
            plugin_signal(Some(7), "other.changed"),
            plugin_signal(None, "plan.changed"),
        ] {
            revisions.clock.lock().unwrap().observe_signal(&signal);
        }
        assert_eq!(revisions.token("session:7:plan"), old_plan);
        let tokens = revisions.live_tokens(
            None,
            Some(&RuntimeSignalResource {
                kind: "plugin".into(),
                session_id: Some(7),
                payload: serde_json::json!({"kind": "other.changed"}),
            }),
        );
        assert!(!tokens.contains_key("session:7:plan"));
    }
}
