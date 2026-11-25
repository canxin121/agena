use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use agena_plugin_sdk::{
    PluginError, Result,
    integration::{
        AdapterBatch, AdapterPollRequest, ChannelDelivery, ChannelReceipt, ConversationKind,
        SendDisposition,
    },
};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    ChatPlatform,
    config::{Connection, Discord, DiscordRoom, Feishu, decimal_id, secret},
    receipt,
    webhooks::{Message, field, to_event},
};

const SLACK: &str = "https://slack.com";
const DISCORD: &str = "https://discord.com/api/v10";
const GRAPH: &str = "https://graph.facebook.com/v23.0";
const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;
const CREDENTIAL_TTL: Duration = Duration::from_secs(300);
const API_BODY_LIMIT: usize = 2 * 1024 * 1024;
const NO_MENTIONS: Value = Value::Array(Vec::new());
const MAX_REPLY_UNITS: usize = 1800;
const MAX_REPLY_CHUNKS: usize = 25;

pub(super) struct PlatformApi {
    client: reqwest::Client,
    credentials: tokio::sync::Mutex<BTreeMap<String, CachedCredential>>,
    #[cfg(test)]
    api_override: Option<String>,
}

struct CachedCredential {
    fingerprint: [u8; 32],
    expires: Instant,
    credential: String,
}

impl PlatformApi {
    pub(super) fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(4))
            .build()
            .map_err(|_| PluginError::internal("could not initialize chat adapter HTTP client"))?;
        Ok(Self {
            client,
            credentials: Default::default(),
            #[cfg(test)]
            api_override: None,
        })
    }

    #[cfg(test)]
    pub(super) fn with_endpoint(mut self, base: String) -> Self {
        self.api_override = Some(base);
        self
    }

    fn request(&self, method: Method, base: &str, path: &str) -> RequestBuilder {
        #[cfg(test)]
        let base = self.api_override.as_deref().unwrap_or(base);
        self.client.request(method, format!("{base}{path}"))
    }

    pub(super) async fn authenticate(&self, account: &Connection) -> Result<String> {
        let credential = secret(account.token_env())?;
        let fingerprint: [u8; 32] = Sha256::digest(credential.as_bytes()).into();
        let cached = self.credentials.lock().await;
        if let Some(previous) = cached.get(account.name())
            && previous.fingerprint == fingerprint
            && previous.expires > Instant::now()
        {
            return Ok(previous.credential.clone());
        }
        drop(cached);
        let (tenant, expected) = account.identity_parts();
        let (credential, ttl) = match account {
            Connection::Slack(_) => {
                let result = request_json(
                    self.request(Method::POST, SLACK, "/api/auth.test")
                        .bearer_auth(&credential),
                    ChatPlatform::Slack,
                )
                .await
                .map_err(|_| auth_failed("Slack"))?;
                if field(&result, "team_id") != tenant
                    || field(&result, "user_id") != expected
                    || !field(&result, "bot_id").starts_with('B')
                {
                    return Err(identity_mismatch("Slack"));
                }
                (credential, CREDENTIAL_TTL)
            }
            Connection::Feishu(config) => self.authenticate_feishu(config, &credential).await?,
            Connection::Discord(_) => {
                let result = request_json(
                    self.request(Method::GET, DISCORD, "/users/@me")
                        .header(reqwest::header::AUTHORIZATION, format!("Bot {credential}")),
                    ChatPlatform::Discord,
                )
                .await
                .map_err(|_| auth_failed("Discord"))?;
                if field(&result, "id") != expected || result["bot"].as_bool() != Some(true) {
                    return Err(identity_mismatch("Discord"));
                }
                (credential, CREDENTIAL_TTL)
            }
            Connection::WhatsApp(_) => {
                let result = request_json(
                    self.request(Method::GET, GRAPH, &format!("/{expected}"))
                        .query(&[("fields", "id")])
                        .bearer_auth(&credential),
                    ChatPlatform::WhatsApp,
                )
                .await
                .map_err(|_| auth_failed("WhatsApp"))?;
                if field(&result, "id") != expected {
                    return Err(identity_mismatch("WhatsApp"));
                }
                (credential, CREDENTIAL_TTL)
            }
        };
        self.credentials.lock().await.insert(
            account.name().into(),
            CachedCredential {
                fingerprint,
                expires: Instant::now() + ttl,
                credential: credential.clone(),
            },
        );
        Ok(credential)
    }

    async fn authenticate_feishu(
        &self,
        account: &Feishu,
        secret: &str,
    ) -> Result<(String, Duration)> {
        let result = request_json(
            self.request(
                Method::POST,
                account.host(),
                "/open-apis/auth/v3/tenant_access_token/internal",
            )
            .json(&json!({"app_id": account.app_id, "app_secret": secret})),
            ChatPlatform::Feishu,
        )
        .await
        .map_err(|_| auth_failed("Feishu"))?;
        let token = field(&result, "tenant_access_token");
        if token.is_empty() {
            return Err(auth_failed("Feishu"));
        }
        let bot = request_json(
            self.request(Method::GET, account.host(), "/open-apis/bot/v3/info")
                .bearer_auth(token),
            ChatPlatform::Feishu,
        )
        .await
        .map_err(|_| auth_failed("Feishu"))?;
        if field(&bot["bot"], "open_id") != account.bot_open_id {
            return Err(identity_mismatch("Feishu"));
        }
        let ttl = result["expire"]
            .as_u64()
            .unwrap_or(300)
            .saturating_sub(60)
            .max(1);
        Ok((token.into(), Duration::from_secs(ttl).min(CREDENTIAL_TTL)))
    }

    pub(super) async fn deliver(
        &self,
        account: &Connection,
        request: &ChannelDelivery,
    ) -> Result<ChannelReceipt> {
        let chunks = reply_chunks(&request.text);
        if chunks.len() > MAX_REPLY_CHUNKS {
            return Ok(receipt(SendDisposition::Rejected, None, None));
        }
        let credential = match self.authenticate(account).await {
            Ok(credential) => credential,
            Err(error) if error.kind == agena_plugin_sdk::PluginErrorKind::InvalidParams => {
                return Ok(receipt(SendDisposition::Rejected, None, None));
            }
            Err(_) => return Ok(receipt(SendDisposition::Retryable, None, None)),
        };
        if let Connection::Feishu(config) = account
            && let Some(thread) = &request.thread
        {
            // Feishu's reply endpoint accepts only a message ID. Confirm
            // that the thread root belongs to the delivery's authorized chat.
            let info = request_json(
                self.request(
                    Method::GET,
                    config.host(),
                    &format!("/open-apis/im/v1/messages/{thread}"),
                )
                .bearer_auth(&credential),
                ChatPlatform::Feishu,
            )
            .await;
            match info {
                Ok(info) if field(&info["data"]["items"][0], "chat_id") == request.resource => {}
                Ok(_) => return Ok(receipt(SendDisposition::Rejected, None, None)),
                Err(error) if error.disposition == SendDisposition::Rejected => {
                    return Ok(receipt(SendDisposition::Rejected, None, None));
                }
                Err(error) => {
                    return Ok(receipt(SendDisposition::Retryable, None, error.retry_after));
                }
            }
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
        let mut remote_id = None;
        for (index, chunk) in chunks.into_iter().enumerate() {
            let result = tokio::time::timeout_at(
                deadline,
                self.send_chunk(account, &credential, request, chunk),
            )
            .await;
            match result {
                Ok(Ok(id)) => {
                    if remote_id.is_none() {
                        remote_id = Some(id);
                    }
                }
                Ok(Err(error)) if index == 0 => {
                    return Ok(receipt(error.disposition, None, error.retry_after));
                }
                _ => return Ok(receipt(SendDisposition::Uncertain, remote_id, None)),
            }
        }
        Ok(receipt(SendDisposition::Sent, remote_id, None))
    }

    async fn send_chunk(
        &self,
        account: &Connection,
        token: &str,
        delivery: &ChannelDelivery,
        text: &str,
    ) -> std::result::Result<String, ApiFailure> {
        let resource = &delivery.resource;
        let (request, platform) = match account {
            Connection::Slack(_) => {
                let mut message = json!({"channel": resource, "text": text, "mrkdwn": false, "unfurl_links": false, "unfurl_media": false});
                if let Some(thread) = &delivery.thread {
                    message["thread_ts"] = json!(thread);
                }
                (
                    self.request(Method::POST, SLACK, "/api/chat.postMessage")
                        .bearer_auth(token)
                        .json(&message),
                    ChatPlatform::Slack,
                )
            }
            Connection::Feishu(config) => {
                let content = json!({"text": text}).to_string();
                let (path, message) = if let Some(thread) = &delivery.thread {
                    (
                        format!("/open-apis/im/v1/messages/{thread}/reply"),
                        json!({"msg_type": "text", "content": content, "reply_in_thread": true}),
                    )
                } else {
                    (
                        "/open-apis/im/v1/messages?receive_id_type=chat_id".into(),
                        json!({"receive_id": resource, "msg_type": "text", "content": content}),
                    )
                };
                (
                    self.request(Method::POST, config.host(), &path)
                        .bearer_auth(token)
                        .json(&message),
                    ChatPlatform::Feishu,
                )
            }
            Connection::Discord(_) => (
                self.request(
                    Method::POST,
                    DISCORD,
                    &format!("/channels/{resource}/messages"),
                )
                .header(reqwest::header::AUTHORIZATION, format!("Bot {token}"))
                .json(&json!({"content": text, "allowed_mentions": {"parse": NO_MENTIONS}})),
                ChatPlatform::Discord,
            ),
            Connection::WhatsApp(config) => (
                self.request(
                    Method::POST,
                    GRAPH,
                    &format!("/{}/messages", config.phone_number_id),
                )
                .bearer_auth(token)
                .json(&json!({
                    "messaging_product": "whatsapp", "to": resource, "type": "text",
                    "text": {"body": text, "preview_url": false}
                })),
                ChatPlatform::WhatsApp,
            ),
        };
        let result = request_json(request, platform).await?;
        let id = match platform {
            ChatPlatform::Slack => field(&result, "ts"),
            ChatPlatform::Feishu => field(&result["data"], "message_id"),
            ChatPlatform::Discord => field(&result, "id"),
            ChatPlatform::WhatsApp => field(&result["messages"][0], "id"),
        };
        if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
            return Err(ApiFailure::uncertain());
        }
        Ok(id.into())
    }

    pub(super) async fn poll_discord(
        &self,
        account: &Connection,
        config: &Discord,
        request: &AdapterPollRequest,
    ) -> Result<AdapterBatch> {
        let mut cursor: DiscordCursor = if request.cursor.is_null() {
            DiscordCursor::default()
        } else {
            super::decode(request.cursor.clone())?
        };
        let rooms: Vec<_> = config
            .rooms
            .iter()
            .filter(|room| {
                request.channel_kinds.is_empty() || request.channel_kinds.contains(&room.kind)
            })
            .collect();
        if rooms.is_empty() {
            return Ok(AdapterBatch {
                events: Vec::new(),
                cursor: request.cursor.clone(),
                retry_after_seconds: None,
            });
        }
        let position = cursor.position % rooms.len();
        let room = rooms[position];
        let token = self.authenticate(account).await?;
        let channel_path = format!("/channels/{}", room.id);
        let auth = format!("Bot {token}");
        let channel = request_json(
            self.request(Method::GET, DISCORD, &channel_path)
                .header(reqwest::header::AUTHORIZATION, &auth),
            ChatPlatform::Discord,
        )
        .await;
        let channel = match channel {
            Err(error) if error.disposition == SendDisposition::Retryable => {
                return discord_backoff(request, error);
            }
            result => result.map_err(|_| {
                PluginError::internal("Discord could not inspect the configured channel")
            })?,
        };
        if field(&channel, "id") != room.id || discord_kind(&channel) != Some(room.kind) {
            return Err(PluginError::invalid_params(
                "Discord channel kind does not match its configuration",
            ));
        }
        let after = cursor
            .after
            .get(&room.id)
            .copied()
            .unwrap_or(0)
            .max(discord_start(request.not_before_ms));
        let limit = request.limit.clamp(1, 32).to_string();
        let result = request_json(
            self.request(Method::GET, DISCORD, &format!("{channel_path}/messages"))
                .header(reqwest::header::AUTHORIZATION, &auth)
                .query(&[("limit", limit), ("after", after.to_string())]),
            ChatPlatform::Discord,
        )
        .await;
        let result = match result {
            Err(error) if error.disposition == SendDisposition::Retryable => {
                return discord_backoff(request, error);
            }
            result => result.map_err(|_| {
                PluginError::internal("Discord could not read configured channel messages")
            })?,
        };
        let list = result
            .as_array()
            .ok_or_else(|| PluginError::internal("Discord returned an invalid message list"))?;
        let mut messages: Vec<_> = list
            .iter()
            .filter(|message| field(message, "channel_id") == room.id)
            .filter_map(|message| Some((field(message, "id").parse::<u64>().ok()?, message)))
            .filter(|(id, _)| *id > after)
            .collect();
        messages.sort_unstable_by_key(|(id, _)| *id);
        let identity = account.identity();
        let mut latest = after;
        let mut events = Vec::new();
        for (id, message) in messages.into_iter().take(32) {
            latest = id;
            if let Some(event) = discord_event(config, room, &identity, id, message, request.now_ms)
                && event.occurred_at_ms >= request.not_before_ms
            {
                events.push(event);
            }
        }
        cursor
            .after
            .retain(|id, _| config.rooms.iter().any(|room| room.id == *id));
        cursor.after.insert(room.id.clone(), latest);
        cursor.position = (position + 1) % rooms.len();
        Ok(AdapterBatch {
            events,
            cursor: super::encode(cursor)?,
            retry_after_seconds: None,
        })
    }
}

fn auth_failed(platform: &str) -> PluginError {
    PluginError::internal(format!(
        "{platform} could not verify the configured bot credential"
    ))
}

fn identity_mismatch(platform: &str) -> PluginError {
    PluginError::invalid_params(format!(
        "{platform} credential does not belong to the configured account"
    ))
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DiscordCursor {
    position: usize,
    after: BTreeMap<String, u64>,
}

fn discord_start(not_before: i64) -> u64 {
    let ms = not_before
        .saturating_sub(DISCORD_EPOCH_MS)
        .clamp(0, (1_i64 << 42) - 1);
    ((ms as u64) << 22).saturating_sub(1)
}

fn discord_kind(channel: &Value) -> Option<ConversationKind> {
    match channel["type"].as_u64()? {
        1 => Some(ConversationKind::Direct),
        5 => Some(ConversationKind::Broadcast),
        0 | 2 | 3 | 10 | 11 | 12 | 13 => Some(ConversationKind::Group),
        _ => None,
    }
}

fn discord_event(
    account: &Discord,
    room: &DiscordRoom,
    identity: &agena_plugin_sdk::integration::ChannelIdentity,
    id: u64,
    value: &Value,
    now_ms: i64,
) -> Option<agena_plugin_sdk::integration::ExternalEvent> {
    let author = &value["author"];
    let actor = field(author, "id");
    if !matches!(value["type"].as_u64(), Some(0 | 19))
        || !decimal_id(actor)
        || actor == account.bot_user_id
        || author["bot"].as_bool() == Some(true)
        || !value["webhook_id"].is_null()
    {
        return None;
    }
    let addressed = value["mentions"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|user| field(user, "id") == account.bot_user_id)
        || field(&value["referenced_message"]["author"], "id") == account.bot_user_id;
    to_event(
        identity,
        Message {
            id: format!("discord:{id}"),
            resource: &room.id,
            actor,
            kind: room.kind,
            thread: None,
            text: field(value, "content"),
            addressed,
            occurred_at_ms: i64::try_from(id >> 22)
                .ok()?
                .checked_add(DISCORD_EPOCH_MS)?,
        },
        now_ms,
    )
}

fn discord_backoff(request: &AdapterPollRequest, failure: ApiFailure) -> Result<AdapterBatch> {
    Ok(AdapterBatch {
        events: Vec::new(),
        cursor: request.cursor.clone(),
        retry_after_seconds: failure.retry_after.or(Some(5)),
    })
}

fn reply_chunks(text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units + ch.len_utf16() > MAX_REPLY_UNITS {
            chunks.push(&text[start..index]);
            start = index;
            units = 0;
        }
        units += ch.len_utf16();
    }
    if start < text.len() {
        chunks.push(&text[start..]);
    }
    chunks
}

#[derive(Debug)]
struct ApiFailure {
    disposition: SendDisposition,
    retry_after: Option<u32>,
}

impl ApiFailure {
    fn uncertain() -> Self {
        Self {
            disposition: SendDisposition::Uncertain,
            retry_after: None,
        }
    }

    fn rejected() -> Self {
        Self {
            disposition: SendDisposition::Rejected,
            retry_after: None,
        }
    }

    fn backoff(retry_after: Option<u32>) -> Self {
        Self {
            disposition: SendDisposition::Retryable,
            retry_after: retry_after.map(|value| value.clamp(1, 3600)),
        }
    }
}

async fn request_json(
    request: RequestBuilder,
    platform: ChatPlatform,
) -> std::result::Result<Value, ApiFailure> {
    let mut response = request.send().await.map_err(|_| ApiFailure::uncertain())?;
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|header| header.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.ceil() as u32);
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Err(ApiFailure::backoff(retry_after));
    }
    if !(status.is_success() || status.is_client_error()) {
        return Err(ApiFailure::uncertain());
    }
    if status.is_client_error() {
        return Err(ApiFailure::rejected());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ApiFailure::uncertain())?
    {
        if body.len() + chunk.len() > API_BODY_LIMIT {
            return Err(ApiFailure::uncertain());
        }
        body.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice::<Value>(&body).map_err(|_| ApiFailure::uncertain())?;
    match platform {
        ChatPlatform::Slack if value["ok"].as_bool() != Some(true) => {
            match field(&value, "error") {
                "ratelimited" => Err(ApiFailure::backoff(retry_after)),
                "internal_error" | "fatal_error" | "" => Err(ApiFailure::uncertain()),
                _ => Err(ApiFailure::rejected()),
            }
        }
        ChatPlatform::Feishu if value["code"].as_i64() != Some(0) => Err(ApiFailure::rejected()),
        ChatPlatform::WhatsApp if value.get("error").is_some() => Err(ApiFailure::rejected()),
        _ => Ok(value),
    }
}
