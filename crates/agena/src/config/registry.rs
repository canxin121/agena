use std::{collections::HashMap, path::PathBuf, sync::Arc};

use aws_credential_types::Credentials;

use crate::{
    model::ModelId,
    plugin::{PluginHost, PluginHostBuilder},
    provider::{
        AmazonBedrockProvider, AnthropicProvider, AuthRefreshStrategy, AuthSecretSelector,
        CapabilityOverrideProvider, CloudflareAiGatewayProvider, CodexProvider, CopilotProvider,
        CopilotProviderOptions as RuntimeCopilotProviderOptions, GeminiProvider, GitlabProvider,
        GitlabProviderConfig, GoogleVertexProvider, ManagedCredential, ModelProvider,
        NamedProvider, OllamaProvider, OpenAiCompatibleProvider, OpenAiProvider, OpencodeProvider,
        ProviderAliasRegistration, ProviderRegistry,
        auth::{AuthData, AuthStore},
        parse_sap_ai_core_service_key,
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
    pub async fn build_provider_registry_with_plugins(
        &self,
        plugins: &PluginHost,
    ) -> Result<ProviderRegistry, ConfigError> {
        let mut registry = self.config.build_provider_registry()?;
        if plugins.is_empty() {
            return Ok(registry);
        }

        let current = registry
            .provider_ids()
            .into_iter()
            .map(|id| crate::plugin::ProviderDescriptor {
                display_name: id.clone(),
                id,
                models: Vec::new(),
                endpoint: None,
                kind: crate::plugin::ProviderKind::Custom,
            })
            .collect();
        let patch = plugins
            .dispatch_provider_list(crate::plugin::ProviderListInput { current })
            .await
            .map_err(|err| ConfigError::Validation(format!("plugin provider.list: {err}")))?;
        for provider_id in patch.remove {
            registry.remove(provider_id.as_str());
        }
        for descriptor in patch.add {
            registry.register_plugin_provider(descriptor)?;
        }
        Ok(registry)
    }

    pub async fn build_plugin_host(&self) -> Result<Arc<PluginHost>, ConfigError> {
        self.build_plugin_host_with_previous_and_mcp(None, None, None)
            .await
    }

    /// Hot-reload-aware build: when a previous plugin host (and its config)
    /// is available, transports for byte-identical entries are reused, so
    /// stdio subprocesses and HTTP plugins survive a config reload that
    /// didn't touch them.
    pub async fn build_plugin_host_with_previous(
        &self,
        previous_host: Option<Arc<PluginHost>>,
        previous_config: Option<&agena_plugin_host::PluginsConfig>,
    ) -> Result<Arc<PluginHost>, ConfigError> {
        self.build_plugin_host_with_previous_and_mcp(previous_host, previous_config, None)
            .await
    }

    pub async fn build_plugin_host_with_previous_and_mcp(
        &self,
        previous_host: Option<Arc<PluginHost>>,
        previous_config: Option<&agena_plugin_host::PluginsConfig>,
        mcp_manager: Option<Arc<agena_mcp_client::McpConnectionManager>>,
    ) -> Result<Arc<PluginHost>, ConfigError> {
        let workspace_root = self
            .meta
            .config_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let agena_version = env!("CARGO_PKG_VERSION").to_string();
        let mut plugin_config = self.config.plugins.clone();
        if mcp_manager.is_some() {
            plugin_config.list.insert(
                crate::tool::mcp_plugin_id().to_string(),
                crate::plugin::PluginEntry::Static {
                    options: serde_json::to_value(&self.config.mcp)
                        .unwrap_or(serde_json::Value::Null),
                    timeouts: Default::default(),
                },
            );
        }
        let mut builder = PluginHostBuilder::new(workspace_root, agena_version)
            .with_config(plugin_config)
            .register_static(
                crate::tool::builtins_plugin_id(),
                crate::tool::new_builtins_plugin(),
            )
            .register_static(
                crate::tool::skills_plugin_id(),
                crate::tool::new_skills_plugin(),
            )
            .register_static(crate::tool::lsp_plugin_id(), crate::tool::new_lsp_plugin())
            .register_static(
                crate::tool::cron_plugin_id(),
                crate::tool::new_cron_plugin(),
            )
            .register_static(crate::tool::fs_plugin_id(), crate::tool::new_fs_plugin())
            .register_static(
                crate::tool::shell_plugin_id(),
                crate::tool::new_shell_plugin(),
            )
            .register_static(crate::tool::web_plugin_id(), crate::tool::new_web_plugin())
            .register_static(
                crate::tool::workflow_plugin_id(),
                crate::tool::new_workflow_plugin(),
            )
            .register_static(
                "agena-memory",
                crate::memory::new_memory_plugin(self.config.memory.clone()),
            )
            .register_static(
                crate::hooks::ShellHookPlugin::id(),
                crate::hooks::ShellHookPlugin::new(self.config.hooks.clone()),
            );
        if let Some(manager) = mcp_manager {
            builder = builder.register_static(
                crate::tool::mcp_plugin_id(),
                crate::tool::new_mcp_plugin(manager),
            );
        }
        if let (Some(prev_host), Some(prev_cfg)) = (previous_host, previous_config) {
            builder = builder.with_previous(prev_host, prev_cfg);
        }
        builder
            .build()
            .await
            .map_err(|e| ConfigError::Validation(format!("plugin host: {e}")))
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
        ProviderDefinition::Ollama(config) => Ok(register_provider(
            provider_id,
            OllamaProvider::new(
                provider_id,
                client,
                config.base_url.clone(),
                config.default_model.clone(),
            ),
        )),
        ProviderDefinition::OpenAi(config) => Ok(register_provider(
            provider_id,
            OpenAiProvider::new_managed(
                client,
                required_managed_secret(
                    provider_id,
                    "api_key",
                    config.api_key.as_ref(),
                    config.api_key_env.as_ref(),
                    env,
                )?,
                config.base_url.clone(),
                config.default_model.clone(),
            )
            .with_extra_headers(to_hash_map(&config.extra_headers))
            .with_api_mode(config.options.api_mode.into())
            .with_stream_mode(config.options.stream_mode.into())
            .with_realtime_ws_url(config.options.realtime_ws_url.clone())
            .with_default_thinking(config.default_thinking.clone()),
        )),
        ProviderDefinition::OpenAiCompatible(config) => {
            let credential = required_managed_secret(
                provider_id,
                "api_key",
                config.api_key.as_ref(),
                config.api_key_env.as_ref(),
                env,
            )?;
            let extra_headers = to_hash_map(&config.extra_headers);
            if matches!(provider_id, "opencode" | "opencode-go") {
                Ok(register_provider(
                    provider_id,
                    OpencodeProvider::new(
                        provider_id,
                        client,
                        credential,
                        config.base_url.clone(),
                        config.default_model.clone(),
                        config.options.auth_header.clone(),
                        config.options.auth_scheme.clone(),
                        extra_headers,
                        config.options.stream_mode.into(),
                        config.options.realtime_ws_url.clone(),
                    ),
                ))
            } else {
                Ok(register_provider(
                    provider_id,
                    OpenAiCompatibleProvider::new_managed(
                        provider_id,
                        client,
                        credential,
                        config.base_url.clone(),
                        config.default_model.clone(),
                    )
                    .with_auth_header(
                        config.options.auth_header.clone(),
                        config.options.auth_scheme.clone(),
                    )
                    .with_extra_headers(extra_headers)
                    .with_stream_mode(config.options.stream_mode.into())
                    .with_realtime_ws_url(config.options.realtime_ws_url.clone())
                    .with_default_thinking(config.default_thinking.clone()),
                ))
            }
        }
        ProviderDefinition::SapAiCore(config) => Ok(register_provider(
            provider_id,
            build_sap_ai_core_provider(provider_id, client, config, env)?,
        )),
        ProviderDefinition::Anthropic(config) => Ok(register_provider(
            provider_id,
            AnthropicProvider::new_managed(
                client,
                required_managed_secret(
                    provider_id,
                    "api_key",
                    config.api_key.as_ref(),
                    config.api_key_env.as_ref(),
                    env,
                )?,
                config.base_url.clone(),
                config.default_model.clone(),
            )
            .with_auth_header(
                config.options.auth_header.clone(),
                config.options.auth_scheme.clone(),
            )
            .with_extra_headers(to_hash_map(&config.extra_headers))
            .with_default_thinking(config.default_thinking.clone()),
        )),
        ProviderDefinition::Gemini(config) => Ok(register_provider(
            provider_id,
            GeminiProvider::new_managed(
                client,
                required_managed_secret(
                    provider_id,
                    "api_key",
                    config.api_key.as_ref(),
                    config.api_key_env.as_ref(),
                    env,
                )?,
                config.base_url.clone(),
                config.default_model.clone(),
            )
            .with_extra_headers(to_hash_map(&config.extra_headers))
            .with_default_thinking(config.default_thinking.clone()),
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

            if has_resolved_secret(
                provider_id,
                "api_key",
                config.api_key.as_ref(),
                config.api_key_env.as_ref(),
                env,
            )? {
                Ok(register_provider(
                    provider_id,
                    GitlabProvider::from_managed_token_with_config(
                        client,
                        required_managed_secret(
                            provider_id,
                            "api_key",
                            config.api_key.as_ref(),
                            config.api_key_env.as_ref(),
                            env,
                        )?,
                        runtime_config,
                    )?,
                ))
            } else {
                Ok(register_provider(
                    provider_id,
                    build_gitlab_auth_provider(
                        provider_id,
                        client,
                        auth_store,
                        auth_snapshot,
                        config,
                        runtime_config,
                    )?,
                ))
            }
        }
        ProviderDefinition::Copilot(config) => {
            let auth = required_auth(auth_snapshot, config.auth_provider_id.as_str(), provider_id)?;
            let options = RuntimeCopilotProviderOptions {
                base_url: copilot_base_url(config),
                default_model: Some(ModelId::new(config.default_model.clone())),
                models_url: config.models_url.clone(),
            };
            Ok(register_provider(
                provider_id,
                CopilotProvider::with_bearer_credential(
                    provider_id,
                    client,
                    ManagedCredential::auth_store(
                        format!("{provider_id} bearer token"),
                        auth_store,
                        config.auth_provider_id.clone(),
                        AuthSecretSelector::RefreshOrAccess,
                        AuthRefreshStrategy::ReloadFromStore,
                    ),
                    auth.enterprise_url().map(ToOwned::to_owned),
                    options,
                )?,
            ))
        }
        ProviderDefinition::AmazonBedrock(config) => {
            let provider = match &config.auth {
                BedrockAuthConfig::Bearer {
                    api_key,
                    api_key_env,
                } => AmazonBedrockProvider::new_managed_bearer(
                    client,
                    required_managed_secret(
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
                } => GoogleVertexProvider::new_managed_token(
                    provider_id,
                    client,
                    config.base_url.clone(),
                    config.default_model.clone(),
                    required_managed_secret(
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
    let inner = OpenAiCompatibleProvider::new_managed(
        provider_id,
        client,
        required_managed_secret(
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

fn build_sap_ai_core_provider(
    provider_id: &str,
    client: reqwest::Client,
    config: &HttpProviderConfig<super::OpenAiCompatibleProviderOptions>,
    env: &dyn ConfigEnvironment,
) -> Result<OpenAiCompatibleProvider, ConfigError> {
    let credential = if has_resolved_secret(
        provider_id,
        "api_key",
        config.api_key.as_ref(),
        config.api_key_env.as_ref(),
        env,
    )? {
        required_managed_secret(
            provider_id,
            "api_key",
            config.api_key.as_ref(),
            config.api_key_env.as_ref(),
            env,
        )?
    } else {
        let service_key_raw = env
            .var("AICORE_SERVICE_KEY")
            .and_then(|value| normalize_text(&value))
            .ok_or_else(|| ConfigError::InvalidProviderConfig {
                provider_id: provider_id.to_owned(),
                message:
                    "sap-ai-core requires `AICORE_SERVICE_KEY` when `api_key` is not configured"
                        .to_owned(),
            })?;
        let service_key =
            parse_sap_ai_core_service_key(service_key_raw.as_str()).map_err(|err| {
                ConfigError::InvalidProviderConfig {
                    provider_id: provider_id.to_owned(),
                    message: format!("failed to parse `AICORE_SERVICE_KEY`: {err}"),
                }
            })?;
        ManagedCredential::sap_ai_core(
            format!("{provider_id} sap ai core token"),
            client.clone(),
            provider_id.to_owned(),
            service_key,
        )
    };

    Ok(OpenAiCompatibleProvider::new_managed(
        provider_id,
        client,
        credential,
        config.base_url.clone(),
        config.default_model.clone(),
    )
    .with_auth_header(
        config.options.auth_header.clone(),
        config.options.auth_scheme.clone(),
    )
    .with_extra_headers(to_hash_map(&config.extra_headers))
    .with_stream_mode(config.options.stream_mode.into())
    .with_realtime_ws_url(config.options.realtime_ws_url.clone()))
}

fn build_gitlab_auth_provider(
    provider_id: &str,
    client: reqwest::Client,
    auth_store: Arc<dyn AuthStore>,
    auth_snapshot: &HashMap<String, AuthData>,
    config: &super::GitlabProviderOptions,
    runtime_config: GitlabProviderConfig,
) -> Result<GitlabProvider, ConfigError> {
    let _ = required_auth(auth_snapshot, config.auth_provider_id.as_str(), provider_id)?;
    GitlabProvider::from_managed_token_with_config(
        client,
        ManagedCredential::auth_store(
            format!("{provider_id} gitlab access token"),
            auth_store,
            config.auth_provider_id.clone(),
            AuthSecretSelector::AccessOrApiKey,
            AuthRefreshStrategy::GitlabOAuth {
                instance_url: config.instance_url.clone(),
            },
        ),
        runtime_config,
    )
    .map_err(ConfigError::from)
}

fn has_resolved_secret(
    provider_id: &str,
    field: &'static str,
    direct: Option<&String>,
    env_key: Option<&String>,
    env: &dyn ConfigEnvironment,
) -> Result<bool, ConfigError> {
    match (
        direct.and_then(|value| normalize_text(value)),
        env_key.and_then(|value| normalize_text(value)),
    ) {
        (Some(_), _) => Ok(true),
        (None, Some(env_key)) => {
            if env
                .var(env_key.as_str())
                .and_then(|value| normalize_text(&value))
                .is_some()
            {
                Ok(true)
            } else {
                Err(ConfigError::MissingEnvironmentVariable {
                    provider_id: provider_id.to_owned(),
                    field,
                    env_key,
                })
            }
        }
        (None, None) => Ok(false),
    }
}

fn required_managed_secret(
    provider_id: &str,
    field: &'static str,
    direct: Option<&String>,
    env_key: Option<&String>,
    env: &dyn ConfigEnvironment,
) -> Result<ManagedCredential, ConfigError> {
    if let Some(value) = direct.and_then(|value| normalize_text(value)) {
        return Ok(ManagedCredential::static_value(
            format!("{provider_id} {field}"),
            value,
        ));
    }

    let Some(env_key) = env_key.and_then(|value| normalize_text(value)) else {
        return Err(ConfigError::MissingProviderField {
            provider_id: provider_id.to_owned(),
            field,
        });
    };

    if env
        .var(env_key.as_str())
        .and_then(|value| normalize_text(&value))
        .is_none()
    {
        return Err(ConfigError::MissingEnvironmentVariable {
            provider_id: provider_id.to_owned(),
            field,
            env_key,
        });
    }

    Ok(ManagedCredential::environment(
        format!("{provider_id} {field}"),
        provider_id.to_owned(),
        field,
        env_key,
    ))
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
            .model_capabilities(&crate::model::ModelRef::new("prod", "gpt-5"))
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
