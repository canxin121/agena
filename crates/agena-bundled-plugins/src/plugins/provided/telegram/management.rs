//! Operator-owned bot registry and durable, per-bot update fan-out.
//!
//! Telegram acknowledges updates at account level. Bindings must never each
//! advance the upstream offset: persist once, then replay independently using
//! host-committed local sequence cursors. No message is authorized by discovery.
use super::*;
use agena_plugin_sdk::host_api::{
    HostClient, HostSecretDeleteRequest, HostSecretGetRequest, HostSecretSetRequest,
    HostStorageGetRequest, HostStorageScope, HostStorageSetRequest, HostStorageVisibility,
};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::sync::Arc;

const NAMESPACE: &str = "telegram-management-v1";
const MAX_BOTS: usize = 32;
const MAX_ROOMS: usize = 1000;
const MAX_UPDATES: usize = 2048;
const MAX_STREAM_BYTES: usize = 8 * 1024 * 1024;
const RETENTION_MS: i64 = 7 * 86_400_000;

#[derive(
    Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema, agena_macros::ToolInput,
)]
#[serde(default, deny_unknown_fields)]
pub(super) struct BotSelection {
    pub bot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BotConfiguration {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub connection: TelegramConfig,
    /// `environment` reads the named server variable. `vault` uses a write-only
    /// token stored separately through the host plugin secret store.
    pub credential_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredBot {
    #[serde(flatten)]
    config: BotConfiguration,
    secret_name: Option<String>,
    updated_at_ms: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Registry {
    revision: u64,
    bots: BTreeMap<String, StoredBot>,
    // A removed identity may not be reused by a different connection. Old
    // binding config must fail closed, never silently select a new bot.
    retired: std::collections::BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedBot {
    pub id: String,
    pub enabled: bool,
    pub connection: TelegramConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct Room {
    pub chat_id: String,
    pub title: String,
    pub username: Option<String>,
    pub kind: String,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
    pub messages_seen: u64,
    pub last_sender_id: Option<String>,
    pub last_sender_name: Option<String>,
    pub last_text: String,
    pub topics: BTreeMap<String, String>,
    pub bot_membership: Option<Value>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BufferedUpdate {
    sequence: u64,
    received_at_ms: i64,
    update_id: i64,
    value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Consumer {
    acknowledged: u64,
    seen_at_ms: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Stream {
    id: String,
    next_sequence: u64,
    floor: u64,
    #[serde(default)]
    floor_received_ms: i64,
    offset: Option<i64>,
    last_poll_ms: i64,
    retry_at_ms: i64,
    last_error: Option<String>,
    verification: Option<Value>,
    updates: Vec<BufferedUpdate>,
    rooms: BTreeMap<String, Room>,
    consumers: BTreeMap<String, Consumer>,
}
impl Default for Stream {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            next_sequence: 1,
            floor: 0,
            floor_received_ms: 0,
            offset: None,
            last_poll_ms: 0,
            retry_at_ms: 0,
            last_error: None,
            verification: None,
            updates: vec![],
            rooms: BTreeMap::new(),
            consumers: BTreeMap::new(),
        }
    }
}

impl Stream {
    fn prune(&mut self, now: i64) {
        self.consumers
            .retain(|_, consumer| now.saturating_sub(consumer.seen_at_ms) <= RETENTION_MS);
        let acknowledged = self
            .consumers
            .values()
            .map(|consumer| consumer.acknowledged)
            .min();
        let keep_tail = self.updates.len().saturating_sub(100);
        let mut count = 0;
        for (index, update) in self.updates.iter().enumerate() {
            if now.saturating_sub(update.received_at_ms) > RETENTION_MS
                || (index < keep_tail && acknowledged.is_some_and(|ack| update.sequence <= ack))
            {
                self.floor = update.sequence;
                self.floor_received_ms = update.received_at_ms;
                count += 1;
            } else {
                break;
            }
        }
        self.updates.drain(..count);
    }

    fn append(&mut self, updates: &[Value], now: i64) -> Result<()> {
        for update in updates {
            let Some(id) = update.get("update_id").and_then(Value::as_i64) else {
                return Err(PluginError::internal(
                    "Telegram returned an update without an identifier",
                ));
            };
            let next = id
                .checked_add(1)
                .ok_or_else(|| PluginError::internal("Telegram update identifier overflow"))?;
            if self.offset.is_some_and(|offset| id < offset) {
                continue;
            }
            if serde_json::to_vec(update)
                .map_err(|e| PluginError::internal_error(&e))?
                .len()
                > 128 * 1024
            {
                return Err(PluginError::internal(
                    "Telegram update exceeds the durable inbox limit; it has not been acknowledged",
                ));
            }
            self.observe(update, now);
            self.updates.push(BufferedUpdate {
                sequence: self.next_sequence,
                received_at_ms: now,
                update_id: id,
                value: update.clone(),
            });
            self.next_sequence = self
                .next_sequence
                .checked_add(1)
                .ok_or_else(|| PluginError::internal("Telegram inbox sequence overflow"))?;
            self.offset = Some(next);
        }
        if self.updates.len() > MAX_UPDATES
            || serde_json::to_vec(self)
                .map_err(|e| PluginError::internal_error(&e))?
                .len()
                > MAX_STREAM_BYTES
        {
            return Err(PluginError::internal(
                "Telegram inbox is full; resume lagging bindings before fetching more updates",
            ));
        }
        Ok(())
    }

    fn observe(&mut self, update: &Value, now: i64) {
        let Some(message) = update
            .get("message")
            .or_else(|| update.get("channel_post"))
            .or_else(|| update.get("my_chat_member"))
            .or_else(|| update.get("chat_member"))
        else {
            return;
        };
        let Some(chat) = message.get("chat") else {
            return;
        };
        let Some(id) = chat
            .get("id")
            .and_then(Value::as_i64)
            .map(|id| id.to_string())
        else {
            return;
        };
        if !self.rooms.contains_key(&id) && self.rooms.len() >= MAX_ROOMS {
            return;
        }
        let entry = self.rooms.entry(id.clone()).or_insert_with(|| Room {
            chat_id: id,
            first_seen_ms: now,
            ..Default::default()
        });
        entry.kind = conversation_kind(chat.get("type").and_then(Value::as_str))
            .map(|kind| kind.as_str().to_string())
            .unwrap_or_else(|| "unknown".into());
        entry.title = chat
            .get("title")
            .and_then(Value::as_str)
            .map(bounded_text)
            .or_else(|| display_name(chat))
            .unwrap_or_else(|| entry.chat_id.clone());
        entry.username = chat
            .get("username")
            .and_then(Value::as_str)
            .map(bounded_text);
        entry.last_seen_ms = now;
        if update.get("my_chat_member").is_some() {
            entry.bot_membership = message.get("new_chat_member").cloned();
        }
        if message.get("message_id").is_some() {
            entry.messages_seen = entry.messages_seen.saturating_add(1);
            entry.last_text = message_text(message)
                .unwrap_or_default()
                .chars()
                .take(500)
                .collect();
            entry.last_sender_id = message
                .get("from")
                .and_then(|v| v.get("id"))
                .and_then(Value::as_i64)
                .map(|id| id.to_string());
            entry.last_sender_name = message.get("from").and_then(display_name);
            if let Some(thread) = message.get("message_thread_id").and_then(Value::as_i64)
                && (entry.topics.len() < 100 || entry.topics.contains_key(&thread.to_string()))
            {
                let key = thread.to_string();
                let name = message
                    .get("forum_topic_created")
                    .or_else(|| message.get("forum_topic_edited"))
                    .and_then(|v| v.get("name"))
                    .and_then(Value::as_str)
                    .map(bounded_text)
                    .or_else(|| entry.topics.get(&key).cloned())
                    .unwrap_or_else(|| key.clone());
                entry.topics.insert(key, name);
            }
        }
    }
}

fn stream_key(bot: &ResolvedBot) -> String {
    // Names and credentials can rotate; bot identity and API origin cannot.
    format!(
        "inbox-{}",
        hex::encode(Sha256::digest(
            format!(
                "{}\0{}\0{}",
                bot.id,
                bot.connection.account.to_ascii_lowercase(),
                bot.connection.api_base
            )
            .as_bytes()
        ))
    )
}
fn invalid(message: impl Into<String>) -> PluginError {
    PluginError::invalid_params(message.into())
}
fn result(summary: &str, output: Value) -> CommandResult {
    let mut result = CommandResult::succeeded(summary);
    result.output = Some(output);
    result
}

impl TelegramPlugin {
    pub(super) fn host(&self) -> Result<&Arc<dyn HostClient>> {
        self.host
            .get()
            .ok_or_else(|| PluginError::internal("Telegram host storage is not initialized"))
    }
    async fn read_record<T: DeserializeOwned + Default>(&self, key: &str) -> Result<T> {
        let response = self
            .host()?
            .storage_get(HostStorageGetRequest {
                scope: HostStorageScope::Global,
                visibility: HostStorageVisibility::Private,
                namespace: NAMESPACE.into(),
                key: key.into(),
            })
            .await?;
        response.value.map_or_else(
            || Ok(T::default()),
            |value| {
                serde_json::from_str(&value).map_err(|_| {
                    PluginError::internal(
                        "Telegram state could not be decoded; existing records were preserved",
                    )
                })
            },
        )
    }
    async fn write_record<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.host()?
            .storage_set(HostStorageSetRequest {
                scope: HostStorageScope::Global,
                visibility: HostStorageVisibility::Private,
                namespace: NAMESPACE.into(),
                key: key.into(),
                value: serde_json::to_string(value).map_err(|e| PluginError::internal_error(&e))?,
            })
            .await
    }
    pub(super) fn account_lock(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.account_locks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id.into())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
    fn legacy_bot(&self) -> Result<ResolvedBot> {
        Ok(ResolvedBot {
            id: "default".into(),
            enabled: true,
            connection: self
                .settings
                .get()
                .cloned()
                .ok_or_else(|| PluginError::internal("Telegram plugin was not initialized"))?,
        })
    }
    pub(super) async fn resolve_bot(
        &self,
        selection: &BotSelection,
        identity: Option<&ChannelIdentity>,
    ) -> Result<ResolvedBot> {
        // Direct unit fixtures intentionally lack a host. They still validate
        // credentials before attempting transport. Runtime instances always
        // have a host through init.
        let registry: Registry = if self.host.get().is_some() {
            self.read_record("bots").await?
        } else {
            Registry::default()
        };
        let mut bots = registry
            .bots
            .values()
            .map(|entry| {
                let mut connection = entry.config.connection.clone();
                connection.credential_secret = entry.secret_name.clone();
                ResolvedBot {
                    id: entry.config.id.clone(),
                    enabled: entry.config.enabled,
                    connection,
                }
            })
            .collect::<Vec<_>>();
        if !registry.bots.contains_key("default") && !registry.retired.contains("default") {
            bots.push(self.legacy_bot()?);
        }
        let selected = if let Some(id) = selection.bot_id.as_ref() {
            bots.into_iter().find(|bot| &bot.id == id)
        } else if let Some(identity) = identity {
            bots.into_iter().find(|bot| {
                identity.tenant == TENANT
                    && bot
                        .connection
                        .account
                        .eq_ignore_ascii_case(&identity.account)
            })
        } else {
            bots.into_iter().find(|bot| bot.id == "default")
        };
        let bot = selected.ok_or_else(|| {
            invalid("Telegram bot was removed or its account does not match this binding")
        })?;
        bot.connection.validate()?;
        if let Some(identity) = identity
            && (identity.tenant != TENANT
                || !bot
                    .connection
                    .account
                    .eq_ignore_ascii_case(&identity.account))
        {
            return Err(invalid(
                "Telegram binding identity does not match the selected bot",
            ));
        }
        Ok(bot)
    }
    pub(super) async fn credential(&self, config: &TelegramConfig) -> Result<String> {
        if let Some(name) = config.credential_secret.as_ref() {
            return self.host()?.secret_get(HostSecretGetRequest { name: name.clone() }).await?.value
                .filter(|value| !value.is_empty()).ok_or_else(|| invalid("The saved Telegram credential is unavailable; replace it in the bot settings"));
        }
        config.token()
    }
    pub(super) async fn bot_catalog(&self) -> Result<CommandResult> {
        let registry: Registry = self.read_record("bots").await?;
        let mut bots = Vec::new();
        for bot in registry.bots.values() {
            let resolved = ResolvedBot {
                id: bot.config.id.clone(),
                enabled: bot.config.enabled,
                connection: bot.config.connection.clone(),
            };
            let stream: Stream = self.read_record(&stream_key(&resolved)).await?;
            bots.push(json!({ "id": bot.config.id, "name": bot.config.name, "enabled": bot.config.enabled,
                "connection": bot.config.connection, "credential_kind": bot.config.credential_kind,
                "has_token": bot.secret_name.is_some() || bot.config.connection.token().is_ok(),
                "managed": true, "updated_at_ms": bot.updated_at_ms,
                "verification": stream.verification, "last_poll_ms": stream.last_poll_ms,
                "last_error": stream.last_error, "room_count": stream.rooms.len(), "buffered_updates": stream.updates.len() }));
        }
        let legacy = self.legacy_bot()?;
        if !registry.bots.contains_key("default")
            && !registry.retired.contains("default")
            && (!legacy.connection.account.is_empty() || registry.bots.is_empty())
        {
            let stream: Stream = self.read_record(&stream_key(&legacy)).await?;
            bots.insert(0, json!({ "id":"default", "name":"Default bot", "enabled":true,
                "connection":legacy.connection, "credential_kind":"environment", "has_token":legacy.connection.token().is_ok(),
                "managed":false, "updated_at_ms":0, "verification":stream.verification,
                "last_poll_ms":stream.last_poll_ms, "last_error":stream.last_error,
                "room_count":stream.rooms.len(), "buffered_updates":stream.updates.len() }));
        }
        Ok(result(
            "Telegram bots loaded",
            json!({ "revision":registry.revision, "bots":bots, "max_bots":MAX_BOTS }),
        ))
    }
    pub(super) async fn save_managed_bot(
        &self,
        expected_revision: u64,
        mut bot: BotConfiguration,
        token: Option<String>,
    ) -> Result<CommandResult> {
        if bot.id.is_empty()
            || bot.id.len() > 64
            || !bot
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        {
            return Err(invalid(
                "Bot ID must contain 1–64 letters, digits, underscores or hyphens",
            ));
        }
        bot.name = bot.name.trim().into();
        if bot.name.is_empty() || bot.name.chars().count() > 80 {
            return Err(invalid("Bot display name must contain 1–80 characters"));
        }
        bot.connection.account = bot
            .connection
            .account
            .trim()
            .trim_start_matches('@')
            .to_ascii_lowercase();
        bot.connection.api_base = bot.connection.api_base.trim().trim_end_matches('/').into();
        bot.connection.bot_token_env = bot.connection.bot_token_env.trim().into();
        bot.connection.validate()?;
        if !["vault", "environment"].contains(&bot.credential_kind.as_str()) {
            return Err(invalid("Unknown Telegram credential source"));
        }
        let gate = self.account_lock(&bot.id);
        let _account = gate.lock().await;
        let _transaction = self.registry_lock.lock().await;
        let mut registry: Registry = self.read_record("bots").await?;
        if registry.revision != expected_revision {
            return Err(invalid(
                "Telegram settings changed in another window; refresh before saving",
            ));
        }
        if registry.retired.contains(&bot.id) {
            return Err(invalid(
                "A removed bot identity cannot be reused; create a new bot",
            ));
        }
        let legacy = self.legacy_bot()?;
        let legacy_remains = bot.id != "default"
            && !registry.bots.contains_key("default")
            && !registry.retired.contains("default")
            && !legacy.connection.account.is_empty();
        let projected = registry.bots.len()
            + usize::from(!registry.bots.contains_key(&bot.id))
            + usize::from(legacy_remains);
        if projected > MAX_BOTS {
            return Err(invalid(
                "At most 32 Telegram bots are supported, including the configured default bot",
            ));
        }
        if registry.bots.values().any(|entry| {
            entry.config.id != bot.id
                && entry
                    .config
                    .connection
                    .account
                    .eq_ignore_ascii_case(&bot.connection.account)
        }) {
            return Err(invalid(
                "This Telegram username is already configured; edit the existing bot instead",
            ));
        }
        if bot.id != "default"
            && !registry.bots.contains_key("default")
            && !registry.retired.contains("default")
            && legacy
                .connection
                .account
                .eq_ignore_ascii_case(&bot.connection.account)
        {
            return Err(invalid(
                "This Telegram username already belongs to the default bot",
            ));
        }
        if let Some(old) = registry.bots.get(&bot.id)
            && (old.config.connection.account != bot.connection.account
                || old.config.connection.api_base != bot.connection.api_base)
        {
            return Err(invalid(
                "A saved bot's account and API endpoint are immutable; add a new bot to change identity",
            ));
        }
        let old_secret = registry
            .bots
            .get(&bot.id)
            .and_then(|old| old.secret_name.clone());
        let supplied = token.filter(|value| !value.trim().is_empty());
        let secret_name = if bot.credential_kind == "vault" {
            if let Some(value) = supplied.as_ref() {
                validate_secret_token(value)?;
                let name = format!("telegram-{}-{}", bot.id, uuid::Uuid::new_v4());
                self.host()?
                    .secret_set(HostSecretSetRequest {
                        name: name.clone(),
                        value: value.trim().into(),
                    })
                    .await?;
                Some(name)
            } else {
                old_secret
                    .clone()
                    .ok_or_else(|| invalid("Enter a BotFather token for this new vault credential"))
                    .map(Some)?
            }
        } else {
            None
        };
        registry.revision = registry
            .revision
            .checked_add(1)
            .ok_or_else(|| PluginError::internal("Telegram registry revision overflow"))?;
        registry.bots.insert(
            bot.id.clone(),
            StoredBot {
                config: bot.clone(),
                secret_name: secret_name.clone(),
                updated_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        );
        if let Err(error) = self.write_record("bots", &registry).await {
            if secret_name != old_secret
                && let Some(name) = secret_name
            {
                let _ = self
                    .host()?
                    .secret_delete(HostSecretDeleteRequest { name })
                    .await;
            }
            return Err(error);
        }
        if old_secret != secret_name
            && let Some(name) = old_secret
        {
            let _ = self
                .host()?
                .secret_delete(HostSecretDeleteRequest { name })
                .await;
        }
        // Token changes invalidate cached verification, never return secrets.
        {
            let resolved = ResolvedBot {
                id: bot.id.clone(),
                enabled: bot.enabled,
                connection: bot.connection.clone(),
            };
            let mut stream: Stream = self.read_record(&stream_key(&resolved)).await?;
            stream.verification = None;
            self.write_record(&stream_key(&resolved), &stream).await?;
        }
        Ok(result(
            "Telegram bot saved",
            json!({"revision":registry.revision,"bot_id":bot.id}),
        ))
    }
    pub(super) async fn retire_bot(
        &self,
        expected_revision: u64,
        id: String,
    ) -> Result<CommandResult> {
        let gate = self.account_lock(&id);
        let _account = gate.lock().await;
        let _transaction = self.registry_lock.lock().await;
        let mut registry: Registry = self.read_record("bots").await?;
        if registry.revision != expected_revision {
            return Err(invalid(
                "Telegram settings changed; refresh before removing a bot",
            ));
        }
        let old = registry
            .bots
            .get(&id)
            .ok_or_else(|| invalid("Save and disable this bot before removing it"))?;
        if old.config.enabled {
            return Err(invalid(
                "Disable this bot before removing it; existing bindings will remain stopped",
            ));
        }
        let secret = old.secret_name.clone();
        registry.bots.remove(&id);
        registry.retired.insert(id.clone());
        registry.revision += 1;
        self.write_record("bots", &registry).await?;
        if let Some(name) = secret {
            let _ = self
                .host()?
                .secret_delete(HostSecretDeleteRequest { name })
                .await;
        }
        Ok(result(
            "Telegram bot removed; existing bindings cannot select a replacement",
            json!({"revision":registry.revision}),
        ))
    }
    pub(super) async fn inspect_bot(
        &self,
        input: BotSelection,
        synchronize: bool,
    ) -> Result<CommandResult> {
        let resolved = self.resolve_bot(&input, None).await?;
        let gate = self.account_lock(&resolved.id);
        let _account = gate.lock().await;
        let resolved = self.resolve_bot(&input, None).await?;
        let key = stream_key(&resolved);
        let mut stream: Stream = self.read_record(&key).await?;
        if synchronize {
            if !resolved.enabled {
                return Err(invalid("Enable this bot before synchronizing its inbox"));
            }
            self.bot_identity(&resolved.connection, false).await?;
            self.collect_updates(&resolved, &mut stream, true).await?;
        }
        let recent = stream.updates.iter().rev().take(100).map(|item| {
            let message = item.value.get("message").or_else(|| item.value.get("channel_post"));
            json!({"sequence":item.sequence,"received_at_ms":item.received_at_ms,
                "chat_id":message.and_then(|m|m.pointer("/chat/id")).and_then(Value::as_i64).map(|id|id.to_string()),
                "sender_id":message.and_then(|m|m.pointer("/from/id")).and_then(Value::as_i64).map(|id|id.to_string()),
                "text":message.and_then(message_text).unwrap_or_default().chars().take(2000).collect::<String>(),
                "update_id":item.update_id})
        }).collect::<Vec<_>>();
        Ok(result(
            "Telegram inbox loaded",
            json!({"bot_id":resolved.id,"rooms":stream.rooms.values().collect::<Vec<_>>(),
            "recent":recent,"buffered_updates":stream.updates.len(),"last_poll_ms":stream.last_poll_ms,
            "retry_at_ms":stream.retry_at_ms,"last_error":stream.last_error,"verification":stream.verification,
            "retention_days":7,"room_limit":MAX_ROOMS,"stream_id":stream.id}),
        ))
    }
    pub(super) async fn verify_bot(&self, input: BotSelection) -> Result<CommandResult> {
        let resolved = self.resolve_bot(&input, None).await?;
        let gate = self.account_lock(&resolved.id);
        let _account = gate.lock().await;
        let resolved = self.resolve_bot(&input, None).await?;
        // Keep both read-only probes within one transport timeout, rather than
        // spending two full timeout windows inside one host command lease.
        let (identity, webhook) = tokio::join!(
            self.bot_identity(&resolved.connection, true),
            self.call(&resolved.connection, "getWebhookInfo", json!({})),
        );
        let bot = match identity {
            Ok(bot) => bot,
            Err(error) => {
                let key = stream_key(&resolved);
                let mut stream: Stream = self.read_record(&key).await?;
                stream.verification = None;
                stream.last_error = Some(error.to_string());
                self.write_record(&key, &stream).await?;
                return Err(error);
            }
        };
        let (webhook_active, pending, warning) = match webhook {
            Ok(value) => (
                value
                    .get("url")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty()),
                value.get("pending_update_count").and_then(Value::as_i64),
                None,
            ),
            Err(failure) => (
                false,
                None,
                Some(failure.into_error("getWebhookInfo").to_string()),
            ),
        };
        let report = json!({"bot_id":bot.id.to_string(),"username":bot.username,"checked_at_ms":chrono::Utc::now().timestamp_millis(),"webhook_active":webhook_active,"pending_updates":pending,"webhook_warning":warning});
        let key = stream_key(&resolved);
        let mut stream: Stream = self.read_record(&key).await?;
        stream.verification = Some(report.clone());
        stream.last_error = None;
        self.write_record(&key, &stream).await?;
        let mut output = CommandResult::succeeded(format!(
            "Connected to @{} (Telegram bot ID {})",
            bot.username, bot.id
        ));
        output.output = Some(report);
        Ok(output)
    }
    pub(super) async fn inspect_room(
        &self,
        input: BotSelection,
        chat_id: String,
    ) -> Result<CommandResult> {
        if chat_id.len() > 80
            || !(chat_id.parse::<i64>().is_ok()
                || chat_id.strip_prefix('@').is_some_and(is_bot_username))
        {
            return Err(invalid("Enter a numeric chat ID or public @username"));
        }
        let resolved = self.resolve_bot(&input, None).await?;
        let gate = self.account_lock(&resolved.id);
        let _account = gate.lock().await;
        let resolved = self.resolve_bot(&input, None).await?;
        let bot = self.bot_identity(&resolved.connection, false).await?;
        let chat = self
            .call(&resolved.connection, "getChat", json!({"chat_id":chat_id}))
            .await
            .map_err(|f| f.into_error("getChat"))?;
        let id = chat
            .get("id")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid("Telegram returned no chat ID"))?
            .to_string();
        let membership = tokio::time::timeout(
            Duration::from_secs(1),
            self.call(
                &resolved.connection,
                "getChatMember",
                json!({"chat_id":id,"user_id":bot.id}),
            ),
        )
        .await
        .ok()
        .and_then(std::result::Result::ok);
        let key = stream_key(&resolved);
        let mut stream: Stream = self.read_record(&key).await?;
        if !stream.rooms.contains_key(&id) && stream.rooms.len() >= MAX_ROOMS {
            return Err(invalid("Telegram discovered-chat limit reached"));
        }
        stream.observe(
            &json!({"my_chat_member":{"chat":chat,"new_chat_member":membership}}),
            chrono::Utc::now().timestamp_millis(),
        );
        if let Some(room) = stream.rooms.get_mut(&id) {
            room.description = chat
                .get("description")
                .and_then(Value::as_str)
                .map(bounded_text);
        }
        self.write_record(&key, &stream).await?;
        Ok(result(
            "Telegram chat inspected",
            json!({"bot_id":resolved.id,"room":stream.rooms.get(&id)}),
        ))
    }
    async fn collect_updates(
        &self,
        bot: &ResolvedBot,
        stream: &mut Stream,
        immediate: bool,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        if now < stream.retry_at_ms
            || (!immediate && now.saturating_sub(stream.last_poll_ms) < 1000)
        {
            return Ok(());
        }
        stream.prune(now);
        let available = MAX_UPDATES.saturating_sub(stream.updates.len()).min(100);
        if available == 0 {
            return Err(PluginError::internal(
                "Telegram inbox is full; a lagging binding must catch up before new updates are confirmed",
            ));
        }
        let mut body = json!({"limit":available,"allowed_updates":["message","channel_post","chat_member","my_chat_member"],"timeout":if immediate {0} else {bot.connection.long_poll_seconds}});
        if let Some(offset) = stream.offset {
            body["offset"] = json!(offset);
        }
        match self.call(&bot.connection, "getUpdates", body).await {
            Ok(value) => {
                let rows = value.as_array().ok_or_else(|| {
                    PluginError::internal("Telegram getUpdates returned an invalid batch")
                })?;
                stream.append(rows, now)?;
                stream.last_error = None;
                stream.retry_at_ms = 0;
                stream.last_poll_ms = now;
                // Persist BEFORE the next network request confirms this offset.
                self.write_record(&stream_key(bot), stream).await?;
            }
            Err(failure) => {
                let transient = failure.transient();
                let backoff = i64::from(failure.retry_after.unwrap_or(30));
                let message = failure.into_error("getUpdates").to_string();
                stream.last_error = Some(message.clone());
                stream.retry_at_ms = now.saturating_add(backoff.saturating_mul(1000));
                stream.last_poll_ms = now;
                self.write_record(&stream_key(bot), stream).await?;
                if !transient {
                    return Err(PluginError::internal(message));
                }
            }
        }
        Ok(())
    }
    pub(super) async fn poll_shared(&self, request: &AdapterPollRequest) -> Result<AdapterBatch> {
        let input: BotSelection = serde_json::from_value(request.config.clone())
            .map_err(|_| invalid("Telegram binding config may only select bot_id"))?;
        let resolved = self
            .resolve_bot(&input, request.channel_identity.as_ref())
            .await?;
        if !resolved.enabled {
            return Err(invalid("This Telegram bot is disabled"));
        }
        // Validate the credential first; it also preserves the explicit error
        // for uninitialized test fixtures rather than hiding it behind storage.
        self.credential(&resolved.connection).await?;
        let gate = self.account_lock(&resolved.id);
        let _account = gate.lock().await;
        let resolved = self
            .resolve_bot(&input, request.channel_identity.as_ref())
            .await?;
        if !resolved.enabled {
            return Err(invalid("This Telegram bot is disabled"));
        }
        let bot = self.bot_identity(&resolved.connection, false).await?;
        let key = stream_key(&resolved);
        let mut stream: Stream = self.read_record(&key).await?;
        if let Some(id) = request.cursor.get("stream_id").and_then(Value::as_str)
            && id != stream.id
        {
            return Err(invalid(
                "Telegram inbox identity changed; review this binding before resuming",
            ));
        }
        if request.cursor.get("sequence").is_none()
            && stream.floor > 0
            && request.not_before_ms < stream.floor_received_ms
        {
            return Err(invalid(
                "This binding predates retained Telegram inbox history; review it before resuming",
            ));
        }
        let cursor = request
            .cursor
            .get("sequence")
            .and_then(Value::as_u64)
            .unwrap_or(stream.floor);
        if cursor < stream.floor || cursor >= stream.next_sequence {
            return Err(invalid(
                "Telegram binding cursor is outside retained history; review and recreate the binding instead of silently skipping messages",
            ));
        }
        stream.consumers.insert(
            request.binding_id.clone(),
            Consumer {
                acknowledged: cursor,
                seen_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        );
        // Drain this binding's retained data before acquiring new upstream data.
        // A busy account cannot starve slower consumers or fill its own queue.
        if !stream.updates.iter().any(|row| row.sequence > cursor) {
            self.collect_updates(&resolved, &mut stream, false).await?;
        }
        let legacy_offset = request.cursor.get("offset").and_then(Value::as_i64);
        let mut next = cursor;
        let mut events = Vec::new();
        for row in stream.updates.iter().filter(|row| row.sequence > cursor) {
            next = row.sequence;
            if legacy_offset.is_some_and(|offset| row.update_id < offset) {
                continue;
            }
            if let Some(event) =
                self.event_from_update(&row.value, &resolved.connection, &bot, request)
            {
                events.push(event);
            }
            if events.len() >= request.limit.clamp(1, 100) as usize {
                break;
            }
        }
        self.write_record(&key, &stream).await?;
        Ok(AdapterBatch {
            events,
            cursor: json!({"stream_id":stream.id,"sequence":next}),
            retry_after_seconds: if stream.retry_at_ms > chrono::Utc::now().timestamp_millis() {
                Some(30)
            } else {
                None
            },
        })
    }
}
fn validate_secret_token(token: &str) -> Result<()> {
    if token.len() > 256
        || !token
            .trim()
            .split_once(':')
            .is_some_and(|(id, secret)| id.parse::<i64>().is_ok() && !secret.is_empty())
        || token
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '?' | '#'))
    {
        return Err(invalid(
            "Enter the BotFather token, in bot-ID:secret format",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_buffer_is_independent_for_two_chat_bindings() {
        let mut state = Stream::default();
        state.append(&[json!({"update_id":1,"message":{"message_id":1,"date":1,"chat":{"id":11,"type":"private"},"from":{"id":11},"text":"one"}}), json!({"update_id":2,"message":{"message_id":1,"date":1,"chat":{"id":22,"type":"private"},"from":{"id":22},"text":"two"}})],1000).unwrap();
        assert_eq!(state.offset, Some(3));
        assert_eq!(
            state.updates.iter().filter(|row| row.sequence > 0).count(),
            2
        );
        // Reading A does not remove either entry before B commits its cursor.
        state.consumers.insert(
            "a".into(),
            Consumer {
                acknowledged: 2,
                seen_at_ms: 1000,
            },
        );
        state.consumers.insert(
            "b".into(),
            Consumer {
                acknowledged: 0,
                seen_at_ms: 1000,
            },
        );
        state.prune(1001);
        assert_eq!(state.updates.len(), 2);
        assert_eq!(state.rooms.len(), 2);
        let restored: Stream =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(restored.id, state.id);
        assert_eq!(restored.offset, Some(3));
    }
    #[test]
    fn backlog_expiry_advances_an_explicit_floor() {
        let mut state = Stream::default();
        state.append(&[json!({"update_id":5})], 1).unwrap();
        state.prune(RETENTION_MS + 2);
        assert!(state.updates.is_empty());
        assert_eq!(state.floor, 1);
    }
    #[test]
    fn credentials_and_ids_are_never_confused() {
        assert!(validate_secret_token("123:abc").is_ok());
        for token in ["BOT_TOKEN", "123:abc/steal", "123:abc?x", "123:abc\n"] {
            assert!(validate_secret_token(token).is_err());
        }
    }
}

#[cfg(test)]
mod integration_tests;
