use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::{
    config::ProcessEnvironment,
    error::AppError,
    provider::auth::{AuthData, AuthStore},
};

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
    fn all(&self) -> Result<HashMap<String, AuthData>, AppError> {
        Ok(self
            .all_provider_configs()?
            .into_iter()
            .filter_map(|(provider_id, resolved)| {
                provider_auth_data(&resolved).map(|auth| (provider_id, auth))
            })
            .collect())
    }

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

        match auth {
            AuthData::Api { key } => {
                auth_table.insert("mode".to_owned(), JsonValue::String("api".to_owned()));
                auth_table.insert("api_key".to_owned(), JsonValue::String(key));
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
        ProviderAuthConfig::None | ProviderAuthConfig::BedrockSigv4(_) => None,
        ProviderAuthConfig::Api(api) => secret_auth_data(api),
        ProviderAuthConfig::Gitlab(config) => gitlab_auth_data(config),
        ProviderAuthConfig::Credential(config) => config.credential.clone(),
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
        ProviderAuthConfig::Credential(super::ProviderCredentialAuthConfig {
            issuer: crate::provider::auth::CredentialIssuer::OpenaiChatgpt,
            ..
        })
    ) && resolved.adapters.values().any(|adapter| {
        matches!(
            &adapter.definition,
            super::ProviderAdapterDefinition::OpenAi(config)
                if matches!(config.options.backend, super::OpenAiBackendConfig::ChatgptCodex)
        )
    })
}

pub fn provider_has_gitlab_adapter(resolved: &ResolvedProviderConfig) -> bool {
    matches!(
        resolved.auth,
        ProviderAuthConfig::Gitlab(_)
            | ProviderAuthConfig::Credential(super::ProviderCredentialAuthConfig {
                issuer: crate::provider::auth::CredentialIssuer::Gitlab,
                ..
            })
    ) || resolved.adapters.values().any(|adapter| {
        matches!(
            adapter.definition,
            super::ProviderAdapterDefinition::Gitlab(_)
        )
    })
}

pub fn provider_gitlab_instance_url(resolved: &ResolvedProviderConfig) -> Option<String> {
    if let ProviderAuthConfig::Gitlab(config) = &resolved.auth {
        return Some(
            config
                .instance_url
                .clone()
                .unwrap_or_else(|| "https://gitlab.com".to_owned()),
        );
    }
    if let ProviderAuthConfig::Credential(config) = &resolved.auth
        && config.issuer == crate::provider::auth::CredentialIssuer::Gitlab
    {
        return Some(
            config
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
        ProviderAuthConfig::Credential(super::ProviderCredentialAuthConfig {
            issuer: crate::provider::auth::CredentialIssuer::GithubCopilot,
            ..
        })
    ) && resolved.adapters.values().any(|adapter| {
        matches!(
            &adapter.definition,
            super::ProviderAdapterDefinition::OpenAi(config)
                if matches!(config.options.backend, super::OpenAiBackendConfig::Api)
        )
    })
}

pub fn provider_supports_api_key_write(resolved: &ResolvedProviderConfig) -> bool {
    match resolved.auth {
        ProviderAuthConfig::Api(_) => {
            !provider_supports_openai_oauth(resolved) && !provider_supports_copilot_device(resolved)
        }
        ProviderAuthConfig::Gitlab(_) => true,
        ProviderAuthConfig::Credential(_) => false,
        ProviderAuthConfig::None | ProviderAuthConfig::BedrockSigv4(_) => false,
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

    let mut merged = RawConfig::default();
    if file_state.found {
        merged.merge_from(file_state.config);
    }
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
    if let Some(key) = normalize_text(secret.api_key.as_deref()) {
        return Some(AuthData::Api { key });
    }
    None
}

fn gitlab_auth_data(secret: &ProviderGitlabAuthConfig) -> Option<AuthData> {
    if let Some(credential) = secret.credential.clone() {
        return Some(credential);
    }
    if let Some(key) = normalize_text(secret.api_key.as_deref()) {
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
        "api_key_env",
        "service_key_env",
        "profile",
        "access_key_id",
        "secret_access_key",
        "session_token",
    ];

    !source_keys.iter().any(|key| table.contains_key(*key))
}

fn credential_issuer_value(issuer: crate::provider::auth::CredentialIssuer) -> &'static str {
    match issuer {
        crate::provider::auth::CredentialIssuer::OpenaiChatgpt => "openai_chatgpt",
        crate::provider::auth::CredentialIssuer::GithubCopilot => "github_copilot",
        crate::provider::auth::CredentialIssuer::Gitlab => "gitlab",
        crate::provider::auth::CredentialIssuer::GoogleAdc => "google_adc",
        crate::provider::auth::CredentialIssuer::SapAiCore => "sap_ai_core",
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
            expires_at_ms,
            account_id,
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
            table.insert("expires_at_ms".to_owned(), JsonValue::from(expires_at_ms));
            if let Some(account_id) = normalize_text(account_id.as_deref()) {
                table.insert("account_id".to_owned(), JsonValue::String(account_id));
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
