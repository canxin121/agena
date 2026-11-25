use agena_plugin_sdk::{
    PluginError, Result, SettingsContract,
    integration::{ChannelDelivery, ChannelIdentity, ConversationKind},
    macro_support::settings_contract_for_default,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::ChatPlatform;

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, bound(deserialize = "T: Deserialize<'de>"))]
struct Accounts<T> {
    /// Platform credentials and stable identities for each connection.
    #[serde(default)]
    #[schemars(length(max = 16))]
    accounts: Vec<T>,
}

impl<T> Default for Accounts<T> {
    fn default() -> Self {
        Self {
            accounts: Vec::new(),
        }
    }
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Slack {
    /// Name used in the webhook URL.
    pub name: String,
    pub team_id: String,
    pub app_id: String,
    pub bot_user_id: String,
    /// Server environment variable holding the bot token.
    pub token_env: String,
    /// Server environment variable holding the Slack signing secret.
    pub signing_secret_env: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Feishu {
    /// Name used in the webhook URL.
    pub name: String,
    pub tenant_key: String,
    pub app_id: String,
    pub bot_open_id: String,
    /// Select Lark's international API when enabled.
    #[serde(default)]
    pub lark: bool,
    pub app_secret_env: String,
    pub encrypt_key_env: String,
    pub verification_token_env: String,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct DiscordRoom {
    pub id: String,
    pub kind: ConversationKind,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Discord {
    pub name: String,
    pub bot_user_id: String,
    pub token_env: String,
    /// Channel or thread IDs visible to the bot. Set the kind for each.
    #[schemars(length(min = 1, max = 16))]
    pub rooms: Vec<DiscordRoom>,
}

#[derive(Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct WhatsApp {
    /// Name used in the webhook URL.
    pub name: String,
    pub business_account_id: String,
    pub phone_number_id: String,
    pub token_env: String,
    pub app_secret_env: String,
    pub verification_token_env: String,
}

pub(super) enum Connection {
    Slack(Slack),
    Feishu(Feishu),
    Discord(Discord),
    WhatsApp(WhatsApp),
}

impl ChatPlatform {
    pub(super) fn settings_contract(self) -> SettingsContract {
        match self {
            Self::Slack => settings_contract_for_default(Accounts::<Slack>::default()),
            Self::Feishu => settings_contract_for_default(Accounts::<Feishu>::default()),
            Self::Discord => settings_contract_for_default(Accounts::<Discord>::default()),
            Self::WhatsApp => settings_contract_for_default(Accounts::<WhatsApp>::default()),
        }
    }

    pub(super) fn parse_settings(self, value: Value) -> Result<Vec<Connection>> {
        let connections = match self {
            Self::Slack => accounts(value, Connection::Slack)?,
            Self::Feishu => accounts(value, Connection::Feishu)?,
            Self::Discord => accounts(value, Connection::Discord)?,
            Self::WhatsApp => accounts(value, Connection::WhatsApp)?,
        };
        for account in &connections {
            account.validate()?;
        }
        Ok(connections)
    }
}

fn accounts<T: DeserializeOwned>(
    value: Value,
    variant: impl Fn(T) -> Connection,
) -> Result<Vec<Connection>> {
    let settings: Accounts<T> = if value.is_null() {
        Accounts::default()
    } else {
        serde_json::from_value(value)
            .map_err(|_| PluginError::invalid_params("invalid chat adapter settings"))?
    };
    if settings.accounts.len() > 16 {
        return Err(PluginError::invalid_params(
            "configure at most 16 connections per chat platform",
        ));
    }
    Ok(settings.accounts.into_iter().map(variant).collect())
}

impl Connection {
    pub fn name(&self) -> &str {
        match self {
            Self::Slack(config) => &config.name,
            Self::Feishu(config) => &config.name,
            Self::Discord(config) => &config.name,
            Self::WhatsApp(config) => &config.name,
        }
    }

    pub(super) fn identity(&self) -> ChannelIdentity {
        let (tenant, account) = self.identity_parts();
        ChannelIdentity {
            tenant: tenant.into(),
            account: account.into(),
        }
    }

    pub(super) fn identity_parts(&self) -> (&str, &str) {
        match self {
            Self::Slack(config) => (&config.team_id, &config.bot_user_id),
            Self::Feishu(config) => (&config.tenant_key, &config.bot_open_id),
            Self::Discord(config) => ("discord", &config.bot_user_id),
            Self::WhatsApp(config) => (&config.business_account_id, &config.phone_number_id),
        }
    }

    pub(super) fn token_env(&self) -> &str {
        match self {
            Self::Slack(config) => &config.token_env,
            Self::Feishu(config) => &config.app_secret_env,
            Self::Discord(config) => &config.token_env,
            Self::WhatsApp(config) => &config.token_env,
        }
    }

    pub(super) fn validate(&self) -> Result<()> {
        let (tenant, account) = self.identity_parts();
        if self.name().len() > 48
            || !platform_id(self.name())
            || !platform_id(tenant)
            || !platform_id(account)
        {
            return Err(PluginError::invalid_params(
                "connection names and platform IDs must be nonempty alphanumeric identifiers with hyphens or underscores",
            ));
        }
        let valid = match self {
            Self::Slack(value) => platform_id(&value.app_id) && env_name(&value.signing_secret_env),
            Self::Feishu(value) => {
                platform_id(&value.app_id)
                    && env_name(&value.encrypt_key_env)
                    && env_name(&value.verification_token_env)
            }
            Self::Discord(value) => {
                decimal_id(account)
                    && (1..=16).contains(&value.rooms.len())
                    && value.rooms.iter().all(|room| decimal_id(&room.id))
                    && value
                        .rooms
                        .iter()
                        .map(|room| &room.id)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == value.rooms.len()
            }
            Self::WhatsApp(value) => {
                decimal_id(tenant)
                    && decimal_id(account)
                    && env_name(&value.app_secret_env)
                    && env_name(&value.verification_token_env)
            }
        };
        if !valid || !env_name(self.token_env()) {
            return Err(PluginError::invalid_params(
                "invalid chat platform IDs, room configuration or credential environment references",
            ));
        }
        Ok(())
    }

    pub(super) fn valid_delivery(&self, delivery: &ChannelDelivery) -> bool {
        let resource = &delivery.resource;
        let thread = delivery.thread.as_deref();
        match self {
            Self::Slack(_) => platform_id(resource) && thread.is_none_or(valid_slack_timestamp),
            Self::Feishu(_) => platform_id(resource) && thread.is_none_or(platform_id),
            Self::Discord(value) => {
                thread.is_none()
                    && value
                        .rooms
                        .iter()
                        .any(|room| room.id == *resource && delivery.kind == Some(room.kind))
            }
            Self::WhatsApp(_) => {
                decimal_id(resource)
                    && thread.is_none()
                    && delivery.kind == Some(ConversationKind::Direct)
            }
        }
    }
}

impl Feishu {
    pub(super) fn host(&self) -> &'static str {
        if self.lark {
            "https://open.larksuite.com"
        } else {
            "https://open.feishu.cn"
        }
    }
}

pub(super) fn platform_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub(super) fn decimal_id(value: &str) -> bool {
    (1..=20).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

pub(super) fn valid_slack_timestamp(value: &str) -> bool {
    value.split_once('.').is_some_and(|(seconds, usec)| {
        (1..=12).contains(&seconds.len())
            && usec.len() == 6
            && seconds
                .bytes()
                .chain(usec.bytes())
                .all(|byte| byte.is_ascii_digit())
    })
}

fn env_name(value: &str) -> bool {
    (1..=96).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !value.as_bytes()[0].is_ascii_digit()
}

pub(super) fn secret(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .filter(|value| {
            !value.trim().is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| {
            PluginError::invalid_params(format!(
                "chat adapter credential environment variable {name} is unavailable"
            ))
        })
}
