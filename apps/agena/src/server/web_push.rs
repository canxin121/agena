//! Browser-independent delivery. The kernel observer persists an outbox before
//! the sender contacts the push service; no open page or SSE connection is needed.
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::Duration,
};

use agena_application::Application;
use agena_domain::ExecutionId;
use agena_secret_store::{FileSecretStore, SecretStore};
use agena_storage::kernel::{
    ChangeKind, ExecutionPhase, ExecutionView, InteractionRequest, KernelChange, KernelReader,
    SessionId, WatchError,
};
use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{Mutex, Notify},
    task::JoinHandle,
};
use web_push::{
    ContentEncoding, SubscriptionInfo, Urgency, VapidSignatureBuilder, WebPushMessageBuilder,
};

use super::{ApiResult, AppError, persistence::db::ServerStateDb, state::AppState};

const STATE_KEY: &str = "notifications.webPush";
const TTL_MS: i64 = 24 * 60 * 60 * 1000;
const MAX_SUBSCRIPTIONS: usize = 32;
const MAX_DELIVERIES: usize = 2048;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum EventKind {
    Completed,
    Failed,
    Cancelled,
    Permission,
    Question,
    Test,
}

#[derive(Clone, Serialize, Deserialize)]
struct Subscriber {
    subscription: SubscriptionInfo,
    messages: BTreeMap<EventKind, String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PushEvent {
    id: String,
    session_id: Option<i64>,
    title: String,
    kind: EventKind,
    created_at_ms: i64,
}

#[derive(Clone, Serialize, Deserialize)]
struct Delivery {
    subscriber_id: String,
    event: PushEvent,
    attempts: u32,
    next_at_ms: i64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct PushState {
    subscribers: BTreeMap<String, Subscriber>,
    seen: VecDeque<String>,
    outbox: VecDeque<Delivery>,
    // Track executions across watch resets and server restarts, including runs
    // whose finish creates no assistant text or exceeds the part-event budget.
    active: BTreeMap<ExecutionId, SessionId>,
}

pub(crate) struct WebPushService {
    db: Arc<ServerStateDb>,
    state: Mutex<PushState>,
    private_key: String,
    public_key: String,
    http: reqwest::Client,
    wake: Notify,
}

pub(crate) struct PushTasks(Vec<JoinHandle<()>>);
impl Drop for PushTasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

impl WebPushService {
    pub(crate) async fn load(db: Arc<ServerStateDb>) -> Result<Self> {
        Self::load_with_secrets(db, FileSecretStore::new("server.web-push")).await
    }

    async fn load_with_secrets(db: Arc<ServerStateDb>, store: FileSecretStore) -> Result<Self> {
        let (private_key, public_key) = tokio::task::spawn_blocking(move || -> Result<_> {
            let private_key = match store.get_secret("vapid-key")? {
                Some(key) => key,
                None => {
                    let mut bytes = [0; 32];
                    getrandom::fill(&mut bytes)
                        .map_err(|_| anyhow!("could not generate Web Push identity"))?;
                    store.get_or_insert_secret("vapid-key", &URL_SAFE_NO_PAD.encode(bytes))?
                }
            };
            let key = VapidSignatureBuilder::from_base64_no_sub(&private_key)
                .context("invalid persisted Web Push identity")?;
            let public_key = URL_SAFE_NO_PAD.encode(key.get_public_key());
            Ok((private_key, public_key))
        })
        .await
        .context("Web Push identity worker failed")??;
        let state = db
            .get_json(STATE_KEY)
            .await
            .map_err(anyhow::Error::msg)?
            .unwrap_or_default();
        Ok(Self {
            db,
            state: Mutex::new(state),
            private_key,
            public_key,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            wake: Notify::new(),
        })
    }

    async fn persist(&self, state: &PushState) -> Result<()> {
        self.db
            .set_json(STATE_KEY, state.clone())
            .await
            .map_err(anyhow::Error::msg)
    }

    pub(crate) fn start(self: &Arc<Self>, application: Application) -> Result<PushTasks> {
        let reader = application
            .session_kernel()
            .map_err(|e| anyhow!("{e}"))?
            .reader
            .clone();
        // Subscribe before returning, so a message submitted during startup is observed.
        let mut watch = reader.watch_all(None)?;
        let observer = self.clone();
        let observer_task = tokio::spawn(async move {
            if let Err(error) = observer.recover(&reader).await {
                tracing::warn!(%error, "Web Push startup recovery failed");
            }
            loop {
                match watch.recv().await {
                    Ok(change) => {
                        if let Err(error) = observer.observe(&reader, &change).await {
                            tracing::warn!(%error, "Web Push event observation failed");
                        }
                    }
                    Err(WatchError::Closed) => break,
                    Err(WatchError::Reset { .. }) => {
                        match reader.watch_all(None) {
                            Ok(replacement) => watch = replacement,
                            Err(error) => {
                                tracing::error!(%error, "Web Push observer stopped");
                                break;
                            }
                        }
                        if let Err(error) = observer.recover(&reader).await {
                            tracing::warn!(%error, "Web Push watch recovery failed");
                        }
                    }
                }
            }
        });
        let sender = self.clone();
        let sender_task = tokio::spawn(async move {
            loop {
                if let Err(error) = sender.send_due().await {
                    tracing::warn!(%error, "Web Push outbox could not be updated");
                }
                tokio::select! {
                    _ = sender.wake.notified() => {},
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                }
            }
        });
        Ok(PushTasks(vec![observer_task, sender_task]))
    }

    async fn observe(&self, reader: &KernelReader, change: &KernelChange) -> Result<()> {
        if !change.include_in_lists
            || !matches!(
                change.kind,
                ChangeKind::Execution | ChangeKind::Interaction | ChangeKind::Membership
            )
        {
            return Ok(());
        }
        if self.state.lock().await.subscribers.is_empty() {
            return Ok(());
        }
        let summary = match reader.summary(change.session).await {
            Ok(summary) => summary,
            Err(agena_storage::kernel::KernelError::NotFound(_)) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if summary.relation_kind == agena_domain::SessionRelationKind::Subagent {
            return Ok(());
        }
        let mut events = Vec::new();
        let mut executions = BTreeMap::new();
        if change.kind == ChangeKind::Execution {
            let mut ids = BTreeSet::new();
            for id in &change.affected_entries {
                let entry = reader.entry(change.session, *id).await?;
                if entry.origin_session_id == change.session {
                    ids.insert(entry.execution_id);
                }
            }
            // Execution changes over 128 entries have no part identities. The
            // authoritative control head still names the execution that finished.
            if change.requires_snapshot || ids.is_empty() {
                let control = reader.control_snapshot(change.session, 128).await?;
                if let Some(execution) = &control.active_execution {
                    ids.insert(execution.execution_id());
                }
                let active = self.state.lock().await.active.clone();
                ids.extend(
                    active
                        .into_iter()
                        .filter_map(|(id, sid)| (sid == change.session).then_some(id)),
                );
            }
            for id in ids {
                let execution = reader.execution(change.session, id).await?;
                if let Some(event) = execution_event(&execution, &summary.title) {
                    events.push(event);
                }
                executions.insert(
                    id,
                    (!execution.phase.is_terminal()).then_some(change.session),
                );
            }
        }
        // Requests are canonical kernel facts. Tool/part display projections
        // and the presence of an open frontend never affect their delivery.
        events.extend(pending_events(reader, change.session, &summary.title).await?);
        let mut state = self.state.lock().await;
        let mut next = state.clone();
        for (id, sid) in executions {
            if let Some(sid) = sid {
                next.active.insert(id, sid);
            } else {
                next.active.remove(&id);
            }
        }
        for event in events {
            enqueue(&mut next, event);
        }
        self.persist(&next).await?;
        *state = next;
        self.wake.notify_one();
        Ok(())
    }

    async fn recover(&self, reader: &KernelReader) -> Result<()> {
        if self.state.lock().await.subscribers.is_empty() {
            return Ok(());
        }
        let mut before = None;
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
                let mut cursor = None;
                loop {
                    let work = reader.active_executions(summary.id, cursor, 128).await?;
                    let mut state = self.state.lock().await;
                    let mut next = state.clone();
                    for execution in &work.executions {
                        next.active.insert(execution.id, summary.id);
                    }
                    self.persist(&next).await?;
                    *state = next;
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
        let active = self.state.lock().await.active.clone();
        let mut sessions = BTreeSet::new();
        for (id, sid) in active {
            sessions.insert(sid);
            let execution = match reader.execution(sid, id).await {
                Ok(execution) => execution,
                Err(agena_storage::kernel::KernelError::NotFound(_)) => {
                    let mut state = self.state.lock().await;
                    state.active.remove(&id);
                    self.persist(&state).await?;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            if let Some(event) = execution_event(&execution, &reader.summary(sid).await?.title) {
                let mut state = self.state.lock().await;
                let mut next = state.clone();
                enqueue(&mut next, event);
                next.active.remove(&id);
                self.persist(&next).await?;
                *state = next;
            }
        }
        for sid in sessions {
            let summary = match reader.summary(sid).await {
                Ok(summary) => summary,
                Err(agena_storage::kernel::KernelError::NotFound(_)) => continue,
                Err(error) => return Err(error.into()),
            };
            let events = pending_events(reader, sid, &summary.title).await?;
            let mut state = self.state.lock().await;
            let mut next = state.clone();
            for event in events {
                enqueue(&mut next, event);
            }
            self.persist(&next).await?;
            *state = next;
        }
        self.wake.notify_one();
        Ok(())
    }

    async fn send(
        &self,
        subscriber: &Subscriber,
        event: &PushEvent,
    ) -> Result<reqwest::StatusCode> {
        let private_key = self.private_key.clone();
        let subscriber = subscriber.clone();
        let event = event.clone();
        let request = tokio::task::spawn_blocking(move || -> Result<_> {
            let mut signature = VapidSignatureBuilder::from_base64(&private_key, &subscriber.subscription)?;
            signature.add_claim("sub", "https://github.com/ai-research/agena");
            let payload = serde_json::to_vec(&serde_json::json!({
                "id": event.id, "session_id": event.session_id, "title": event.title,
                "body": subscriber.messages.get(&event.kind).map(String::as_str).unwrap_or("Agena needs your attention."),
            }))?;
            let mut builder = WebPushMessageBuilder::new(&subscriber.subscription);
            builder.set_vapid_signature(signature.build()?);
            builder.set_payload(ContentEncoding::Aes128Gcm, &payload);
            builder.set_ttl((TTL_MS / 1000) as u32);
            builder.set_urgency(Urgency::High);
            Ok(web_push::request_builder::build_request::<Vec<u8>>(builder.build()?))
        }).await.context("Web Push encryption worker failed")??;
        let mut http = self.http.post(request.uri().to_string());
        for (name, value) in request.headers() {
            http = http.header(name.as_str(), value.as_bytes());
        }
        // Never include the capability endpoint, encryption keys, or response
        // bodies in errors/logs. Push-service error bodies can echo credentials.
        let response = http
            .body(request.into_body())
            .send()
            .await
            .map_err(|_| anyhow!("Web Push service connection failed"))?;
        Ok(response.status())
    }

    async fn send_due(&self) -> Result<()> {
        // One network request at a time bounds load; the kernel observer runs
        // independently and can keep persisting new events during an outage.
        for _ in 0..MAX_SUBSCRIPTIONS {
            let next = {
                let state = self.state.lock().await;
                state
                    .outbox
                    .iter()
                    .find(|item| item.next_at_ms <= now_ms())
                    .cloned()
                    .and_then(|delivery| {
                        state
                            .subscribers
                            .get(&delivery.subscriber_id)
                            .cloned()
                            .map(|subscriber| (delivery, subscriber))
                    })
            };
            let Some((delivery, subscriber)) = next else {
                break;
            };
            let expired = delivery.event.created_at_ms + TTL_MS < now_ms();
            let result = if expired {
                Ok(reqwest::StatusCode::GONE)
            } else {
                self.send(&subscriber, &delivery.event).await
            };
            let mut state = self.state.lock().await;
            let mut next = state.clone();
            let gone =
                !expired && matches!(&result, Ok(status) if matches!(status.as_u16(), 404 | 410));
            if gone {
                next.subscribers.remove(&delivery.subscriber_id);
                next.outbox
                    .retain(|item| item.subscriber_id != delivery.subscriber_id);
            } else if let Some(index) = next.outbox.iter().position(|item| {
                item.subscriber_id == delivery.subscriber_id && item.event.id == delivery.event.id
            }) {
                if expired || matches!(&result, Ok(status) if status.is_success()) {
                    next.outbox.remove(index);
                } else {
                    let item = &mut next.outbox[index];
                    item.attempts = item.attempts.saturating_add(1);
                    item.next_at_ms =
                        now_ms() + (5000_i64 * 2_i64.pow(item.attempts.min(6))).min(300_000);
                    tracing::warn!(
                        status = result.as_ref().ok().map(|s| s.as_u16()),
                        "Web Push delivery deferred; will retry"
                    );
                }
            }
            self.persist(&next).await?;
            *state = next;
        }
        Ok(())
    }
}

async fn pending_events(
    reader: &KernelReader,
    sid: SessionId,
    title: &str,
) -> Result<Vec<PushEvent>> {
    let mut events = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader.pending_interactions(sid, cursor, 128).await?;
        for request in &page.interactions {
            if request.decision.is_some() {
                continue;
            }
            events.push(PushEvent {
                id: format!("request:{}:{}", sid.get(), request.id.get()),
                session_id: Some(sid.get()),
                title: short_title(title),
                kind: match request.request {
                    InteractionRequest::Permission { .. } => EventKind::Permission,
                    InteractionRequest::Input { .. } => EventKind::Question,
                },
                created_at_ms: request.created_at_ms,
            });
        }
        if !page.has_more {
            break;
        }
        cursor = page.next;
    }
    Ok(events)
}

fn short_title(title: &str) -> String {
    title.chars().take(120).collect()
}

fn execution_event(execution: &ExecutionView, title: &str) -> Option<PushEvent> {
    let kind = match execution.phase {
        ExecutionPhase::Succeeded => EventKind::Completed,
        ExecutionPhase::Failed | ExecutionPhase::Interrupted => EventKind::Failed,
        ExecutionPhase::Cancelled => EventKind::Cancelled,
        _ => return None,
    };
    Some(PushEvent {
        id: format!("execution:{}:{}", execution.id, execution.generation.get()),
        session_id: Some(execution.session_id.get()),
        title: short_title(title),
        kind,
        created_at_ms: now_ms(),
    })
}

fn enqueue(state: &mut PushState, event: PushEvent) {
    if state.seen.contains(&event.id) {
        return;
    }
    state.seen.push_back(event.id.clone());
    while state.seen.len() > 4096 {
        state.seen.pop_front();
    }
    for subscriber_id in state.subscribers.keys() {
        state.outbox.push_back(Delivery {
            subscriber_id: subscriber_id.clone(),
            event: event.clone(),
            attempts: 0,
            next_at_ms: now_ms(),
        });
    }
    while state.outbox.len() > MAX_DELIVERIES {
        state.outbox.pop_front();
    }
}

fn validate_subscription(subscriber: &Subscriber) -> Result<()> {
    let endpoint = url::Url::parse(&subscriber.subscription.endpoint)?;
    // Authenticated clients may only register real browser push services. Do
    // not turn subscription registration into an arbitrary HTTP proxy.
    let host = endpoint.host_str().unwrap_or_default();
    let allowed = matches!(
        host,
        "fcm.googleapis.com" | "updates.push.services.mozilla.com" | "web.push.apple.com"
    ) || host.ends_with(".notify.windows.com");
    if endpoint.scheme() != "https"
        || !allowed
        || endpoint.port_or_known_default() != Some(443)
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.fragment().is_some()
        || subscriber.subscription.endpoint.len() > 4096
    {
        bail!("invalid browser push endpoint");
    }
    let key = URL_SAFE_NO_PAD
        .decode(&subscriber.subscription.keys.p256dh)
        .map_err(|_| anyhow!("invalid browser push public key"))?;
    let auth = URL_SAFE_NO_PAD
        .decode(&subscriber.subscription.keys.auth)
        .map_err(|_| anyhow!("invalid browser push authentication key"))?;
    if key.len() != 65 || key[0] != 4 || auth.len() != 16 {
        bail!("invalid browser push keys");
    }
    if [
        EventKind::Completed,
        EventKind::Failed,
        EventKind::Cancelled,
        EventKind::Permission,
        EventKind::Question,
        EventKind::Test,
    ]
    .iter()
    .any(|kind| {
        !subscriber
            .messages
            .get(kind)
            .is_some_and(|text| !text.is_empty() && text.len() <= 600)
    }) {
        bail!("invalid browser notification messages");
    }
    Ok(())
}

pub(crate) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/browser-notifications/config", get(config))
        .route(
            "/api/v1/browser-notifications/subscriptions",
            post(subscribe).delete(unsubscribe),
        )
        .route(
            "/api/v1/browser-notifications/test",
            post(test_notification),
        )
}

async fn config(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "public_key": state.web_push.public_key }))
}

#[derive(Deserialize)]
struct SubscriptionRequest {
    subscription: SubscriptionInfo,
    messages: BTreeMap<EventKind, String>,
}

async fn subscribe(
    State(app): State<Arc<AppState>>,
    Json(body): Json<SubscriptionRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let subscriber = Subscriber {
        subscription: body.subscription,
        messages: body.messages,
    };
    validate_subscription(&subscriber)
        .map_err(|_| AppError::bad_request("Invalid browser push subscription."))?;
    let id = subscriber_id(&subscriber.subscription.endpoint);
    let mut state = app.web_push.state.lock().await;
    if !state.subscribers.contains_key(&id) && state.subscribers.len() >= MAX_SUBSCRIPTIONS {
        return Err(AppError::too_many_requests(
            "Too many browser notification subscriptions.",
        ));
    }
    let mut next = state.clone();
    next.subscribers.insert(id.clone(), subscriber);
    app.web_push
        .persist(&next)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    *state = next;
    drop(state);
    let reader = app
        .application
        .session_kernel()
        .map_err(|e| AppError::internal(e.to_string()))?
        .reader
        .clone();
    app.web_push
        .recover(&reader)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(serde_json::json!({ "id": id })))
}

fn subscriber_id(endpoint: &str) -> String {
    use sha2::{Digest, Sha256};
    URL_SAFE_NO_PAD.encode(Sha256::digest(endpoint.as_bytes()))
}

#[derive(Deserialize)]
struct EndpointRequest {
    endpoint: String,
}

async fn unsubscribe(
    State(app): State<Arc<AppState>>,
    Json(body): Json<EndpointRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let id = subscriber_id(&body.endpoint);
    let mut state = app.web_push.state.lock().await;
    let mut next = state.clone();
    next.subscribers.remove(&id);
    next.outbox.retain(|item| item.subscriber_id != id);
    app.web_push
        .persist(&next)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    *state = next;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn test_notification(
    State(app): State<Arc<AppState>>,
    Json(body): Json<EndpointRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let id = subscriber_id(&body.endpoint);
    let subscriber = app
        .web_push
        .state
        .lock()
        .await
        .subscribers
        .get(&id)
        .cloned()
        .ok_or_else(|| AppError::not_found("This browser is not subscribed to notifications."))?;
    let event = PushEvent {
        id: format!("test:{}", uuid::Uuid::new_v4()),
        session_id: None,
        title: "Agena".into(),
        kind: EventKind::Test,
        created_at_ms: now_ms(),
    };
    let status = app
        .web_push
        .send(&subscriber, &event)
        .await
        .map_err(|e| AppError::bad_gateway(e.to_string()))?;
    if !status.is_success() {
        return Err(AppError::bad_gateway(format!(
            "Browser push service rejected the test (HTTP {}).",
            status.as_u16()
        )));
    }
    Ok(Json(serde_json::json!({ "id": event.id })))
}

#[cfg(test)]
mod tests;
