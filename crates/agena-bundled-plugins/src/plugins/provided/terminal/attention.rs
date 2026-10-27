//! Canonical attention policy owned by `agena.terminal`. Both delivery
//! adapters consume the same persisted, plugin-emitted `host.notify` events.
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};

use agena_domain::ExecutionId;
use agena_plugin_host::sdk::{
    AttentionKind, AttentionNotification,
    host_api::{HostClient, PluginNotifyRequest},
};
use agena_storage::{
    instance_state::InstanceStateRepository,
    kernel::{
        ChangeKind, ExecutionPhase, ExecutionView, InteractionRequest, KernelChange, KernelReader,
        SessionId, WatchError,
    },
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

const STATE_KEY: &str = "notifications.attention";
pub(super) const HISTORY_LIMIT: usize = 512;
const TTL_MS: i64 = 24 * 60 * 60 * 1000;

// Runtime snapshots overlap briefly during a reload. Only one plugin task
// may write an instance's attention history; its successor loads the latest
// state after the predecessor releases ownership on shutdown.
fn observer_ownership(path: &Path) -> Arc<tokio::sync::Mutex<()>> {
    type Owners = BTreeMap<std::path::PathBuf, Weak<tokio::sync::Mutex<()>>>;
    static OWNERS: OnceLock<Mutex<Owners>> = OnceLock::new();
    let mut owners = OWNERS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    owners.retain(|_, owner| owner.strong_count() > 0);
    if let Some(owner) = owners.get(path).and_then(Weak::upgrade) {
        return owner;
    }
    let owner = Arc::new(tokio::sync::Mutex::new(()));
    owners.insert(path.to_owned(), Arc::downgrade(&owner));
    owner
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Runtime capabilities supplied at the bundled-plugin composition boundary.
#[derive(Clone)]
pub struct NotificationSource {
    pub reader: KernelReader,
    pub state: Arc<dyn InstanceStateRepository>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct AttentionState {
    active: BTreeMap<ExecutionId, SessionId>,
    seen: VecDeque<String>,
    recent: VecDeque<PluginNotifyRequest>,
}

struct Observer {
    source: NotificationSource,
    state: AttentionState,
    host: Arc<dyn HostClient>,
}

impl Observer {
    async fn load(source: NotificationSource, host: Arc<dyn HostClient>) -> Result<Self> {
        let state = if let Some(payload) = source.state.get_payload(STATE_KEY).await? {
            serde_json::from_str(&payload)?
        } else {
            // Preserve in-flight recovery when upgrading from the original
            // browser-specific observer. Subscriptions/keys keep their owner.
            let mut state = AttentionState::default();
            if let Some(payload) = source.state.get_payload("notifications.webPush").await? {
                let legacy: serde_json::Value = serde_json::from_str(&payload)?;
                if let Some(active) = legacy.get("active") {
                    state.active = serde_json::from_value(active.clone())?;
                }
            }
            state
        };
        Ok(Self {
            source,
            state,
            host,
        })
    }

    async fn commit(&mut self, next: AttentionState) -> Result<()> {
        self.source
            .state
            .set_payload(STATE_KEY, serde_json::to_string(&next)?)
            .await?;
        self.state = next;
        Ok(())
    }

    async fn publish(&self, notifications: impl IntoIterator<Item = PluginNotifyRequest>) {
        let mut changed = false;
        for notification in notifications {
            if let Err(error) = self.host.notify(notification).await {
                tracing::warn!(diagnostic = %error.diagnostic.message, "attention notification publication deferred");
            } else {
                changed = true;
            }
        }
        // Publish after the queue mutation, so terminal clients never refresh
        // before the canonical observer has produced the notification.
        if changed {
            let _ = self
                .host
                .publish_event(agena_plugin_host::sdk::EventEnvelope {
                    kind: "attention.changed".into(),
                    timestamp_ms: now_ms(),
                    session_id: None,
                    payload: serde_json::json!({}),
                })
                .await;
        }
    }

    async fn observe(&mut self, change: &KernelChange) -> Result<()> {
        if !change.include_in_lists
            || !matches!(
                change.kind,
                ChangeKind::Execution | ChangeKind::Interaction | ChangeKind::Membership
            )
        {
            return Ok(());
        }
        let reader = &self.source.reader;
        let summary = match reader.summary(change.session).await {
            Ok(summary) => summary,
            Err(agena_storage::kernel::KernelError::NotFound(_)) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if summary.relation_kind == agena_domain::SessionRelationKind::Subagent {
            return Ok(());
        }
        let mut next = self.state.clone();
        let mut notifications = Vec::new();
        if change.kind == ChangeKind::Execution {
            let mut ids = BTreeSet::new();
            for id in &change.affected_entries {
                let entry = reader.entry(change.session, *id).await?;
                if entry.origin_session_id == change.session {
                    ids.insert(entry.execution_id);
                }
            }
            if change.requires_snapshot || ids.is_empty() {
                let control = reader.control_snapshot(change.session, 128).await?;
                if let Some(execution) = &control.active_execution {
                    ids.insert(execution.execution_id());
                }
                ids.extend(
                    next.active
                        .iter()
                        .filter_map(|(id, sid)| (*sid == change.session).then_some(*id)),
                );
            }
            for id in ids {
                let execution = reader.execution(change.session, id).await?;
                if let Some(notification) = execution_notification(&execution, &summary.title) {
                    notifications.push(notification);
                }
                if execution.phase.is_terminal() {
                    next.active.remove(&id);
                } else {
                    next.active.insert(id, change.session);
                }
            }
        }
        reconcile_requests(
            reader,
            change.session,
            &summary.title,
            &next,
            &mut notifications,
        )
        .await?;
        let changed = record_all(&mut next, notifications);
        self.commit(next).await?;
        self.publish(changed).await;
        Ok(())
    }

    async fn recover(&mut self) -> Result<()> {
        let reader = &self.source.reader;
        let mut next = self.state.clone();
        let mut before = None;
        let mut sessions = BTreeSet::new();
        loop {
            let page = reader
                .query_summaries(agena_storage::kernel::SummaryQuery {
                    states: vec![
                        agena_domain::SessionStateKind::Running,
                        agena_domain::SessionStateKind::AwaitingInteraction,
                    ],
                    exclude_subagents: true,
                    limit: 128,
                    before,
                    ..Default::default()
                })
                .await?;
            for summary in &page.sessions {
                sessions.insert(summary.id);
                let mut cursor = None;
                loop {
                    let work = reader.active_executions(summary.id, cursor, 128).await?;
                    for execution in &work.executions {
                        next.active.insert(execution.id, summary.id);
                    }
                    if !work.has_more {
                        break;
                    }
                    cursor = work.next;
                }
            }
            if !page.has_more {
                break;
            }
            before = page.next;
        }
        let mut notifications = Vec::new();
        for (id, sid) in next.active.clone() {
            sessions.insert(sid);
            let execution = match reader.execution(sid, id).await {
                Ok(execution) => execution,
                Err(agena_storage::kernel::KernelError::NotFound(_)) => {
                    next.active.remove(&id);
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            if let Some(notification) =
                execution_notification(&execution, &reader.summary(sid).await?.title)
            {
                notifications.push(notification);
                next.active.remove(&id);
            }
        }
        for notification in &next.recent {
            if notification
                .attention
                .as_ref()
                .is_some_and(|event| event.kind.is_request() && !event.resolved)
                && let Some(sid) = notification.session_id
            {
                sessions.insert(SessionId::try_from(sid)?);
            }
        }
        for sid in sessions {
            match reader.summary(sid).await {
                Ok(summary) => {
                    reconcile_requests(reader, sid, &summary.title, &next, &mut notifications)
                        .await?
                }
                Err(agena_storage::kernel::KernelError::NotFound(_)) => {
                    resolve_missing_requests(sid, &next, &BTreeSet::new(), &mut notifications)
                }
                Err(error) => return Err(error.into()),
            }
        }
        record_all(&mut next, notifications);
        self.commit(next).await?;
        // Replaying the persisted recent queue also repairs publication gaps
        // across crashes and host reloads; adapters dedupe by the event id.
        self.publish(
            self.state
                .recent
                .iter()
                .filter(|notification| {
                    notification.attention.as_ref().is_some_and(|event| {
                        event.kind.is_request() || event.created_at_ms + TTL_MS >= now_ms()
                    })
                })
                .cloned()
                .collect::<Vec<_>>(),
        )
        .await;
        Ok(())
    }
}

pub(super) async fn start(
    source: NotificationSource,
    host: Arc<dyn HostClient>,
) -> Result<JoinHandle<()>> {
    let observer = Observer::load(source, host).await?;
    let ownership = observer_ownership(observer.source.state.path());
    let mut watch = observer.source.reader.watch_all(None)?;
    Ok(tokio::spawn(async move {
        let _ownership = ownership.lock().await;
        let mut observer = loop {
            match Observer::load(observer.source.clone(), observer.host.clone()).await {
                Ok(current) => break current,
                Err(error) => {
                    tracing::warn!(%error, "attention history reload deferred");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        };
        if let Err(error) = observer.recover().await {
            tracing::warn!(%error, "attention startup recovery failed");
        }
        let mut recovery = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(30),
            Duration::from_secs(30),
        );
        loop {
            let result = tokio::select! {
                _ = recovery.tick() => observer.recover().await,
                change = watch.recv() => match change {
                    Ok(change) => observer.observe(&change).await,
                    Err(WatchError::Closed) => break,
                    Err(WatchError::Reset { .. }) => {
                        match observer.source.reader.watch_all(None) {
                            Ok(next) => watch = next,
                            Err(error) => { tracing::error!(%error, "attention observer stopped"); break; }
                        }
                        observer.recover().await
                    }
                }
            };
            if let Err(error) = result {
                tracing::warn!(%error, "attention observation deferred");
            }
        }
    }))
}

fn record_all(
    state: &mut AttentionState,
    notifications: Vec<PluginNotifyRequest>,
) -> Vec<PluginNotifyRequest> {
    let mut changed = Vec::new();
    for notification in notifications {
        let event = notification
            .attention
            .as_ref()
            .expect("canonical attention event");
        if let Some(existing) = state
            .recent
            .iter_mut()
            .find(|old| old.attention.as_ref().is_some_and(|old| old.id == event.id))
        {
            if event.resolved && !existing.attention.as_ref().is_some_and(|old| old.resolved) {
                *existing = notification.clone();
                changed.push(notification);
            }
            continue;
        }
        if state.seen.contains(&event.id) {
            continue;
        }
        state.seen.push_back(event.id.clone());
        while state.seen.len() > 4096 {
            state.seen.pop_front();
        }
        state.recent.push_back(notification.clone());
        changed.push(notification);
        while state.recent.len() > HISTORY_LIMIT {
            let index = state
                .recent
                .iter()
                .position(|item| {
                    item.attention
                        .as_ref()
                        .is_none_or(|event| !event.kind.is_request() || event.resolved)
                })
                .unwrap_or(0);
            state.recent.remove(index);
        }
    }
    changed
}

fn notification(
    id: String,
    kind: AttentionKind,
    sid: SessionId,
    title: &str,
    created_at_ms: i64,
) -> PluginNotifyRequest {
    PluginNotifyRequest {
        attention: Some(AttentionNotification {
            id,
            kind,
            created_at_ms,
            resolved: false,
        }),
        session_id: Some(sid.get()),
        title: title.chars().take(120).collect(),
        body: match kind {
            AttentionKind::Completed => "AI has finished replying.",
            AttentionKind::Failed => "AI stopped with an error.",
            AttentionKind::Cancelled => "AI execution was interrupted.",
            AttentionKind::Permission => "AI needs your approval to continue.",
            AttentionKind::Question => "AI is waiting for your answer.",
        }
        .to_owned(),
        severity: match kind {
            AttentionKind::Completed => "success",
            AttentionKind::Failed => "error",
            _ => "warning",
        }
        .to_owned(),
        actions: Vec::new(),
    }
}

fn execution_notification(execution: &ExecutionView, title: &str) -> Option<PluginNotifyRequest> {
    let kind = match execution.phase {
        ExecutionPhase::Succeeded => AttentionKind::Completed,
        ExecutionPhase::Failed | ExecutionPhase::Interrupted => AttentionKind::Failed,
        ExecutionPhase::Cancelled => AttentionKind::Cancelled,
        _ => return None,
    };
    Some(notification(
        format!("execution:{}:{}", execution.id, execution.generation.get()),
        kind,
        execution.session_id,
        title,
        now_ms(),
    ))
}

async fn reconcile_requests(
    reader: &KernelReader,
    sid: SessionId,
    title: &str,
    state: &AttentionState,
    notifications: &mut Vec<PluginNotifyRequest>,
) -> Result<()> {
    let mut pending = BTreeSet::new();
    let mut cursor = None;
    loop {
        let page = reader.pending_interactions(sid, cursor, 128).await?;
        for request in &page.interactions {
            if request.decision.is_some() {
                continue;
            }
            let id = format!("request:{}:{}", sid.get(), request.id.get());
            pending.insert(id.clone());
            let kind = match request.request {
                InteractionRequest::Permission { .. } => AttentionKind::Permission,
                InteractionRequest::Input { .. } => AttentionKind::Question,
            };
            notifications.push(notification(id, kind, sid, title, request.created_at_ms));
        }
        if !page.has_more {
            break;
        }
        cursor = page.next;
    }
    resolve_missing_requests(sid, state, &pending, notifications);
    Ok(())
}

fn resolve_missing_requests(
    sid: SessionId,
    state: &AttentionState,
    pending: &BTreeSet<String>,
    notifications: &mut Vec<PluginNotifyRequest>,
) {
    for existing in &state.recent {
        if existing.session_id != Some(sid.get()) {
            continue;
        }
        if let Some(event) = &existing.attention
            && event.kind.is_request()
            && !event.resolved
            && !pending.contains(&event.id)
        {
            let mut resolved = existing.clone();
            resolved
                .attention
                .as_mut()
                .expect("attention event")
                .resolved = true;
            notifications.push(resolved);
        }
    }
}

#[cfg(test)]
mod tests;
