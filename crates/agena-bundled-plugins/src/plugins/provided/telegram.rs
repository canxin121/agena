//! `agena.telegram` — a Telegram Bot API channel adapter.
//!
//! Long polling (`getUpdates`) instead of a webhook: the host already consumes
//! events by polling, and long polling needs no public HTTPS endpoint,
//! certificate or reverse proxy. Each bot owns a persisted upstream offset and
//! replay inbox; bindings keep independent local cursors, so one chat cannot
//! acknowledge another chat's messages before they have been persisted.
//!
//! This adapter reports evidence, never authority. The bot's own `@username`
//! comes from `getMe` and must match the configured `account`, so a token
//! pasted into the wrong connection cannot silently serve a different bot's
//! rooms. Receiving a message proves its sender was in that chat at that
//! moment, which is all `is_member` claims; that fact is never upgraded into an
//! Agena role. Roles, tool grants, quotas and the inbox live in the host's
//! `channel_policy` and are rechecked there for every message.
//!
//! The endpoint and token never come from message content. They are operator
//! configuration, validated before use, exactly like the reference bridge's
//! `reply_url`.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use agena_plugin_host::sdk::{CommandResult, PluginError, PluginSettings, Result};
use agena_plugin_sdk::integration::{
    AdapterBatch, AdapterDescription, AdapterKind, AdapterPollRequest, ChannelDelivery,
    ChannelEnvelope, ChannelIdentity, ChannelReceipt, ConversationKind, ExternalEvent,
    SendDisposition,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
mod management;
use management::{BotConfiguration, BotSelection};

pub(crate) const TELEGRAM_PLUGIN_ID: &str = "agena.telegram";

/// The `tenant` half of every identity this adapter reports.
///
/// Telegram scopes bots rather than accounts under an organisation, so the
/// platform is the tenant and the bot's own username is the account. The room
/// rides on each message, so one bot serving many rooms needs no extra axis.
const TENANT: &str = "telegram";

/// Telegram bounds a message at 4096 UTF-16 units. Keep a margin so a
/// reopened code fence can never push a chunk over the hard limit.
const SEND_CHUNK_UNITS: usize = 3500;

/// The host gives a non-`describe` service call 20 seconds. Answer well before
/// that, so a slow round-trip becomes an honest uncertain receipt instead of a
/// transport timeout the host has to interpret.
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// `getUpdates` holds the request open for at most this long, which must stay
/// comfortably inside [`HTTP_TIMEOUT`].
const MAX_LONG_POLL_SECONDS: u32 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
/// Settings for one Telegram bot connection.
pub struct TelegramConfig {
    /// Name of the environment variable holding the Bot API token, never the
    /// token itself: connection settings are stored in the database.
    pub bot_token_env: String,
    /// The bot's own username without `@`, as set in BotFather. Verified
    /// against `getMe` before any message is accepted.
    pub account: String,
    /// Bot API root: the public endpoint, or a self-hosted Bot API server.
    pub api_base: String,
    /// Bounded long-poll window in seconds. Zero answers immediately, which
    /// suits the host's own polling cadence.
    pub long_poll_seconds: u32,
    /// Permit plain-HTTP `api_base`. Only a loopback host qualifies.
    pub allow_loopback_http: bool,
    #[serde(skip)]
    #[schemars(skip)]
    credential_secret: Option<String>,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            bot_token_env: "AGENA_TELEGRAM_BOT_TOKEN".into(),
            account: String::new(),
            api_base: "https://api.telegram.org".into(),
            long_poll_seconds: 0,
            allow_loopback_http: false,
            credential_secret: None,
        }
    }
}

impl TelegramConfig {
    /// Validate everything that does not need the network.
    fn validate(&self) -> Result<()> {
        self.validate_transport()?;
        if !is_bot_username(&self.account) {
            return Err(PluginError::invalid_params(
                "account must be the bot's own username without '@', for example my_agent_bot",
            ));
        }
        Ok(())
    }

    fn validate_transport(&self) -> Result<()> {
        if !is_env_name(&self.bot_token_env) {
            return Err(PluginError::invalid_params(
                "bot_token_env must be an environment variable name such as AGENA_TELEGRAM_BOT_TOKEN",
            ));
        }
        if self.long_poll_seconds > MAX_LONG_POLL_SECONDS {
            return Err(PluginError::invalid_params(format!(
                "long_poll_seconds must be at most {MAX_LONG_POLL_SECONDS}"
            )));
        }
        let url = url::Url::parse(self.api_base.trim())
            .map_err(|_| PluginError::invalid_params("api_base must be an absolute URL"))?;
        if url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(PluginError::invalid_params(
                "api_base must be a bare URL without embedded credentials",
            ));
        }
        let loopback = match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
            None => false,
        };
        if !match url.scheme() {
            "https" => true,
            "http" => loopback && self.allow_loopback_http,
            _ => false,
        } {
            return Err(PluginError::invalid_params(
                "api_base must use HTTPS; plain HTTP needs a loopback host and allow_loopback_http",
            ));
        }
        Ok(())
    }

    fn token(&self) -> Result<String> {
        let token = std::env::var(&self.bot_token_env)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                PluginError::internal(format!(
                    "Telegram bot token is unavailable; set environment variable {}",
                    self.bot_token_env
                ))
            })?;
        // A token is `<bot id>:<secret>` and becomes part of the request path,
        // so refuse anything else before it can reach a URL or a log line.
        if !token
            .split_once(':')
            .is_some_and(|(id, secret)| !id.is_empty() && !secret.is_empty())
            || token
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '/' | '?' | '#'))
        {
            return Err(PluginError::invalid_params(
                "the configured Telegram bot token is malformed",
            ));
        }
        Ok(token)
    }
}

/// The bot behind one token, as Telegram reports it.
#[derive(Debug, Clone)]
struct BotIdentity {
    id: i64,
    username: String,
}

/// Presentation copy for the settings contract; the contract itself, and every
/// bound in it, still comes from [`TelegramConfig`].
static TELEGRAM_SETTINGS_METADATA: &[(&str, &str, &str)] = &[
    (
        "",
        "Telegram Plugin Config",
        "Default environment-backed connection. The Telegram manager stores multiple BotFather credentials privately and binds each connection to a local AI Bot.",
    ),
    (
        "/bot_token_env",
        "Bot Token Environment Variable",
        "Name of the server environment variable holding the Bot API token, never the token itself.",
    ),
    (
        "/account",
        "Bot Username",
        "The bot's own username without '@'. A token belonging to another bot is refused instead of serving its chats.",
    ),
    (
        "/api_base",
        "Bot API Base URL",
        "HTTPS Bot API root. Change it only for a self-hosted Bot API server.",
    ),
    (
        "/long_poll_seconds",
        "Long Poll Seconds",
        "How long getUpdates may hold the request open. Zero returns immediately.",
    ),
    (
        "/allow_loopback_http",
        "Allow Loopback HTTP",
        "Permit a plain-HTTP api_base on a loopback host, for a local test fixture only.",
    ),
];

/// Project and global hosts load distinct plugin instances, but managed bots
/// and their inboxes use the same global storage namespace. Their read/modify/
/// write transactions and upstream polling must therefore share process locks.
#[derive(Default)]
struct TelegramLocks {
    registry: Arc<tokio::sync::Mutex<()>>,
    accounts: Arc<Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

pub(crate) struct TelegramPlugin {
    client: OnceLock<reqwest::Client>,
    host: OnceLock<Arc<dyn agena_plugin_sdk::host_api::HostClient>>,
    registry_lock: Arc<tokio::sync::Mutex<()>>,
    account_locks: Arc<Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// The legacy default connection. Additional bots live in private host
    /// storage with separate credential references. Bindings select a stable
    /// bot ID and room; they can never supply an endpoint or secret.
    settings: PluginSettings<TelegramConfig>,
    /// `getMe` answers per endpoint and token name, so a poll does not spend a
    /// round-trip re-proving the same identity on every tick.
    identities: Arc<Mutex<BTreeMap<String, BotIdentity>>>,
    background: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for TelegramPlugin {
    fn drop(&mut self) {
        self.stop_background();
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "telegram",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Telegram channel adapter: direct chats, groups and channels over the official Bot API.",
    help = "Connect multiple BotFather tokens to local AI Bots. The Telegram manager discovers chats through one durable collector per token and applies private/group/channel switches plus chat allow or deny lists. Per-room tool grants remain available in advanced channel settings.",
    settings = TelegramConfig,
    settings_default = TelegramConfig::default(),
    settings_metadata = TELEGRAM_SETTINGS_METADATA,
    settings_field = settings,
    translations(
        locale("zh-CN", summary = "Telegram 渠道适配器：通过官方 Bot API 接入私聊、群聊和频道。"),
        locale("zh-TW", summary = "Telegram 渠道轉接器：透過官方 Bot API 接入私聊、群組與頻道。"),
        locale("ja-JP", summary = "Telegram チャネルアダプター：公式 Bot API で個人チャット・グループ・チャンネルを接続します。"),
        locale("ko-KR", summary = "Telegram 채널 어댑터: 공식 Bot API로 개인 채팅, 그룹, 채널을 연결합니다."),
        locale("fr-FR", summary = "Adaptateur de canal Telegram : conversations privées, groupes et canaux via l’API Bot officielle."),
        locale("de-DE", summary = "Telegram-Kanaladapter: Einzelchats, Gruppen und Kanäle über die offizielle Bot-API."),
        locale("es-ES", summary = "Adaptador de canal de Telegram: chats privados, grupos y canales mediante la API oficial de bots."),
        locale("pt-BR", summary = "Adaptador de canal do Telegram: conversas privadas, grupos e canais pela API oficial de bots."),
        locale("hi-IN", summary = "Telegram चैनल अनुकूलक: आधिकारिक Bot API से निजी चैट, समूह और चैनल जोड़ता है।"),
        locale("ar-SA", summary = "محوّل قناة Telegram: المحادثات الفردية والمجموعات والقنوات عبر واجهة البوت الرسمية.")
    )
)]
impl TelegramPlugin {
    pub(crate) fn new() -> Self {
        static LOCKS: OnceLock<TelegramLocks> = OnceLock::new();
        let locks = LOCKS.get_or_init(TelegramLocks::default);
        Self {
            client: OnceLock::new(),
            host: OnceLock::new(),
            registry_lock: locks.registry.clone(),
            account_locks: locks.accounts.clone(),
            settings: PluginSettings::new(),
            identities: Arc::new(Mutex::new(BTreeMap::new())),
            background: Mutex::new(None),
        }
    }

    /// The configured connection, refusing to run before the host has
    /// initialized it. Settings are validated on every call, so a broken
    /// endpoint fails the poll instead of silently reaching somewhere else.
    #[cfg(test)]
    fn config(&self) -> Result<TelegramConfig> {
        let config = self
            .settings
            .get()
            .cloned()
            .ok_or_else(|| PluginError::internal("Telegram plugin was not initialized"))?;
        config.validate()?;
        Ok(config)
    }

    fn client(&self) -> &reqwest::Client {
        self.client.get_or_init(|| {
            reqwest::Client::builder()
                // A redirect would carry the token in the path to another host.
                .redirect(reqwest::redirect::Policy::none())
                .timeout(HTTP_TIMEOUT)
                .build()
                .expect("Telegram HTTP client must build")
        })
    }

    #[hook(
        init,
        summary = "Initialize Telegram bot storage",
        help = "Connect private bot configuration, credential storage and the durable per-bot inbox.",
        translations(
            locale(
                "zh-CN",
                summary = "初始化 Telegram 存储",
                help = "连接机器人私有配置、凭据存储和各机器人的持久收件队列。"
            ),
            locale(
                "zh-TW",
                summary = "初始化 Telegram 儲存",
                help = "連接機器人私有設定、憑證儲存和各機器人的持久收件佇列。"
            ),
            locale(
                "fr-FR",
                summary = "Initialiser le stockage Telegram",
                help = "Connecte la configuration privée, les identifiants et la boîte de réception durable de chaque bot."
            ),
            locale(
                "es-ES",
                summary = "Inicializar el almacenamiento de Telegram",
                help = "Conecta la configuración privada, las credenciales y la bandeja persistente de cada bot."
            ),
            locale(
                "pt-BR",
                summary = "Inicializar o armazenamento do Telegram",
                help = "Conecta a configuração privada, as credenciais e a caixa persistente de cada bot."
            ),
            locale(
                "de-DE",
                summary = "Telegram-Speicher initialisieren",
                help = "Verbindet private Bot-Konfiguration, Zugangsdaten und den dauerhaften Posteingang jedes Bots."
            ),
            locale(
                "ja-JP",
                summary = "Telegram ストレージを初期化",
                help = "各ボットの非公開設定、認証情報、永続的な受信キューを接続します。"
            ),
            locale(
                "ko-KR",
                summary = "Telegram 저장소 초기화",
                help = "각 봇의 비공개 설정, 자격 증명 및 영구 수신 큐를 연결합니다."
            ),
            locale(
                "hi-IN",
                summary = "Telegram संग्रहण प्रारंभ करें",
                help = "निजी बॉट कॉन्फ़िगरेशन, क्रेडेंशियल संग्रहण और प्रत्येक बॉट की स्थायी इनबॉक्स जोड़ता है।"
            ),
            locale(
                "ar-SA",
                summary = "تهيئة تخزين Telegram",
                help = "يربط إعدادات البوت الخاصة وبيانات الاعتماد وصندوق الوارد الدائم لكل بوت."
            )
        )
    )]
    async fn init(
        &self,
        _ctx: agena_plugin_sdk::InitContext,
        host: Arc<dyn agena_plugin_sdk::host_api::HostClient>,
    ) -> Result<agena_plugin_sdk::InitOutcome> {
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("Telegram storage already initialized"))?;
        Ok(agena_plugin_sdk::InitOutcome::ack(
            agena_plugin_sdk::Plugin::manifest(self),
        ))
    }

    #[hook(application.ready, summary = "Start Telegram discovery and routing", help = "Starts the shared per-bot collector after application queries and commands are available.")]
    async fn application_ready(&self) -> Result<()> {
        self.start_background()
    }

    #[hook(
        shutdown,
        summary = "Stop Telegram discovery",
        help = "Stops background collection before the plugin is unloaded."
    )]
    async fn shutdown(&self) -> Result<()> {
        self.stop_background();
        Ok(())
    }

    #[command(id = "list_ai_bots", title = "List local AI Bots")]
    async fn list_ai_bots(&self) -> Result<CommandResult> {
        self.local_ai_bots().await
    }

    #[command(id = "add_chat", title = "Add a Telegram chat by ID")]
    async fn add_chat(
        &self,
        bot_id: String,
        chat_id: String,
        kind: ConversationKind,
        title: Option<String>,
    ) -> Result<CommandResult> {
        self.add_manual_chat(bot_id, chat_id, kind, title).await
    }

    #[command(id = "check_connection", title = "Verify Telegram bot connection")]
    async fn check_connection(&self, input: BotSelection) -> Result<CommandResult> {
        self.verify_bot(input).await
    }
    #[command(id = "list_bots", title = "List configured Telegram bots")]
    async fn list_bots(&self) -> Result<CommandResult> {
        self.bot_catalog().await
    }
    #[command(id = "save_bot", title = "Save Telegram bot configuration")]
    async fn save_bot(
        &self,
        expected_revision: u64,
        bot: BotConfiguration,
        #[arg(secret, optional)] token: Option<String>,
    ) -> Result<CommandResult> {
        self.save_managed_bot(expected_revision, bot, token).await
    }
    #[command(id = "remove_bot", title = "Remove a disabled Telegram bot")]
    async fn remove_bot(&self, expected_revision: u64, bot_id: String) -> Result<CommandResult> {
        self.retire_bot(expected_revision, bot_id).await
    }
    #[command(id = "bot_inventory", title = "Inspect Telegram bot chats and inbox")]
    async fn bot_inventory(&self, input: BotSelection) -> Result<CommandResult> {
        self.inspect_bot(input, false).await
    }
    #[command(id = "sync_chats", title = "Synchronize Telegram received chats")]
    async fn sync_chats(&self, input: BotSelection) -> Result<CommandResult> {
        self.inspect_bot(input, true).await
    }
    #[command(
        id = "inspect_chat",
        title = "Inspect Telegram chat and bot membership"
    )]
    async fn inspect_chat(&self, bot_id: Option<String>, chat_id: String) -> Result<CommandResult> {
        self.inspect_room(BotSelection { bot_id }, chat_id).await
    }

    #[service("agena.integration", version = 1, method = "describe")]
    async fn describe(&self) -> Result<AdapterDescription> {
        Ok(AdapterDescription {
            kind: AdapterKind::Channel,
            title: "Telegram".into(),
            description: "Direct chats, group chats and channels over the official Bot API, with no public endpoint required.".into(),
            // A binding selects rooms; the Bot API endpoint, token variable and
            // verified bot identity are plugin settings. Advertising the
            // connection here would offer a field the host stores but the
            // adapter deliberately never reads.
            config_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {"bot_id":{"type":"string","minLength":1,"maxLength":64}}
            }),
            // Cheap to allow: the adapter long-polls and the host's own tick
            // already bounds how often `poll` runs.
            minimum_poll_seconds: 1,
            // Telegram has no idempotency key, so a lost acknowledgement must
            // stay uncertain rather than be replayed blindly.
            idempotent_send: false,
            channel_context_version: Some(1),
        })
    }

    #[service("agena.integration", version = 1, method = "poll")]
    async fn poll(&self, request: &AdapterPollRequest) -> Result<AdapterBatch> {
        self.poll_shared(request).await
    }

    #[service("agena.integration", version = 1, method = "send")]
    async fn send(&self, request: &ChannelDelivery) -> Result<ChannelReceipt> {
        let selection: BotSelection =
            serde_json::from_value(request.config.clone()).map_err(|_| {
                PluginError::invalid_params("Telegram binding config may only select bot_id")
            })?;
        let resolved = self
            .resolve_bot(&selection, request.channel.as_ref())
            .await?;
        let gate = self.account_lock(&resolved.id);
        let _account = gate.lock().await;
        let resolved = self
            .resolve_bot(&selection, request.channel.as_ref())
            .await?;
        if !resolved.enabled {
            return Ok(receipt(SendDisposition::Rejected, None));
        }
        self.require_binding_owner(&resolved, &request.binding_id)
            .await?;
        if self.host.get().is_some()
            && !request
                .kind
                .is_some_and(|kind| resolved.chat_access.allows(&request.resource, kind))
        {
            return Ok(receipt(SendDisposition::Rejected, None));
        }
        let config = resolved.connection;
        self.bot_identity(&config, false).await?;
        let chunks = split_markdown(&request.text, SEND_CHUNK_UNITS);
        if chunks.is_empty() {
            return Ok(receipt(SendDisposition::Rejected, None));
        }
        let mut anchor: Option<String> = None;
        for (sent, chunk) in chunks.iter().enumerate() {
            let value = match self
                .send_chunk(&config, &request.resource, request.thread.as_deref(), chunk)
                .await
            {
                Ok(value) => value,
                Err(failure) => {
                    // Some chunks reached the room. Replaying the delivery
                    // would repeat them, so the only honest verdict is that
                    // its final state is unknown.
                    let disposition = if sent > 0 {
                        SendDisposition::Uncertain
                    } else {
                        failure.receipt_disposition()
                    };
                    return Ok(ChannelReceipt {
                        disposition,
                        remote_id: anchor,
                        retry_after_seconds: (disposition == SendDisposition::Retryable)
                            .then_some(failure.retry_after)
                            .flatten(),
                    });
                }
            };
            if anchor.is_none() {
                anchor = value
                    .get("message_id")
                    .and_then(Value::as_i64)
                    .map(|id| id.to_string());
            }
        }
        Ok(receipt(SendDisposition::Sent, anchor))
    }

    /// Send one chunk, falling back to plain text if Telegram rejects the
    /// markup. A formatting bug must not cost the operator the whole answer.
    async fn send_chunk(
        &self,
        config: &TelegramConfig,
        chat_id: &str,
        thread: Option<&str>,
        chunk: &str,
    ) -> std::result::Result<Value, ApiFailure> {
        let mut body = json!({
            "chat_id": chat_id,
            "text": markdown_to_html(chunk),
            "parse_mode": "HTML",
            "disable_web_page_preview": true,
        });
        if let Some(thread) = thread {
            body["message_thread_id"] = json!(thread);
        }
        match self.call(config, "sendMessage", body).await {
            Err(failure) if failure.mentions_entity_parsing() => {
                let mut plain = json!({ "chat_id": chat_id, "text": chunk });
                if let Some(thread) = thread {
                    plain["message_thread_id"] = json!(thread);
                }
                self.call(config, "sendMessage", plain).await
            }
            outcome => outcome,
        }
    }

    /// The bot behind this token, verified against the configured account.
    async fn bot_identity(
        &self,
        config: &TelegramConfig,
        force_refresh: bool,
    ) -> Result<BotIdentity> {
        use sha2::{Digest, Sha256};
        let credential = self.credential(config).await?;
        let key = format!(
            "{}|{}",
            config.api_base.trim(),
            hex::encode(Sha256::digest(credential.as_bytes()))
        );
        if !force_refresh {
            let cached = self
                .identities
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&key)
                .cloned();
            if let Some(bot) = cached {
                return verify_account(config, bot);
            }
        }
        let value = self
            .call(config, "getMe", json!({}))
            .await
            .map_err(|failure| failure.into_error("getMe"))?;
        let id = value
            .get("id")
            .and_then(Value::as_i64)
            .ok_or_else(|| PluginError::internal("Telegram getMe returned no bot identity"))?;
        let username = value
            .get("username")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| PluginError::internal("Telegram bot has no username"))?
            .to_owned();
        let bot = verify_account(config, BotIdentity { id, username })?;
        self.identities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, bot.clone());
        Ok(bot)
    }

    async fn call(
        &self,
        config: &TelegramConfig,
        method: &str,
        body: Value,
    ) -> std::result::Result<Value, ApiFailure> {
        config.validate().map_err(ApiFailure::from_plugin)?;
        let token = self
            .credential(config)
            .await
            .map_err(ApiFailure::from_plugin)?;
        self.call_with_token(config, &token, method, body).await
    }

    async fn call_with_token(
        &self,
        config: &TelegramConfig,
        token: &str,
        method: &str,
        body: Value,
    ) -> std::result::Result<Value, ApiFailure> {
        config
            .validate_transport()
            .map_err(ApiFailure::from_plugin)?;
        management::validate_secret_token(token).map_err(ApiFailure::from_plugin)?;
        // The token is part of the path, so this URL is never logged or echoed.
        let url = format!(
            "{}/bot{token}/{method}",
            config.api_base.trim().trim_end_matches('/')
        );
        let mut response = self
            .client()
            .post(url)
            .timeout(if method == "getMe" {
                Duration::from_secs(3)
            } else {
                HTTP_TIMEOUT
            })
            .json(&body)
            .send()
            .await
            .map_err(|error| ApiFailure {
                transport: true,
                status: None,
                code: None,
                description: if error.is_timeout() {
                    "the request to Telegram timed out".into()
                } else {
                    "Telegram is unreachable from this host".into()
                },
                retry_after: None,
            })?;
        let status = response.status().as_u16();
        // Bound untrusted/self-hosted API bodies without buffering an entire
        // oversized response. A failed read does not confirm any new offset.
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ApiFailure {
            transport: true,
            status: Some(status),
            code: None,
            description: "the Telegram response body was truncated".into(),
            retry_after: None,
        })? {
            if bytes.len().saturating_add(chunk.len()) > 8 * 1024 * 1024 {
                return Err(ApiFailure {
                    transport: true,
                    status: Some(status),
                    code: None,
                    description: "Telegram response exceeds the 8 MiB limit".into(),
                    retry_after: None,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if (200..300).contains(&status) && value.get("ok").and_then(Value::as_bool) == Some(true) {
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
        Err(ApiFailure {
            transport: false,
            status: Some(status),
            code: value
                .get("error_code")
                .and_then(Value::as_i64)
                .or_else(|| Some(i64::from(status))),
            description: value
                .get("description")
                .and_then(Value::as_str)
                .map(|text| bounded_text(&text.replace(token, "[redacted]")))
                .unwrap_or_else(|| format!("Telegram answered HTTP {status}")),
            retry_after: value
                .get("parameters")
                .and_then(|parameters| parameters.get("retry_after"))
                .and_then(Value::as_i64)
                .and_then(|seconds| u32::try_from(seconds).ok()),
        })
    }

    /// Map one Bot API update onto the host's normalized event, or drop it.
    ///
    /// Dropping is a decision, not an oversight: an edit replays a message
    /// identity the host deliberately refuses to overwrite, a service message
    /// carries no instruction, and a room shape the binding does not declare
    /// would only be refused after crossing the wire.
    fn event_from_update(
        &self,
        update: &Value,
        config: &TelegramConfig,
        bot: &BotIdentity,
        request: &AdapterPollRequest,
    ) -> Option<ExternalEvent> {
        if let Some(message) = update.get("message") {
            return self.message_event(message, config, bot, request, false);
        }
        if let Some(post) = update.get("channel_post") {
            return self.message_event(post, config, bot, request, true);
        }
        if let Some(change) = update.get("chat_member") {
            return revocation_event(change, config, request);
        }
        None
    }

    fn message_event(
        &self,
        message: &Value,
        config: &TelegramConfig,
        bot: &BotIdentity,
        request: &AdapterPollRequest,
        channel_post: bool,
    ) -> Option<ExternalEvent> {
        let chat = message.get("chat")?;
        let chat_id = chat.get("id").and_then(Value::as_i64)?;
        let kind = conversation_kind(chat.get("type").and_then(Value::as_str))?;
        if !request.channel_kinds.is_empty() && !request.channel_kinds.contains(&kind) {
            return None;
        }
        let message_id = message.get("message_id").and_then(Value::as_i64)?;
        let occurred_at_ms = message
            .get("date")
            .and_then(Value::as_i64)?
            .checked_mul(1000)?;
        // History from before the grant existed is checkpointed, never
        // replayed: enabling a connection must not answer a week of backlog.
        if occurred_at_ms < request.not_before_ms {
            return None;
        }
        let from = message.get("from");
        // A channel post has no author: Telegram publishes it as the channel
        // itself, so the room is the only identity the platform discloses.
        let (actor, sender_name, sender_is_bot) = match (from, channel_post) {
            (Some(from), _) => (
                from.get("id").and_then(Value::as_i64)?.to_string(),
                display_name(from),
                from.get("is_bot").and_then(Value::as_bool).unwrap_or(false),
            ),
            (None, true) => (format!("channel:{chat_id}"), None, false),
            (None, false) => return None,
        };
        let text = message_text(message)?;
        let thread = match kind {
            // A private chat has no topics, and the host refuses a thread on a
            // direct conversation.
            ConversationKind::Direct => None,
            _ => message
                .get("message_thread_id")
                .and_then(Value::as_i64)
                .map(|id| id.to_string()),
        };
        let addressed_to_bot = match kind {
            ConversationKind::Direct => true,
            // A broadcast channel never addresses the bot. A binding that
            // wants to observe every post selects the `every_message` trigger.
            ConversationKind::Broadcast => false,
            ConversationKind::Group => names_the_bot(message, bot),
        };
        Some(ExternalEvent {
            id: format!("telegram:{chat_id}:{message_id}"),
            event_type: "message".into(),
            resource: chat_id.to_string(),
            actor: actor.clone(),
            thread,
            bot: sender_is_bot,
            occurred_at_ms,
            text,
            data: json!({
                "telegram": {
                    "message_id": message_id,
                    "chat_type": kind.as_str(),
                    "chat_title": chat.get("title").and_then(Value::as_str).map(bounded_text),
                    "sender": sender_name,
                    "sender_is_bot": sender_is_bot,
                    "names_bot": addressed_to_bot,
                    "media": media_kind(message),
                }
            }),
            channel: Some(ChannelEnvelope {
                identity: ChannelIdentity {
                    tenant: TENANT.into(),
                    account: config.account.to_ascii_lowercase(),
                },
                kind,
                // The update arrived over the Bot API channel authenticated by
                // this bot's own token, so the sender id is Telegram's, not
                // something the sender wrote.
                actor_verified: true,
                // Posting in a chat is evidence of being in it right now.
                // Leaving is reported separately as a `chat_member` update.
                is_member: true,
                addressed_to_bot,
                verified_at_ms: request.now_ms,
            }),
        })
    }
}

/// Map a verified platform removal onto the host's revocation event.
///
/// Only a transition out of the chat counts. The host applies this ahead of
/// ordinary messages and outside chat budgets, and it can only remove access.
fn revocation_event(
    change: &Value,
    config: &TelegramConfig,
    request: &AdapterPollRequest,
) -> Option<ExternalEvent> {
    let chat = change.get("chat")?;
    let chat_id = chat.get("id").and_then(Value::as_i64)?;
    let kind = conversation_kind(chat.get("type").and_then(Value::as_str))?;
    let previous = change.get("old_chat_member")?;
    let current = change.get("new_chat_member")?;
    let status = current.get("status").and_then(Value::as_str)?;
    if !matches!(status, "left" | "kicked")
        || previous.get("status").and_then(Value::as_str) == Some(status)
    {
        return None;
    }
    let user = current.get("user").or_else(|| previous.get("user"))?;
    let actor = user.get("id").and_then(Value::as_i64)?.to_string();
    let occurred_at_ms = change
        .get("date")
        .and_then(Value::as_i64)?
        .checked_mul(1000)?;
    if occurred_at_ms < request.not_before_ms {
        return None;
    }
    Some(ExternalEvent {
        id: format!("telegram:{chat_id}:member:{actor}:{occurred_at_ms}"),
        event_type: "membership.revoked".into(),
        resource: chat_id.to_string(),
        actor,
        thread: None,
        bot: false,
        occurred_at_ms,
        text: "(member left this chat)".into(),
        data: json!({ "telegram": { "chat_type": kind.as_str(), "status": status } }),
        channel: Some(ChannelEnvelope {
            identity: ChannelIdentity {
                tenant: TENANT.into(),
                account: config.account.to_ascii_lowercase(),
            },
            kind,
            actor_verified: true,
            is_member: false,
            addressed_to_bot: false,
            verified_at_ms: request.now_ms,
        }),
    })
}

/// A token pasted into the wrong connection is a real misconfiguration and
/// must fail loudly instead of quietly serving another bot's rooms.
fn verify_account(config: &TelegramConfig, bot: BotIdentity) -> Result<BotIdentity> {
    if !bot.username.eq_ignore_ascii_case(config.account.trim()) {
        return Err(PluginError::invalid_params(format!(
            "this token belongs to @{}, but account is configured as @{}; correct the connection settings",
            bot.username, config.account
        )));
    }
    Ok(bot)
}

fn conversation_kind(chat_type: Option<&str>) -> Option<ConversationKind> {
    match chat_type {
        Some("private") => Some(ConversationKind::Direct),
        Some("group" | "supergroup") => Some(ConversationKind::Group),
        Some("channel") => Some(ConversationKind::Broadcast),
        _ => None,
    }
}

/// True when this message names the bot, using only server-produced evidence.
///
/// Scanning the raw text for `@name` would also match an impostor string in a
/// link preview, so the message entities are consulted instead. Telegram gives
/// entity offsets in UTF-16 units and puts no `text` on a `mention` entity, so
/// the slice is taken from the message body itself.
fn names_the_bot(message: &Value, bot: &BotIdentity) -> bool {
    let username = format!("@{}", bot.username.to_ascii_lowercase());
    let body = message.get("text").and_then(Value::as_str);
    let named = ["entities", "caption_entities"]
        .iter()
        .filter_map(|key| message.get(*key).and_then(Value::as_array))
        .flatten()
        .any(|entity| {
            let slice = || {
                let offset = usize::try_from(entity.get("offset")?.as_u64()?).ok()?;
                let length = usize::try_from(entity.get("length")?.as_u64()?).ok()?;
                utf16_slice(body?, offset, length)
            };
            match entity.get("type").and_then(Value::as_str) {
                Some("mention") => {
                    slice().is_some_and(|text| text.to_ascii_lowercase().trim_end() == username)
                }
                Some("text_mention") => {
                    entity
                        .get("user")
                        .and_then(|user| user.get("id"))
                        .and_then(Value::as_i64)
                        == Some(bot.id)
                }
                // `/command@bot_name` addresses this bot and nothing else.
                Some("bot_command") => slice()
                    .is_some_and(|text| text.to_ascii_lowercase().trim_end().ends_with(&username)),
                _ => false,
            }
        });
    named
        || message
            .get("reply_to_message")
            .and_then(|reply| reply.get("from"))
            .and_then(|from| from.get("id"))
            .and_then(Value::as_i64)
            == Some(bot.id)
}

/// Slice a string by Telegram's UTF-16 offset and length.
fn utf16_slice(text: &str, offset: usize, length: usize) -> Option<String> {
    let mut units = 0usize;
    let mut start = None;
    for (index, ch) in text.char_indices() {
        if units == offset && start.is_none() {
            start = Some(index);
        }
        if units >= offset + length {
            return Some(text[start?..index].to_owned());
        }
        units += ch.len_utf16();
    }
    if units == offset {
        start = Some(text.len());
    }
    Some(text[start?..].to_owned())
}

/// The user's words, or an honest placeholder for a message that has none.
///
/// Media is not downloaded. The integration protocol is text in, text out, and
/// pretending an attachment had been read would be worse than saying so.
fn message_text(message: &Value) -> Option<String> {
    for key in ["text", "caption"] {
        if let Some(value) = message.get(key).and_then(Value::as_str) {
            let value = bounded_text(value);
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
    }
    media_kind(message).map(|kind| format!("[{kind} omitted: this connection handles text only]"))
}

fn media_kind(message: &Value) -> Option<&'static str> {
    [
        ("photo", "photo"),
        ("video", "video"),
        ("video_note", "video note"),
        ("animation", "animation"),
        ("voice", "voice message"),
        ("audio", "audio"),
        ("document", "document"),
        ("sticker", "sticker"),
        ("location", "location"),
        ("contact", "contact"),
        ("poll", "poll"),
    ]
    .into_iter()
    .find_map(|(key, label)| message.get(key).is_some().then_some(label))
}

fn display_name(user: &Value) -> Option<String> {
    let parts = ["first_name", "last_name"]
        .into_iter()
        .filter_map(|key| user.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return user
            .get("username")
            .and_then(Value::as_str)
            .map(bounded_text);
    }
    Some(bounded_text(&parts.join(" ")))
}

/// Trim provider text to the host's 24 KiB event body without splitting a
/// character.
fn bounded_text(text: &str) -> String {
    const LIMIT: usize = 24 * 1024;
    if text.len() <= LIMIT {
        return text.to_owned();
    }
    let mut end = LIMIT;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn is_env_name(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_bot_username(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 64
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn receipt(disposition: SendDisposition, remote_id: Option<String>) -> ChannelReceipt {
    ChannelReceipt {
        disposition,
        remote_id,
        retry_after_seconds: None,
    }
}

/// One failed Bot API round-trip, and the honest thing to do about it.
struct ApiFailure {
    transport: bool,
    status: Option<u16>,
    code: Option<i64>,
    description: String,
    retry_after: Option<u32>,
}

impl ApiFailure {
    fn from_plugin(error: PluginError) -> Self {
        Self {
            transport: false,
            status: None,
            code: None,
            description: error.to_string(),
            retry_after: None,
        }
    }

    /// A failure that says nothing about the grant and should simply be
    /// retried: rate limits, provider outages and unreachable networks.
    fn transient(&self) -> bool {
        self.transport || matches!(self.code, Some(429)) || matches!(self.code, Some(500..=599))
    }

    fn mentions_entity_parsing(&self) -> bool {
        self.code == Some(400) && self.description.contains("can't parse entities")
    }

    fn receipt_disposition(&self) -> SendDisposition {
        if self.transient() {
            SendDisposition::Retryable
        } else if matches!(self.code, Some(400..=499)) {
            // Telegram refused the message itself: retrying the same text
            // would be refused again.
            SendDisposition::Rejected
        } else {
            SendDisposition::Uncertain
        }
    }

    fn into_error(self, method: &str) -> PluginError {
        let diagnostic = format!("Telegram {method} failed: {}", self.description);
        match self.status {
            // A refused credential or a wrong endpoint is configuration, not
            // an outage, so say which so the operator can fix it.
            Some(401 | 403 | 404) => PluginError::invalid_params(diagnostic),
            _ => PluginError::internal(diagnostic),
        }
    }
}

/// Split assistant markdown into chunks Telegram accepts.
///
/// A fence that a split lands inside is closed and reopened, so no chunk ever
/// renders as unterminated code. Length is counted in UTF-16 units because that
/// is what Telegram bounds, which is what makes emoji and CJK count correctly.
pub(crate) fn split_markdown(text: &str, limit: usize) -> Vec<String> {
    // A tiny limit cannot hold a fence marker plus content; refuse to silently
    // produce chunks that exceed what the caller asked for.
    let limit = limit.max(16);
    // Expand any single line wider than the budget first, so the greedy pass
    // below only ever reasons about segments that can fit in a chunk.
    let mut segments: Vec<String> = Vec::new();
    for line in text.split_inclusive('\n') {
        if utf16_len(line) <= limit || line.trim_start().starts_with("```") {
            segments.push(line.to_owned());
        } else {
            segments.extend(hard_split(line, limit));
        }
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut chunk = String::new();
    let mut units = 0usize;
    // The opener of the fence this chunk sits inside, if any.
    let mut fence: Option<String> = None;
    for segment in segments {
        let is_fence = segment.trim_start().starts_with("```");
        let closes = is_fence && fence.is_some();
        let width = utf16_len(&segment);
        if units > 0 && units + width > limit {
            if fence.is_some() {
                chunk.push_str("```\n");
            }
            chunks.push(std::mem::take(&mut chunk));
            units = 0;
            if closes {
                // The marker above already closed this fence. Carrying the real
                // closing line into the next chunk would open it with a stray,
                // unbalanced fence, which Telegram rejects outright.
                fence = None;
                continue;
            }
            if let Some(open) = fence.as_ref() {
                let reopened = format!("{open}\n");
                let reopened_units = utf16_len(&reopened);
                if reopened_units + width <= limit {
                    units = reopened_units;
                    chunk.push_str(&reopened);
                } else {
                    // Too narrow to reopen and still fit: the fence ends here
                    // rather than producing an oversized or unterminated chunk.
                    fence = None;
                }
            }
        }
        if is_fence {
            fence = (!closes).then(|| normalized_fence(&segment));
        }
        units += width;
        chunk.push_str(&segment);
    }
    if !chunk.trim().is_empty() {
        if fence.is_some() {
            chunk.push_str("```\n");
        }
        chunks.push(chunk);
    }
    chunks
        .into_iter()
        .filter(|chunk| !chunk.trim().is_empty())
        .collect()
}

fn normalized_fence(line: &str) -> String {
    let line = line.trim_end();
    if line.len() > 64 {
        "```".to_owned()
    } else {
        line.to_owned()
    }
}

fn hard_split(line: &str, budget: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut piece = String::new();
    let mut units = 0usize;
    for ch in line.chars() {
        let width = ch.len_utf16();
        if units > 0 && units + width > budget {
            pieces.push(std::mem::take(&mut piece));
            units = 0;
        }
        units += width;
        piece.push(ch);
    }
    if !piece.is_empty() {
        pieces.push(piece);
    }
    if pieces.is_empty() {
        pieces.push(String::new());
    }
    pieces
}

fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// Render the assistant's markdown as Telegram's supported HTML subset.
///
/// Text is escaped before any tag is introduced, so a `<` the model writes
/// stays literal, and only constructs Telegram renders are converted. One
/// unbalanced tag makes Telegram reject the whole message, so every insertion
/// here is a matched pair.
pub(crate) fn markdown_to_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let mut lines = text.split_inclusive('\n');
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if let Some(info) = trimmed.strip_prefix("```") {
            let mut body = String::new();
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    break;
                }
                body.push_str(inner);
            }
            let body = body.strip_suffix('\n').unwrap_or(&body);
            let language = info.trim();
            if is_env_name(language) {
                out.push_str(&format!(
                    "<pre><code class=\"language-{language}\">{}</code></pre>\n",
                    escape_html(body)
                ));
            } else {
                out.push_str(&format!("<pre>{}</pre>\n", escape_html(body)));
            }
            continue;
        }
        let (content, newline) = match line.strip_suffix('\n') {
            Some(content) => (content, "\n"),
            None => (line, ""),
        };
        if let Some(quoted) = content.strip_prefix("> ") {
            out.push_str("<blockquote>");
            out.push_str(&inline_html(quoted));
            out.push_str("</blockquote>");
        } else {
            out.push_str(&inline_html(content));
        }
        out.push_str(newline);
    }
    out
}

fn inline_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    // Every branch either emits a balanced pair or copies one ordinary
    // character, so this loop always advances and never leaves a tag open.
    while let Some(ch) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
            && end > 0
        {
            out.push_str(&format!("<code>{}</code>", escape_html(&after[..end])));
            rest = &after[end + 1..];
            continue;
        }
        if let Some((marker, tag)) = [
            ("**", "b"),
            ("__", "b"),
            ("~~", "s"),
            ("*", "i"),
            ("_", "i"),
        ]
        .into_iter()
        .find(|(marker, _)| rest.starts_with(marker))
            && let Some(end) = rest[marker.len()..].find(marker)
            && end > 0
        {
            let inner = &rest[marker.len()..marker.len() + end];
            out.push_str(&format!("<{tag}>{}</{tag}>", inline_html(inner)));
            rest = &rest[marker.len() + end + marker.len()..];
            continue;
        }
        if let Some((label, url)) = parse_link(rest) {
            out.push_str(&format!(
                "<a href=\"{}\">{}</a>",
                escape_html(url),
                inline_html(label)
            ));
            rest = &rest[label.len() + url.len() + 4..];
            continue;
        }
        out.push_str(&escape_html(&ch.to_string()));
        rest = &rest[ch.len_utf8()..];
    }
    out
}

fn parse_link(text: &str) -> Option<(&str, &str)> {
    let inner = text.strip_prefix('[')?;
    let label = &inner[..inner.find(']')?];
    let after = inner[label.len() + 1..].strip_prefix('(')?;
    let url = &after[..after.find(')')?];
    if url.is_empty()
        || url.chars().any(char::is_whitespace)
        || !(url.starts_with("https://") || url.starts_with("http://"))
    {
        return None;
    }
    Some((label, url))
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests;
