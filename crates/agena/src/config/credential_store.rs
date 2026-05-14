use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use toml_edit::{DocumentMut, InlineTable, Item, Table, Value, value};

use crate::{
    config::{ConfigLoader, LoadConfigRequest, ProcessEnvironment},
    error::AppError,
    provider::auth::{AuthData, AuthStore},
};

use super::{ProviderApiAuthConfig, ProviderAuthConfig, ResolvedProviderConfig};

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
        Ok(load_provider_configs(self.config_path.as_path())?)
    }

    fn read_doc(&self) -> Result<DocumentMut, AppError> {
        if self.config_path.exists() {
            let text = fs::read_to_string(&self.config_path)?;
            let doc = text.parse::<DocumentMut>().map_err(|err| {
                AppError::Config(format!("parse {}: {err}", self.config_path.display()))
            })?;
            Ok(doc)
        } else {
            Ok(DocumentMut::new())
        }
    }

    fn write_doc(&self, doc: &DocumentMut) -> Result<(), AppError> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.config_path, doc.to_string())?;
        Ok(())
    }

    fn ensure_provider_auth_table<'a>(
        &self,
        doc: &'a mut DocumentMut,
        provider_id: &str,
    ) -> &'a mut Table {
        if !doc.contains_key("providers") || !doc["providers"].is_table() {
            doc["providers"] = Item::Table(Table::new());
        }

        let providers = doc["providers"].as_table_mut().expect("providers table");
        if !providers.contains_key(provider_id) || !providers[provider_id].is_table() {
            providers[provider_id] = Item::Table(Table::new());
        }

        let provider = providers[provider_id]
            .as_table_mut()
            .expect("provider table");
        if !provider.contains_key("auth") || !provider["auth"].is_table() {
            provider["auth"] = Item::Table(Table::new());
        }

        provider["auth"]
            .as_table_mut()
            .expect("provider auth table")
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
                auth_table["mode"] = value("api");
                auth_table["api_key"] = value(key);
            }
            AuthData::OAuth { .. } => {
                let issuer = auth.issuer().ok_or_else(|| {
                    AppError::Config(format!(
                        "{provider_id} oauth credential must include an issuer"
                    ))
                })?;
                auth_table["mode"] = value("credential");
                auth_table["issuer"] = value(credential_issuer_value(issuer));
                auth_table["credential"] = auth_data_item(auth);
            }
            AuthData::WellKnown { .. } => {
                auth_table["mode"] = value("credential");
                auth_table["credential"] = auth_data_item(auth);
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
            .as_table_mut()
            .get_mut("providers")
            .and_then(Item::as_table_mut)
        else {
            return Ok(());
        };
        let Some(provider) = providers.get_mut(provider_id).and_then(Item::as_table_mut) else {
            return Ok(());
        };
        let Some(auth) = provider.get_mut("auth").and_then(Item::as_table_mut) else {
            return Ok(());
        };

        auth.remove("credential");
        auth.remove("issuer");
        auth.remove("api_key");

        if auth_table_has_no_sources(auth) {
            auth.remove("mode");
        }

        if auth.iter().next().is_none() {
            provider.remove("auth");
        }

        self.write_doc(&doc)
    }
}

pub fn provider_auth_data(resolved: &ResolvedProviderConfig) -> Option<AuthData> {
    match &resolved.auth {
        ProviderAuthConfig::None
        | ProviderAuthConfig::GoogleAdc
        | ProviderAuthConfig::BedrockSigv4(_) => None,
        ProviderAuthConfig::Api(api) => secret_auth_data(api),
        ProviderAuthConfig::Credential(config) => config.credential.clone(),
        ProviderAuthConfig::SapAiCore(config) => secret_auth_data(&config.api),
    }
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
    resolved.adapters.values().any(|adapter| {
        matches!(
            adapter.definition,
            super::ProviderAdapterDefinition::Gitlab(_)
        )
    })
}

pub fn provider_gitlab_instance_url(resolved: &ResolvedProviderConfig) -> Option<String> {
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
        ProviderAuthConfig::SapAiCore(_) => true,
        ProviderAuthConfig::Api(_) => {
            !provider_supports_openai_oauth(resolved) && !provider_supports_copilot_device(resolved)
        }
        ProviderAuthConfig::Credential(_) => false,
        ProviderAuthConfig::None
        | ProviderAuthConfig::GoogleAdc
        | ProviderAuthConfig::BedrockSigv4(_) => false,
    }
}

fn load_provider_configs(path: &Path) -> Result<HashMap<String, ResolvedProviderConfig>, AppError> {
    let loader = ConfigLoader::new(ProcessEnvironment);
    let resolution = loader.load(&LoadConfigRequest {
        config_path: Some(path.to_path_buf()),
        ..LoadConfigRequest::default()
    })?;
    Ok(resolution.config.providers.into_iter().collect())
}

fn secret_auth_data(secret: &ProviderApiAuthConfig) -> Option<AuthData> {
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

fn auth_table_has_no_sources(table: &Table) -> bool {
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

    !source_keys.iter().any(|key| table.contains_key(key))
}

fn credential_issuer_value(issuer: crate::provider::auth::CredentialIssuer) -> &'static str {
    match issuer {
        crate::provider::auth::CredentialIssuer::OpenaiChatgpt => "openai_chatgpt",
        crate::provider::auth::CredentialIssuer::GithubCopilot => "github_copilot",
        crate::provider::auth::CredentialIssuer::Gitlab => "gitlab",
    }
}

fn auth_data_item(auth: AuthData) -> Item {
    let mut table = InlineTable::new();
    match auth {
        AuthData::Api { key } => {
            table.insert("type", Value::from("api"));
            table.insert("key", Value::from(key));
        }
        AuthData::OAuth {
            issuer,
            refresh,
            access,
            expires_at_ms,
            account_id,
            enterprise_url,
        } => {
            table.insert("type", Value::from("oauth"));
            if let Some(issuer) = issuer {
                table.insert("issuer", Value::from(credential_issuer_value(issuer)));
            }
            table.insert("refresh", Value::from(refresh));
            table.insert("access", Value::from(access));
            table.insert("expires_at_ms", Value::from(expires_at_ms));
            if let Some(account_id) = normalize_text(account_id.as_deref()) {
                table.insert("account_id", Value::from(account_id));
            }
            if let Some(enterprise_url) = normalize_text(enterprise_url.as_deref()) {
                table.insert("enterprise_url", Value::from(enterprise_url));
            }
        }
        AuthData::WellKnown { key, token } => {
            table.insert("type", Value::from("well_known"));
            table.insert("key", Value::from(key));
            table.insert("token", Value::from(token));
        }
    }
    Item::Value(Value::InlineTable(table))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_config_path() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("agena-provider-auth-store-{suffix}.toml"))
    }

    #[test]
    fn config_store_writes_inline_api_credential() {
        let path = temp_config_path();
        fs::write(
            &path,
            r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.adapters.openai]
enabled = true
"#,
        )
        .expect("config should be written");

        let store = ProviderConfigCredentialStore::new(path.clone());
        store
            .set(
                "openai",
                AuthData::Api {
                    key: "sk-test".to_owned(),
                },
            )
            .expect("config store should persist api key");

        let text = fs::read_to_string(&path).expect("config should be readable");
        assert!(text.contains("[providers.openai.auth]"));
        assert!(text.contains("mode = \"api\""));
        assert!(text.contains("api_key = \"sk-test\""));
    }

    #[test]
    fn config_store_get_prefers_inline_credential_over_empty_secret_env() {
        let path = temp_config_path();
        fs::write(
            &path,
            r#"
[providers.openai_chatgpt]
default_model = "openai/gpt-5.3-codex"

[providers.openai_chatgpt.auth]
mode = "credential"
issuer = "openai_chatgpt"
credential = { type = "oauth", issuer = "openai_chatgpt", refresh = "refresh", access = "access", expires_at_ms = 123 }

[providers.openai_chatgpt.adapters.openai]
enabled = true
backend = "chatgpt_codex"
"#,
        )
        .expect("config should be written");

        let store = ProviderConfigCredentialStore::new(path);
        let auth = store
            .get("openai_chatgpt")
            .expect("store read should succeed");
        assert!(matches!(
            auth,
            Some(AuthData::OAuth {
                refresh,
                access,
                expires_at_ms: 123,
                ..
            }) if refresh == "refresh" && access == "access"
        ));
    }

    #[test]
    fn config_store_remove_only_clears_inline_credential() {
        let path = temp_config_path();
        fs::write(
            &path,
            r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.auth]
mode = "api"
api_key_env = "OPENAI_API_KEY"
api_key = "sk-inline"
"#,
        )
        .expect("config should be written");

        let store = ProviderConfigCredentialStore::new(path.clone());
        store
            .remove("openai")
            .expect("credential removal should succeed");

        let text = fs::read_to_string(path).expect("config should be readable");
        assert!(text.contains("api_key_env = \"OPENAI_API_KEY\""));
        assert!(!text.contains("sk-inline"));
    }
}
