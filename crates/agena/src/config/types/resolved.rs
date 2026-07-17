use super::{
    AgentConfig, BTreeMap, ConfigError, ConfigOutputFormat, ExecutionSelection, HarnessesConfig,
    PathBuf, PluginConfig, ProviderNativeToolBinding, ResolvedProviderConfig, RuntimeConfig,
    Serialize, SessionConfig, TracingConfig, UiConfig,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Default,
    File,
    Project,
    Environment,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppliedLayer {
    pub source: ConfigSource,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigResolutionMeta {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub project_config_path: PathBuf,
    pub project_config_found: bool,
    pub applied_layers: Vec<AppliedLayer>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfigResolution {
    pub config: ResolvedConfig,
    pub meta: ConfigResolutionMeta,
}

impl ConfigResolution {
    pub fn render(&self, format: ConfigOutputFormat) -> Result<String, ConfigError> {
        match format {
            ConfigOutputFormat::Json => Ok(serde_json::to_string_pretty(self)?),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedConfig {
    #[serde(skip_serializing)]
    pub default_selection: ExecutionSelection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
    pub tracing: TracingConfig,
    pub ui: UiConfig,
    pub runtime: RuntimeConfig,
    pub session: SessionConfig,
    #[serde(
        default,
        skip_serializing_if = "crate::agent::PermissionConfig::is_empty"
    )]
    pub permission: crate::agent::PermissionConfig,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, AgentConfig>,
    pub plugins: PluginConfig,
    #[serde(default, skip_serializing_if = "HarnessesConfig::is_empty")]
    pub harnesses: HarnessesConfig,
    pub providers: BTreeMap<String, ResolvedProviderConfig>,
}

impl ResolvedConfig {
    pub fn plugin_storage(&self) -> std::sync::Arc<dyn crate::plugins::storage::PluginStorage> {
        std::sync::Arc::new(crate::plugins::storage::FilePluginStorage::new(
            crate::plugins::storage::default_storage_root(),
        ))
    }

    pub fn plugin_secret_store(
        &self,
    ) -> std::sync::Arc<dyn crate::plugins::storage::PluginSecretStore> {
        std::sync::Arc::new(crate::plugins::storage::PluginKeyringSecretStore::system(
            crate::plugins::storage::default_storage_root(),
            true,
        ))
    }

    pub fn provider_model_tool_bindings(
        &self,
        provider_id: &str,
    ) -> Option<BTreeMap<String, Vec<ProviderNativeToolBinding>>> {
        self.providers.get(provider_id).map(|provider| {
            provider
                .models
                .iter()
                .filter_map(|(route, model)| {
                    let bindings = model.provider_native_tool_bindings();
                    (!bindings.is_empty()).then(|| (route.clone(), bindings))
                })
                .collect()
        })
    }
}
