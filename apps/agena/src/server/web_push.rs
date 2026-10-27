//! Web Push delivery adapter for plugin-owned attention notifications.
//! Event policy and recovery live in `agena.terminal`, shared with the TUI.
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use agena_application::{Application, dto::HostNotification};
use agena_secret_store::{FileSecretStore, SecretStore};
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
    #[serde(default)]
    since_ms: i64,
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
        let observer = self.clone();
        let observer_task = tokio::spawn(async move {
            let mut previous = Vec::new();
            loop {
                let notifications = application.plugin_runtime().host_notifications();
                if notifications != previous {
                    match observer
                        .sync_notifications(notifications.clone(), None)
                        .await
                    {
                        Ok(()) => previous = notifications,
                        Err(error) => {
                            tracing::warn!(%error, "plugin Web Push notifications could not be queued")
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
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

    async fn sync_notifications(
        &self,
        notifications: Vec<HostNotification>,
        new_subscriber: Option<&str>,
    ) -> Result<()> {
        let mut state = self.state.lock().await;
        if state.subscribers.is_empty() {
            return Ok(());
        }
        let mut next = state.clone();
        let mut changed = false;
        for notification in notifications {
            let Some(attention) = notification.attention else {
                continue;
            };
            if attention.resolved {
                let previous_len = next.outbox.len();
                next.outbox
                    .retain(|delivery| delivery.event.id != attention.id);
                changed |= next.outbox.len() != previous_len;
                continue;
            }
            let kind = match attention.kind.as_str() {
                "completed" => EventKind::Completed,
                "failed" => EventKind::Failed,
                "cancelled" => EventKind::Cancelled,
                "permission" => EventKind::Permission,
                "question" => EventKind::Question,
                _ => continue,
            };
            let event = PushEvent {
                id: attention.id,
                session_id: notification.session_id,
                title: notification.title,
                kind,
                created_at_ms: attention.created_at_ms,
            };
            let fresh = !next.seen.contains(&event.id);
            for (subscriber_id, subscriber) in &next.subscribers {
                let pending_for_new_browser =
                    new_subscriber == Some(subscriber_id.as_str()) && attention.kind.is_request();
                if !(fresh || pending_for_new_browser) {
                    continue;
                }
                if event.created_at_ms < subscriber.since_ms && !attention.kind.is_request() {
                    continue;
                }
                if next.outbox.iter().any(|delivery| {
                    delivery.subscriber_id == *subscriber_id && delivery.event.id == event.id
                }) {
                    continue;
                }
                next.outbox.push_back(Delivery {
                    subscriber_id: subscriber_id.clone(),
                    event: event.clone(),
                    attempts: 0,
                    next_at_ms: now_ms(),
                });
                changed = true;
            }
            if fresh {
                next.seen.push_back(event.id);
                changed = true;
            }
        }
        if !changed {
            return Ok(());
        }
        while next.seen.len() > 4096 {
            next.seen.pop_front();
        }
        while next.outbox.len() > MAX_DELIVERIES {
            next.outbox.pop_front();
        }
        self.persist(&next).await?;
        *state = next;
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
        // One network request at a time bounds load; the plugin observer runs
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
            let pending_request = matches!(
                delivery.event.kind,
                EventKind::Permission | EventKind::Question
            );
            let expired = !pending_request && delivery.event.created_at_ms + TTL_MS < now_ms();
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

#[cfg(test)]
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
        .route("/api/v1/browser-notifications/events", get(events))
        .route(
            "/api/v1/browser-notifications/subscriptions",
            post(subscribe).delete(unsubscribe),
        )
        .route(
            "/api/v1/browser-notifications/test",
            post(test_notification),
        )
}

async fn events(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let notifications = state
        .application
        .plugin_runtime()
        .host_notifications()
        .into_iter()
        .filter(|notification| notification.attention.is_some())
        .collect::<Vec<_>>();
    Json(serde_json::json!({"notifications": notifications, "snapshot_at_ms": now_ms()}))
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
    let mut subscriber = Subscriber {
        subscription: body.subscription,
        since_ms: now_ms(),
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
    let new_subscriber = !state.subscribers.contains_key(&id);
    if let Some(previous) = state.subscribers.get(&id) {
        subscriber.since_ms = previous.since_ms;
    }
    let mut next = state.clone();
    next.subscribers.insert(id.clone(), subscriber);
    app.web_push
        .persist(&next)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    *state = next;
    drop(state);
    app.web_push
        .sync_notifications(
            app.application.plugin_runtime().host_notifications(),
            new_subscriber.then_some(id.as_str()),
        )
        .await
        .map_err(|error| AppError::internal(error.to_string()))?;
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
