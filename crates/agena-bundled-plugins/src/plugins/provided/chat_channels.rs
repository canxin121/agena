mod api;
mod config;
mod inbox;
mod webhooks;

use std::{
    collections::BTreeSet,
    sync::{Arc, OnceLock},
};

use agena_assistant::{ChannelWebhookRequest, ChannelWebhookResponse};
use agena_plugin_sdk::{
    InitContext, InitOutcome, Plugin, PluginError, PluginManifest, PluginServiceInvokeInput,
    Result, TransportKind,
    host_api::HostClient,
    integration::{
        AdapterBatch, AdapterDescription, AdapterKind, AdapterPollRequest, ChannelDelivery,
        ChannelIdentity, ChannelReceipt, INTEGRATION_API_VERSION, INTEGRATION_SERVICE,
        SendDisposition, integration_export,
    },
    macro_support::service_method_for,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use self::{api::PlatformApi, config::Connection};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChatPlatform {
    Slack,
    Feishu,
    Discord,
    WhatsApp,
}

pub(crate) const CHAT_PLATFORM_PLUGINS: [ChatPlatform; 4] = [
    ChatPlatform::Slack,
    ChatPlatform::Feishu,
    ChatPlatform::Discord,
    ChatPlatform::WhatsApp,
];

impl ChatPlatform {
    pub(crate) fn plugin_id(self) -> &'static str {
        match self {
            Self::Slack => "agena.slack",
            Self::Feishu => "agena.feishu",
            Self::Discord => "agena.discord",
            Self::WhatsApp => "agena.whatsapp",
        }
    }

    fn name(self) -> &'static str {
        self.plugin_id()
            .strip_prefix("agena.")
            .expect("bundled plugin")
    }

    fn title(self) -> &'static str {
        match self {
            Self::Slack => "Slack",
            Self::Feishu => "Feishu / Lark",
            Self::Discord => "Discord",
            Self::WhatsApp => "WhatsApp Cloud",
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::Slack => "Receive signed Slack message events and reply using the Slack Bot API.",
            Self::Feishu => {
                "Receive signed Feishu or Lark message events and reply using the bot API."
            }
            Self::Discord => {
                "Read and reply to messages in configured Discord channels using the Discord Bot API."
            }
            Self::WhatsApp => {
                "Receive signed WhatsApp Cloud text messages and reply using the Meta Graph API."
            }
        }
    }

    fn describe(self) -> AdapterDescription {
        AdapterDescription {
            kind: AdapterKind::Channel,
            title: self.title().into(),
            description: format!(
                "{} Select the configured tenant and bot account in the channel policy.",
                self.summary()
            ),
            config_schema: json!({"type": "object", "additionalProperties": false, "properties": {}}),
            minimum_poll_seconds: if self == Self::Discord { 5 } else { 1 },
            idempotent_send: false,
            channel_context_version: Some(1),
        }
    }
}

pub(crate) struct ChatChannel {
    platform: ChatPlatform,
    state: OnceLock<ChannelState>,
}

struct ChannelState {
    host: Arc<dyn HostClient>,
    connections: Vec<Connection>,
    api: PlatformApi,
}

impl ChatChannel {
    pub(crate) fn new(platform: ChatPlatform) -> Self {
        Self {
            platform,
            state: OnceLock::new(),
        }
    }

    fn state(&self) -> Result<&ChannelState> {
        self.state
            .get()
            .ok_or_else(|| PluginError::internal("chat adapter is not initialized"))
    }

    async fn poll(&self, request: AdapterPollRequest) -> Result<AdapterBatch> {
        let state = self.state()?;
        let account = state.connection(request.channel_identity.as_ref(), &request.config)?;
        if let Connection::Discord(config) = account {
            return state.api.poll_discord(account, config, &request).await;
        }
        let batch = inbox::poll(&*state.host, account, &request).await?;
        if !batch.events.is_empty() {
            state.api.authenticate(account).await?;
        }
        Ok(batch)
    }

    async fn send(&self, request: ChannelDelivery) -> Result<ChannelReceipt> {
        let state = self.state()?;
        let Ok(account) = state.connection(request.channel.as_ref(), &request.config) else {
            return Ok(receipt(SendDisposition::Rejected, None, None));
        };
        if !account.valid_delivery(&request) || request.text.trim().is_empty() {
            return Ok(receipt(SendDisposition::Rejected, None, None));
        }
        state.api.deliver(account, &request).await
    }

    async fn webhook(&self, request: ChannelWebhookRequest) -> Result<ChannelWebhookResponse> {
        let state = self.state()?;
        let Some(account) = state
            .connections
            .iter()
            .find(|account| account.name() == request.connection)
        else {
            return Ok(ChannelWebhookResponse::NotFound);
        };
        match webhooks::decode(
            account,
            &state.connections,
            request.envelope,
            chrono::Utc::now().timestamp_millis(),
        )? {
            webhooks::Decoded::Reply(reply) => Ok(reply),
            webhooks::Decoded::Messages(messages) => {
                if !messages.is_empty() {
                    inbox::append(state.host.clone(), messages).await?;
                }
                Ok(ChannelWebhookResponse::Accepted)
            }
        }
    }
}

impl ChannelState {
    fn connection(
        &self,
        identity: Option<&ChannelIdentity>,
        config: &Value,
    ) -> Result<&Connection> {
        if config.as_object().is_none_or(|value| !value.is_empty()) {
            return Err(PluginError::invalid_params(
                "this adapter selects its account from the channel policy",
            ));
        }
        let identity = identity
            .ok_or_else(|| PluginError::invalid_params("a channel account identity is required"))?;
        self.connections
            .iter()
            .find(|account| &account.identity() == identity)
            .ok_or_else(|| {
                PluginError::invalid_params(
                    "the channel policy does not match a configured tenant and bot account",
                )
            })
    }
}

#[async_trait::async_trait]
impl Plugin for ChatChannel {
    fn manifest(&self) -> PluginManifest {
        let mut manifest =
            PluginManifest::new("agena", self.platform.name(), env!("CARGO_PKG_VERSION"));
        manifest.display_name = Some(self.platform.title().into());
        manifest.summary = Some(self.platform.summary().into());
        manifest.transports = vec![TransportKind::Static];
        manifest.settings = Some(self.platform.settings_contract());
        let mut export = integration_export(true);
        if self.platform != ChatPlatform::Discord {
            export = export.with_method(service_method_for::<
                ChannelWebhookRequest,
                ChannelWebhookResponse,
            >("webhook"));
        }
        manifest.services.exports.push(export);
        manifest
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        let connections = self.platform.parse_settings(ctx.settings)?;
        let mut names = BTreeSet::new();
        let mut identities = BTreeSet::new();
        for account in &connections {
            let identity = account.identity();
            if !names.insert(account.name())
                || !identities.insert((identity.tenant, identity.account))
            {
                return Err(PluginError::invalid_params(
                    "chat connection names and platform identities must be unique",
                ));
            }
        }
        self.state
            .set(ChannelState {
                host,
                connections,
                api: PlatformApi::new()?,
            })
            .map_err(|_| PluginError::internal("chat adapter already initialized"))?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn service_invoke(&self, input: PluginServiceInvokeInput) -> Result<Value> {
        if input.service != INTEGRATION_SERVICE || input.api_version != INTEGRATION_API_VERSION {
            return Err(PluginError::not_implemented(&input.service));
        }
        match input.method.as_str() {
            "describe" => encode(self.platform.describe()),
            "poll" => encode(self.poll(decode(input.input)?).await?),
            "send" => encode(self.send(decode(input.input)?).await?),
            "webhook" if self.platform != ChatPlatform::Discord => {
                encode(self.webhook(decode(input.input)?).await?)
            }
            _ => Err(PluginError::not_implemented(&input.method)),
        }
    }
}

fn encode<T: Serialize>(result: T) -> Result<Value> {
    serde_json::to_value(result).map_err(|error| PluginError::internal_error(&error))
}

fn decode<T: DeserializeOwned>(input: Value) -> Result<T> {
    serde_json::from_value(input)
        .map_err(|_| PluginError::invalid_params("invalid chat adapter input"))
}

fn receipt(
    disposition: SendDisposition,
    remote_id: Option<String>,
    retry_after_seconds: Option<u32>,
) -> ChannelReceipt {
    ChannelReceipt {
        disposition,
        remote_id,
        retry_after_seconds,
    }
}

#[cfg(test)]
mod tests;
