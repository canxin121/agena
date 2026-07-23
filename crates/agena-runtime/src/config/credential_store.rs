use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use agena_provider::{AuthData, OpenAiResponsesBackendConfig, ProviderCredentialAuthConfig};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::{config::ProcessEnvironment, error::AppError, provider::auth::AuthStore};

use super::{
    ProviderApiAuthConfig, ProviderAuthConfig, ProviderGitlabAuthConfig, ResolvedProviderConfig,
    raw::{RawConfig, RawConfigFile},
};

#[derive(Debug, Clone)]
pub struct ProviderConfigCredentialStore {
    config_path: PathBuf,
}

impl ProviderConfigCredentialStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            config_path: path.into(),
        }
    }

    pub fn all_provider_configs(
        &self,
    ) -> Result<HashMap<String, ResolvedProviderConfig>, AppError> {
        load_provider_configs(self.config_path.as_path())
    }

    fn read_doc(&self) -> Result<JsonValue, AppError> {
        if self.config_path.exists() {
            let text = fs::read_to_string(&self.config_path)?;
            let doc = serde_json::from_str::<JsonValue>(text.as_str()).map_err(|err| {
                AppError::Config(format!("parse {}: {err}", self.config_path.display()))
            })?;
            Ok(normalize_root_object(doc))
        } else {
            Ok(JsonValue::Object(JsonMap::new()))
        }
    }

    fn write_doc(&self, doc: &JsonValue) -> Result<(), AppError> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.config_path, serde_json::to_string_pretty(doc)?)?;
        Ok(())
    }

    fn ensure_provider_auth_table<'a>(
        &self,
        doc: &'a mut JsonValue,
        provider_id: &str,
    ) -> &'a mut JsonMap<String, JsonValue> {
        if !doc.is_object() {
            *doc = JsonValue::Object(JsonMap::new());
        }

        let root = doc.as_object_mut().expect("root object");
        let providers = root
            .entry("providers".to_owned())
            .or_insert_with(|| JsonValue::Object(JsonMap::new()));
        if !providers.is_object() {
            *providers = JsonValue::Object(JsonMap::new());
        }
        let providers = providers.as_object_mut().expect("providers object");

        let provider = providers
            .entry(provider_id.to_owned())
            .or_insert_with(|| JsonValue::Object(JsonMap::new()));
        if !provider.is_object() {
            *provider = JsonValue::Object(JsonMap::new());
        }
        let provider = provider.as_object_mut().expect("provider object");

        let auth = provider
            .entry("auth".to_owned())
            .or_insert_with(|| JsonValue::Object(JsonMap::new()));
        if !auth.is_object() {
            *auth = JsonValue::Object(JsonMap::new());
        }
        auth.as_object_mut().expect("provider auth object")
    }
}

impl AuthStore for ProviderConfigCredentialStore {
    fn get(&self, provider_id: &str) -> Result<Option<AuthData>, AppError> {
        Ok(self
            .all_provider_configs()?
            .remove(provider_id)
            .and_then(|resolved| provider_auth_data(&resolved)))
    }

    fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), AppError> {
        let mut doc = self.read_doc()?;
        let auth_table = self.ensure_provider_auth_table(&mut doc, provider_id);
        auth_table.remove("mode");
        auth_table.remove("credential");
        auth_table.remove("issuer");
        auth_table.remove("access");

        match auth {
            AuthData::Api { key } => {
                auth_table.insert("mode".to_owned(), JsonValue::String("api".to_owned()));
                auth_table.insert(
                    "api_key".to_owned(),
                    serde_json::json!({
                        "kind": "inline",
                        "value": key,
                    }),
                );
            }
            AuthData::OAuth { .. } => {
                let issuer = auth.issuer().ok_or_else(|| {
                    AppError::Config(format!(
                        "{provider_id} oauth credential must include an issuer"
                    ))
                })?;
                auth_table.insert(
                    "mode".to_owned(),
                    JsonValue::String("credential".to_owned()),
                );
                auth_table.insert(
                    "issuer".to_owned(),
                    JsonValue::String(credential_issuer_value(issuer).to_owned()),
                );
                auth_table.insert("credential".to_owned(), auth_data_item(auth));
            }
            AuthData::WellKnown { .. } => {
                auth_table.insert(
                    "mode".to_owned(),
                    JsonValue::String("credential".to_owned()),
                );
                auth_table.insert("credential".to_owned(), auth_data_item(auth));
            }
        }
        self.write_doc(&doc)
    }

    fn remove(&self, provider_id: &str) -> Result<(), AppError> {
        if !self.config_path.exists() {
            return Ok(());
        }

        let mut doc = self.read_doc()?;
        let Some(providers) = doc
            .as_object_mut()
            .and_then(|root| root.get_mut("providers"))
            .and_then(JsonValue::as_object_mut)
        else {
            return Ok(());
        };
        let Some(provider) = providers
            .get_mut(provider_id)
            .and_then(JsonValue::as_object_mut)
        else {
            return Ok(());
        };
        let Some(auth) = provider.get_mut("auth").and_then(JsonValue::as_object_mut) else {
            return Ok(());
        };

        auth.remove("credential");
        auth.remove("issuer");
        auth.remove("api_key");
        auth.remove("access");

        if auth_table_has_no_sources(auth) {
            auth.remove("mode");
        }

        if auth.is_empty() {
            provider.remove("auth");
        }

        self.write_doc(&doc)
    }
}

pub fn provider_auth_data(resolved: &ResolvedProviderConfig) -> Option<AuthData> {
    match &resolved.auth {
        ProviderAuthConfig::None => None,
        ProviderAuthConfig::Api(api) => secret_auth_data(api),
        ProviderAuthConfig::Credential(config) => config.credential().cloned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderOAuthTarget {
    OpenAi,
    Gitlab { instance_url: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderDeviceAuthTarget {
    OpenAi,
    Copilot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAuthTargetError {
    AmbiguousProvider,
    AmbiguousGitlab,
}

pub fn provider_supports_openai_oauth(resolved: &ResolvedProviderConfig) -> bool {
    matches!(
        resolved.auth,
        ProviderAuthConfig::Credential(ProviderCredentialAuthConfig::OpenaiChatgpt { .. })
    ) && resolved.adapters.values().any(|adapter| {
        matches!(
            &adapter.definition,
            super::ProviderAdapterDefinition::OpenAiResponses(config)
                if matches!(
                    config.options.backend,
                    OpenAiResponsesBackendConfig::ChatgptCodex
                )
        )
    })
}

pub fn provider_has_gitlab_adapter(resolved: &ResolvedProviderConfig) -> bool {
    matches!(
        resolved.auth,
        ProviderAuthConfig::Api(ref api)
            if api.gitlab().is_some()
    ) || resolved.adapters.values().any(|adapter| {
        matches!(
            adapter.definition,
            super::ProviderAdapterDefinition::Gitlab(_)
        )
    }) || matches!(
        resolved.auth,
        ProviderAuthConfig::Credential(ProviderCredentialAuthConfig::Gitlab { .. })
    )
}

pub fn provider_gitlab_instance_url(resolved: &ResolvedProviderConfig) -> Option<String> {
    if let ProviderAuthConfig::Api(api) = &resolved.auth
        && let Some(config) = api.gitlab()
    {
        return Some(
            config
                .instance_url
                .clone()
                .unwrap_or_else(|| "https://gitlab.com".to_owned()),
        );
    }
    if let ProviderAuthConfig::Credential(config) = &resolved.auth
        && let Some(gitlab) = config.gitlab()
    {
        return Some(
            gitlab
                .instance_url
                .clone()
                .unwrap_or_else(|| "https://gitlab.com".to_owned()),
        );
    }

    let mut urls = resolved
        .adapters
        .values()
        .filter_map(|adapter| {
            if let super::ProviderAdapterDefinition::Gitlab(options) = &adapter.definition {
                options.instance_url.clone()
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    urls.sort();
    urls.dedup();
    match urls.as_slice() {
        [] => None,
        [instance_url] => Some(instance_url.clone()),
        _ => None,
    }
}

pub fn provider_supports_copilot_device(resolved: &ResolvedProviderConfig) -> bool {
    matches!(
        resolved.auth,
        ProviderAuthConfig::Credential(ProviderCredentialAuthConfig::GithubCopilot { .. })
    ) && resolved.adapters.values().any(|adapter| {
        matches!(
            &adapter.definition,
            super::ProviderAdapterDefinition::OpenAiResponses(config)
                if matches!(
                    config.options.backend,
                    OpenAiResponsesBackendConfig::Api
                )
        ) || matches!(
            &adapter.definition,
            super::ProviderAdapterDefinition::OpenAiChatCompletions(_)
        )
    })
}

pub fn provider_supports_api_key_write(resolved: &ResolvedProviderConfig) -> bool {
    match resolved.auth {
        ProviderAuthConfig::Api(_) => {
            !provider_supports_openai_oauth(resolved) && !provider_supports_copilot_device(resolved)
        }
        ProviderAuthConfig::Credential(_) => false,
        ProviderAuthConfig::None => false,
    }
}

pub fn resolve_provider_oauth_target(
    resolved: &ResolvedProviderConfig,
) -> Result<Option<ProviderOAuthTarget>, ProviderAuthTargetError> {
    let openai = provider_supports_openai_oauth(resolved);
    let gitlab = provider_has_gitlab_adapter(resolved);
    match (openai, gitlab) {
        (true, false) => Ok(Some(ProviderOAuthTarget::OpenAi)),
        (false, true) => provider_gitlab_instance_url(resolved)
            .map(|instance_url| ProviderOAuthTarget::Gitlab { instance_url })
            .map(Some)
            .ok_or(ProviderAuthTargetError::AmbiguousGitlab),
        (false, false) => Ok(None),
        _ => Err(ProviderAuthTargetError::AmbiguousProvider),
    }
}

pub fn resolve_provider_device_auth_target(
    resolved: &ResolvedProviderConfig,
) -> Result<Option<ProviderDeviceAuthTarget>, ProviderAuthTargetError> {
    let openai = provider_supports_openai_oauth(resolved);
    let copilot = provider_supports_copilot_device(resolved);
    match (openai, copilot) {
        (true, false) => Ok(Some(ProviderDeviceAuthTarget::OpenAi)),
        (false, true) => Ok(Some(ProviderDeviceAuthTarget::Copilot)),
        (false, false) => Ok(None),
        _ => Err(ProviderAuthTargetError::AmbiguousProvider),
    }
}

fn load_provider_configs(path: &Path) -> Result<HashMap<String, ResolvedProviderConfig>, AppError> {
    let env = ProcessEnvironment;
    let file_state = RawConfigFile::read(path)?;
    let env_overlay = RawConfig::from_env(&env)?;

    let mut merged = if file_state.found {
        file_state.config
    } else {
        RawConfig::default()
    };
    if !env_overlay.is_empty() {
        merged.merge_from(env_overlay);
    }

    Ok(merged
        .resolve_with_env(&env)?
        .providers
        .into_iter()
        .collect())
}

fn secret_auth_data(secret: &ProviderApiAuthConfig) -> Option<AuthData> {
    if let Some(key) = normalize_text(secret.api_key()) {
        return Some(AuthData::Api { key });
    }
    if let Some(gitlab) = secret.gitlab() {
        return gitlab_auth_data(&gitlab);
    }
    None
}

fn gitlab_auth_data(secret: &ProviderGitlabAuthConfig) -> Option<AuthData> {
    if let Some(credential) = secret.access.credential().cloned() {
        return Some(credential);
    }
    if let Some(key) = secret
        .access
        .api_key_source()
        .and_then(|source| normalize_text(source.inline()))
    {
        return Some(AuthData::Api { key });
    }
    None
}

fn normalize_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn auth_table_has_no_sources(table: &JsonMap<String, JsonValue>) -> bool {
    let source_keys = [
        "credential",
        "api_key",
        "access",
        "service_key_env",
        "profile",
        "access_key_id",
        "secret_access_key",
        "session_token",
    ];

    !source_keys.iter().any(|key| table.contains_key(*key))
}

fn credential_issuer_value(issuer: agena_provider::CredentialIssuer) -> &'static str {
    match issuer {
        agena_provider::CredentialIssuer::OpenaiChatgpt => "openai_chatgpt",
        agena_provider::CredentialIssuer::GithubCopilot => "github_copilot",
        agena_provider::CredentialIssuer::Gitlab => "gitlab",
        agena_provider::CredentialIssuer::GoogleAdc => "google_adc",
        agena_provider::CredentialIssuer::SapAiCore => "sap_ai_core",
    }
}

fn auth_data_item(auth: AuthData) -> JsonValue {
    let mut table = JsonMap::new();
    match auth {
        AuthData::Api { key } => {
            table.insert("type".to_owned(), JsonValue::String("api".to_owned()));
            table.insert("key".to_owned(), JsonValue::String(key));
        }
        AuthData::OAuth {
            issuer,
            refresh,
            access,
            id_token,
            expires_at_ms,
            account_id,
            chatgpt_account_is_fedramp,
            enterprise_url,
            user,
        } => {
            table.insert("type".to_owned(), JsonValue::String("oauth".to_owned()));
            if let Some(issuer) = issuer {
                table.insert(
                    "issuer".to_owned(),
                    JsonValue::String(credential_issuer_value(issuer).to_owned()),
                );
            }
            table.insert("refresh".to_owned(), JsonValue::String(refresh));
            table.insert("access".to_owned(), JsonValue::String(access));
            if let Some(id_token) = normalize_text(id_token.as_deref()) {
                table.insert("id_token".to_owned(), JsonValue::String(id_token));
            }
            table.insert("expires_at_ms".to_owned(), JsonValue::from(expires_at_ms));
            if let Some(account_id) = normalize_text(account_id.as_deref()) {
                table.insert("account_id".to_owned(), JsonValue::String(account_id));
            }
            if chatgpt_account_is_fedramp {
                table.insert(
                    "chatgpt_account_is_fedramp".to_owned(),
                    JsonValue::Bool(true),
                );
            }
            if let Some(enterprise_url) = normalize_text(enterprise_url.as_deref()) {
                table.insert(
                    "enterprise_url".to_owned(),
                    JsonValue::String(enterprise_url),
                );
            }
            if let Some(user) = user {
                let mut user_table = JsonMap::new();
                user_table.insert("id".to_owned(), JsonValue::String(user.id));
                user_table.insert("username".to_owned(), JsonValue::String(user.username));
                if let Some(name) = normalize_text(user.name.as_deref()) {
                    user_table.insert("name".to_owned(), JsonValue::String(name));
                }
                if let Some(email) = normalize_text(user.email.as_deref()) {
                    user_table.insert("email".to_owned(), JsonValue::String(email));
                }
                if let Some(avatar_url) = normalize_text(user.avatar_url.as_deref()) {
                    user_table.insert("avatar_url".to_owned(), JsonValue::String(avatar_url));
                }
                table.insert("user".to_owned(), JsonValue::Object(user_table));
            }
        }
        AuthData::WellKnown { key, token } => {
            table.insert(
                "type".to_owned(),
                JsonValue::String("well_known".to_owned()),
            );
            table.insert("key".to_owned(), JsonValue::String(key));
            table.insert("token".to_owned(), JsonValue::String(token));
        }
    }
    JsonValue::Object(table)
}

fn normalize_root_object(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Null => JsonValue::Object(JsonMap::new()),
        JsonValue::Object(_) => value,
        other => other,
    }
}
