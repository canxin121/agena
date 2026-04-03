use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use aws_credential_types::Credentials;

use crate::{
    plugin::PluginManager,
    provider::{
        AmazonBedrockProvider, AnthropicProvider, CapabilityOverrideProvider,
        CloudflareAiGatewayProvider, CodexProvider, CopilotProvider,
        CopilotProviderOptions as RuntimeCopilotProviderOptions, GeminiProvider, GitlabProvider,
        GitlabProviderConfig, GoogleVertexProvider, ModelProvider, NamedProvider,
        OpenAiCompatibleProvider, OpenAiProvider, ProviderAliasRegistration, ProviderRegistry,
        auth::{AuthData, AuthStore},
    },
};

use super::{
    BedrockAuthConfig, CloudflareAiGatewayProviderOptions, ConfigEnvironment, ConfigError,
    GoogleVertexAuthConfig, HttpProviderConfig, ProcessEnvironment, ProviderDefinition,
    ResolvedConfig, ResolvedProviderConfig,
};

impl ResolvedConfig {
    pub fn build_provider_registry(&self) -> Result<ProviderRegistry, ConfigError> {
        self.build_provider_registry_with_env(&ProcessEnvironment)
    }

    pub fn build_provider_registry_with_env(
        &self,
        env: &dyn ConfigEnvironment,
    ) -> Result<ProviderRegistry, ConfigError> {
        let client = ProviderRegistry::build_http_client(self.provider_http_client_config())?;
        let auth_store = Arc::new(self.auth_store());
        let auth_snapshot = auth_store.all()?;
        let mut registry = ProviderRegistry::with_runtime_config(self.provider_runtime_config());
        let mut aliases = Vec::new();

        for (provider_id, resolved) in &self.providers {
            if !resolved.enabled {
                continue;
            }

            match &resolved.definition {
                ProviderDefinition::Alias(alias) => aliases.push((
                    provider_id.clone(),
                    alias.clone(),
                    resolved.capability_overrides.clone(),
                )),
                _ => {
                    let provider = build_provider(
                        provider_id.as_str(),
                        resolved,
                        client.clone(),
                        auth_store.clone(),
                        &auth_snapshot,
                        env,
                    )?;
                    registry.register_arc(provider);
                }
            }
        }

        register_aliases(&mut registry, aliases)?;
        Ok(registry)
    }
}

impl super::ConfigResolution {
    pub fn build_plugin_manager(&self) -> Result<PluginManager, ConfigError> {
        let mut manager = PluginManager::new();
        if !self.config.plugins.enabled {
            return Ok(manager);
        }

        let explicit_paths = !self.config.plugins.paths.is_empty();
        let base_dir = self
            .meta
            .config_path
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let paths = if explicit_paths {
            self.config.plugins.paths.clone()
        } else {
            vec![PathBuf::from("plugins")]
        };

        for raw_path in paths {
            let path = if raw_path.is_absolute() {
                raw_path
            } else {
                base_dir.join(raw_path)
            };

            if !path.exists() {
                if explicit_paths {
                    return Err(ConfigError::Validation(format!(
                        "plugin path does not exist: {}",
                        path.display()
                    )));
                }
                continue;
            }

            if path.is_dir() {
                manager.discover_directory(&path).map_err(|err| {
                    ConfigError::Validation(format!(
                        "failed to discover plugins in {}: {err}",
                        path.display()
                    ))
                })?;
            } else {
                manager.load_dynamic(&path).map_err(|err| {
                    ConfigError::Validation(format!(
                        "failed to load plugin {}: {err}",
                        path.display()
                    ))
                })?;
            }
        }

        Ok(manager)
    }
}

fn build_provider(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
    client: reqwest::Client,
    auth_store: Arc<dyn AuthStore>,
    auth_snapshot: &HashMap<String, AuthData>,
    env: &dyn ConfigEnvironment,
) -> Result<Arc<dyn ModelProvider>, ConfigError> {
    let provider = match &resolved.definition {
        ProviderDefinition::Alias(_) => Err(ConfigError::Validation(format!(
            "provider `{provider_id}` alias should be registered in alias phase"
        ))),
        ProviderDefinition::OpenAi(config) => Ok(register_provider(
            provider_id,
            OpenAiProvider::new(
                client,
                required_secret(provider_id, "api_key", config, env)?,
                config.base_url.clone(),
                config.default_model.clone(),
            )
            .with_extra_headers(to_hash_map(&config.extra_headers))
            .with_api_mode(config.options.api_mode.into())
            .with_stream_mode(config.options.stream_mode.into())
            .with_realtime_ws_url(config.options.realtime_ws_url.clone()),
        )),
        ProviderDefinition::OpenAiCompatible(config) => Ok(register_provider(
            provider_id,
            OpenAiCompatibleProvider::new(
                provider_id,
                client,
                required_secret(provider_id, "api_key", config, env)?,
                config.base_url.clone(),
                config.default_model.clone(),
            )
            .with_auth_header(
                config.options.auth_header.clone(),
                config.options.auth_scheme.clone(),
            )
            .with_extra_headers(to_hash_map(&config.extra_headers))
            .with_stream_mode(config.options.stream_mode.into())
            .with_realtime_ws_url(config.options.realtime_ws_url.clone()),
        )),
        ProviderDefinition::Anthropic(config) => Ok(register_provider(
            provider_id,
            AnthropicProvider::new(
                client,
                required_secret(provider_id, "api_key", config, env)?,
                config.base_url.clone(),
                config.default_model.clone(),
            )
            .with_auth_header(
                config.options.auth_header.clone(),
                config.options.auth_scheme.clone(),
            )
            .with_extra_headers(to_hash_map(&config.extra_headers))
            .with_include_thinking(config.options.include_thinking),
        )),
        ProviderDefinition::Gemini(config) => Ok(register_provider(
            provider_id,
            GeminiProvider::new(
                client,
                required_secret(provider_id, "api_key", config, env)?,
                config.base_url.clone(),
                config.default_model.clone(),
            ),
        )),
        ProviderDefinition::Codex(config) => {
            let auth = required_auth(auth_snapshot, config.auth_provider_id.as_str(), provider_id)?;
            Ok(register_provider(
                provider_id,
                CodexProvider::from_auth_with_options(
                    client,
                    auth_store,
                    auth,
                    config.default_model.clone(),
                    config.auth_provider_id.clone(),
                )?,
            ))
        }
        ProviderDefinition::Gitlab(config) => {
            let runtime_config = GitlabProviderConfig {
                instance_url: config.instance_url.clone(),
                ai_gateway_url: config.ai_gateway_url.clone(),
                default_model: config.default_model.clone(),
                ai_gateway_headers: to_hash_map(&config.ai_gateway_headers),
                feature_flags: to_hash_map(&config.feature_flags),
            };

            if let Some(api_key) = optional_secret(
                provider_id,
                "api_key",
                config.api_key.as_ref(),
                config.api_key_env.as_ref(),
                env,
            )? {
                Ok(register_provider(
                    provider_id,
                    GitlabProvider::from_token_with_config(client, api_key, runtime_config)?,
                ))
            } else {
                let auth =
                    required_auth(auth_snapshot, config.auth_provider_id.as_str(), provider_id)?;
                Ok(register_provider(
                    provider_id,
                    GitlabProvider::from_auth_with_config(client, auth, runtime_config)?,
                ))
            }
        }
        ProviderDefinition::Copilot(config) => {
            let auth = required_auth(auth_snapshot, config.auth_provider_id.as_str(), provider_id)?;
            let options = RuntimeCopilotProviderOptions {
                base_url: copilot_base_url(config),
                default_model: Some(config.default_model.clone()),
                models_url: config.models_url.clone(),
            };
            Ok(register_provider(
                provider_id,
                CopilotProvider::from_auth_with_options(provider_id, client, auth, options)?,
            ))
        }
        ProviderDefinition::AmazonBedrock(config) => {
            let provider = match &config.auth {
                BedrockAuthConfig::Bearer {
                    api_key,
                    api_key_env,
                } => AmazonBedrockProvider::new_bearer(
                    client,
                    required_resolved_secret(
                        provider_id,
                        "api_key",
                        api_key.as_ref(),
                        api_key_env.as_ref(),
                        env,
                    )?,
                    config.base_url.clone(),
                    config.default_model.clone(),
                    config.region.clone(),
                ),
                BedrockAuthConfig::Sigv4 {
                    profile,
                    access_key_id,
                    secret_access_key,
                    session_token,
                } => AmazonBedrockProvider::new_sigv4(
                    client,
                    config.base_url.clone(),
                    config.default_model.clone(),
                    config.region.clone(),
                    profile.clone(),
                    static_bedrock_credentials(
                        access_key_id.clone(),
                        secret_access_key.clone(),
                        session_token.clone(),
                        provider_id,
                    )?,
                ),
            };
            Ok(register_provider(provider_id, provider))
        }
        ProviderDefinition::GoogleVertex(config) => {
            let provider = match &config.auth {
                GoogleVertexAuthConfig::StaticToken {
                    access_token,
                    access_token_env,
                } => GoogleVertexProvider::new_static_token(
                    provider_id,
                    client,
                    config.base_url.clone(),
                    config.default_model.clone(),
                    required_resolved_secret(
                        provider_id,
                        "access_token",
                        access_token.as_ref(),
                        access_token_env.as_ref(),
                        env,
                    )?,
                ),
                GoogleVertexAuthConfig::Adc => GoogleVertexProvider::new_adc(
                    provider_id,
                    client,
                    config.base_url.clone(),
                    config.default_model.clone(),
                ),
            };
            Ok(register_provider(provider_id, provider))
        }
        ProviderDefinition::CloudflareAiGateway(config) => Ok(register_provider(
            provider_id,
            build_cloudflare_provider(provider_id, client, config, env)?,
        )),
    }?;

    Ok(apply_capability_overrides(
        provider,
        resolved.capability_overrides.clone(),
    ))
}

fn register_aliases(
    registry: &mut ProviderRegistry,
    mut aliases: Vec<(
        String,
        super::ProviderAliasConfig,
        Vec<crate::provider::ProviderCapabilityOverrideRule>,
    )>,
) -> Result<(), ConfigError> {
    while !aliases.is_empty() {
        let mut remaining = Vec::new();
        let mut progressed = false;

        for (alias_id, alias, capability_overrides) in aliases {
            let Some(_target) = registry.get(alias.target_provider_id.as_str()) else {
                remaining.push((alias_id, alias, capability_overrides));
                continue;
            };

            let mut registration =
                ProviderAliasRegistration::new(alias_id.clone(), alias.target_provider_id.clone());
            if let Some(model) = alias.default_model {
                registration = registration.with_default_model(model);
            }
            if !capability_overrides.is_empty() {
                registration = registration.with_capability_overrides(capability_overrides);
            }
            registry.register_alias(registration).map_err(|err| {
                ConfigError::Validation(format!(
                    "failed to register provider alias `{alias_id}`: {err}"
                ))
            })?;
            progressed = true;
        }

        if !progressed {
            let unresolved = remaining
                .into_iter()
                .map(|(alias_id, alias, _)| {
                    format!("{alias_id}->{target}", target = alias.target_provider_id)
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ConfigError::Validation(format!(
                "unresolved provider aliases: {unresolved}"
            )));
        }

        aliases = remaining;
    }

    Ok(())
}

fn build_cloudflare_provider(
    provider_id: &str,
    client: reqwest::Client,
    config: &CloudflareAiGatewayProviderOptions,
    env: &dyn ConfigEnvironment,
) -> Result<CloudflareAiGatewayProvider, ConfigError> {
    let inner = OpenAiCompatibleProvider::new(
        provider_id,
        client,
        required_resolved_secret(
            provider_id,
            "api_key",
            config.api_key.as_ref(),
            config.api_key_env.as_ref(),
            env,
        )?,
        config.base_url.clone(),
        config.default_model.clone(),
    );
    Ok(CloudflareAiGatewayProvider::new(inner))
}

fn required_secret(
    provider_id: &str,
    field: &'static str,
    config: &HttpProviderConfig<impl Clone>,
    env: &dyn ConfigEnvironment,
) -> Result<String, ConfigError> {
    required_resolved_secret(
        provider_id,
        field,
        config.api_key.as_ref(),
        config.api_key_env.as_ref(),
        env,
    )
}

fn required_resolved_secret(
    provider_id: &str,
    field: &'static str,
    direct: Option<&String>,
    env_key: Option<&String>,
    env: &dyn ConfigEnvironment,
) -> Result<String, ConfigError> {
    optional_secret(provider_id, field, direct, env_key, env)?.ok_or_else(|| {
        ConfigError::MissingProviderField {
            provider_id: provider_id.to_owned(),
            field,
        }
    })
}

fn optional_secret(
    provider_id: &str,
    field: &'static str,
    direct: Option<&String>,
    env_key: Option<&String>,
    env: &dyn ConfigEnvironment,
) -> Result<Option<String>, ConfigError> {
    if let Some(value) = direct.and_then(|value| normalize_text(value)) {
        return Ok(Some(value));
    }

    let Some(env_key) = env_key.and_then(|value| normalize_text(value)) else {
        return Ok(None);
    };

    let Some(value) = env
        .var(env_key.as_str())
        .and_then(|value| normalize_text(&value))
    else {
        return Err(ConfigError::MissingEnvironmentVariable {
            provider_id: provider_id.to_owned(),
            field,
            env_key,
        });
    };

    Ok(Some(value))
}

fn required_auth<'a>(
    auth_snapshot: &'a HashMap<String, AuthData>,
    auth_provider_id: &str,
    provider_id: &str,
) -> Result<&'a AuthData, ConfigError> {
    auth_snapshot.get(auth_provider_id).ok_or_else(|| {
        ConfigError::Validation(format!(
            "provider `{provider_id}` requires auth credential `{auth_provider_id}`"
        ))
    })
}

fn static_bedrock_credentials(
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
    provider_id: &str,
) -> Result<Option<Credentials>, ConfigError> {
    match (
        access_key_id.and_then(|value| normalize_text(&value)),
        secret_access_key.and_then(|value| normalize_text(&value)),
    ) {
        (Some(access_key_id), Some(secret_access_key)) => Ok(Some(Credentials::new(
            access_key_id,
            secret_access_key,
            session_token.and_then(|value| normalize_text(&value)),
            None,
            "agena-config",
        ))),
        (None, None) => Ok(None),
        _ => Err(ConfigError::InvalidProviderConfig {
            provider_id: provider_id.to_owned(),
            message: "access_key_id and secret_access_key must be provided together".to_owned(),
        }),
    }
}

fn copilot_base_url(config: &super::CopilotProviderOptions) -> Option<String> {
    if config.auth_provider_id == "github-copilot-enterprise"
        && config.base_url == "https://api.githubcopilot.com"
    {
        None
    } else {
        Some(config.base_url.clone())
    }
}

fn register_provider<P>(provider_id: &str, provider: P) -> Arc<dyn ModelProvider>
where
    P: ModelProvider + 'static,
{
    let native_id = provider.id().to_owned();
    let provider: Arc<dyn ModelProvider> = Arc::new(provider);
    if native_id == provider_id {
        provider
    } else {
        Arc::new(NamedProvider::new(provider_id.to_owned(), provider))
    }
}

fn apply_capability_overrides(
    provider: Arc<dyn ModelProvider>,
    rules: Vec<crate::provider::ProviderCapabilityOverrideRule>,
) -> Arc<dyn ModelProvider> {
    CapabilityOverrideProvider::new(provider, rules)
}

fn normalize_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn to_hash_map<K, V>(map: &std::collections::BTreeMap<K, V>) -> HashMap<K, V>
where
    K: Clone + Eq + std::hash::Hash,
    V: Clone,
{
    map.iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::config::{ConfigEnvironment, ConfigLoader, LoadConfigRequest};
    use crate::provider::CapabilitySupport;

    #[derive(Clone, Default)]
    struct TestEnvironment {
        vars: BTreeMap<String, String>,
    }

    impl ConfigEnvironment for TestEnvironment {
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }

        fn vars(&self) -> Vec<(String, String)> {
            self.vars
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        }
    }

    #[test]
    fn registry_builder_resolves_env_secret_and_alias() {
        let path = write_temp_file(
            r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
api_key_env = "OPENAI_API_KEY"

[providers.prod]
kind = "alias"
target_provider_id = "openai"
default_model = "gpt-5"
"#,
        );

        let env = TestEnvironment {
            vars: BTreeMap::from([("OPENAI_API_KEY".to_owned(), "sk-test".to_owned())]),
        };
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");
        let registry = resolution
            .config
            .build_provider_registry_with_env(&env)
            .expect("registry should build");

        let ids = registry.provider_ids();
        assert!(ids.iter().any(|id| id == "openai"));
        assert!(ids.iter().any(|id| id == "prod"));
    }

    #[test]
    fn registry_builder_applies_capability_overrides_to_alias_models() {
        let path = write_temp_file(
            r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
api_key_env = "OPENAI_API_KEY"

[providers.prod]
kind = "alias"
target_provider_id = "openai"
default_model = "gpt-5"

[[providers.prod.capability_overrides]]
model = "gpt-5"
image_input = "unsupported"
"#,
        );

        let env = TestEnvironment {
            vars: BTreeMap::from([("OPENAI_API_KEY".to_owned(), "sk-test".to_owned())]),
        };
        let loader = ConfigLoader::new(env.clone());
        let resolution = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");
        let registry = resolution
            .config
            .build_provider_registry_with_env(&env)
            .expect("registry should build");

        let capabilities = registry
            .model_capabilities("prod", "")
            .expect("aliased provider capabilities should resolve");
        assert_eq!(capabilities.image_input, CapabilitySupport::Unsupported);
        assert_eq!(capabilities.document_input, CapabilitySupport::Supported);
    }

    fn write_temp_file(content: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("agena-config-registry-{suffix}.toml"));
        fs::write(&path, content).expect("temp file should be written");
        path
    }
}
