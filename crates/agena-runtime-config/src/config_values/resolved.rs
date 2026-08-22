use super::{
    BTreeMap, HarnessesConfig, PathBuf, ResolvedProviderConfig, RuntimeConfig, Serialize,
    SessionConfig, UiConfig,
};
use crate::RuntimeTracingConfiguration;
use agena_domain::{ExecutionSelection, PermissionConfig};
use agena_plugin_host::PluginsConfig as PluginConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
/// Source of a configuration layer.
pub enum ConfigSource {
    Default,
    File,
    Project,
    Environment,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// One applied configuration layer and its source.
pub struct AppliedLayer {
    pub source: ConfigSource,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Metadata about how a configuration was resolved.
pub struct ConfigResolutionMeta {
    pub config_path: PathBuf,
    pub config_found: bool,
    pub project_config_path: PathBuf,
    pub project_config_found: bool,
    pub applied_layers: Vec<AppliedLayer>,
}

impl ConfigResolutionMeta {
    /// Build the stable layer/provenance projection from the concrete loader's
    /// source-presence facts. Runtime owns the order and wire descriptions;
    /// schema adapters only report which layers were present.
    pub fn from_layer_presence(
        config_path: PathBuf,
        config_found: bool,
        project_config_path: PathBuf,
        project_config_found: bool,
        environment_applied: bool,
        cli_override_count: usize,
    ) -> Self {
        let mut applied_layers = vec![AppliedLayer {
            source: ConfigSource::Default,
            description: "built-in defaults".to_owned(),
        }];
        if config_found {
            applied_layers.push(AppliedLayer {
                source: ConfigSource::File,
                description: format!("file:{}", config_path.display()),
            });
        }
        if project_config_found {
            applied_layers.push(AppliedLayer {
                source: ConfigSource::Project,
                description: format!("project:{}", project_config_path.display()),
            });
        }
        if environment_applied {
            applied_layers.push(AppliedLayer {
                source: ConfigSource::Environment,
                description: "process environment".to_owned(),
            });
        }
        if cli_override_count > 0 {
            applied_layers.push(AppliedLayer {
                source: ConfigSource::Cli,
                description: format!("{cli_override_count} cli override(s)"),
            });
        }
        Self {
            config_path,
            config_found,
            project_config_path,
            project_config_found,
            applied_layers,
        }
    }

    pub fn applied_layer_descriptions(&self) -> Vec<String> {
        self.applied_layers
            .iter()
            .map(|layer| layer.description.clone())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
/// A resolved configuration together with its metadata.
pub struct ConfigResolution {
    pub config: ResolvedConfig,
    pub meta: ConfigResolutionMeta,
}

/// Serialize the complete resolved configuration document at the Runtime
/// boundary. Callers should not reconstruct this value from concrete schema types.
pub fn config_resolution_json_value(
    resolution: &ConfigResolution,
) -> Result<serde_json::Value, serde_json::Error> {
    let mut value = serde_json::to_value(resolution)?;
    inject_resolved_default_selection(&mut value, &resolution.config.default_selection);
    Ok(value)
}

pub fn resolved_config_json_value(
    config: &ResolvedConfig,
) -> Result<serde_json::Value, serde_json::Error> {
    let mut value = serde_json::to_value(config)?;
    inject_resolved_default_selection(&mut value, &config.default_selection);
    Ok(value)
}

/// Re-materialize the typed global model selection into the effective JSON
/// document. It is kept out of the concrete `ResolvedConfig` serialization so
/// runtime consumers use the typed value while settings clients can continue
/// to read the stable `providers.default_selection` path.
fn inject_resolved_default_selection(
    value: &mut serde_json::Value,
    selection: &ExecutionSelection,
) {
    let providers = match value {
        serde_json::Value::Object(root) if root.contains_key("providers") => {
            root.get_mut("providers")
        }
        serde_json::Value::Object(root) => root
            .get_mut("config")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|config| config.get_mut("providers")),
        _ => None,
    };
    let Some(providers) = providers.and_then(serde_json::Value::as_object_mut) else {
        return;
    };

    if selection.is_empty() {
        return;
    }

    let mut default_selection = serde_json::Map::new();
    if let Some(provider) = selection
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        default_selection.insert(
            "provider".to_owned(),
            serde_json::Value::String(provider.to_owned()),
        );
    }
    if let Some(adapter) = selection
        .adapter
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        default_selection.insert(
            "adapter".to_owned(),
            serde_json::Value::String(adapter.to_owned()),
        );
    }
    if let Some(model) = selection
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        default_selection.insert(
            "model".to_owned(),
            serde_json::Value::String(model.to_owned()),
        );
    }
    if let Some(mode) = selection
        .thinking_mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        default_selection.insert(
            "thinking_mode".to_owned(),
            serde_json::Value::String(mode.to_owned()),
        );
    }
    if let Some(mode) = selection
        .speed_mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        default_selection.insert(
            "speed_mode".to_owned(),
            serde_json::Value::String(mode.to_owned()),
        );
    }
    if let Some(mode) = selection
        .verbosity
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        default_selection.insert(
            "verbosity".to_owned(),
            serde_json::Value::String(mode.to_owned()),
        );
    }
    if let Some(parallel_tool_calls) = selection.parallel_tool_calls {
        default_selection.insert(
            "parallel_tool_calls".to_owned(),
            serde_json::Value::Bool(parallel_tool_calls),
        );
    }
    providers.insert(
        "default_selection".to_owned(),
        serde_json::Value::Object(default_selection),
    );
}

#[derive(Debug, Clone, PartialEq, Serialize)]
/// Validated configuration used at runtime.
pub struct ResolvedConfig {
    #[serde(skip_serializing)]
    pub default_selection: ExecutionSelection,
    pub tracing: RuntimeTracingConfiguration,
    pub ui: UiConfig,
    pub runtime: RuntimeConfig,
    pub session: SessionConfig,
    #[serde(default, skip_serializing_if = "PermissionConfig::is_empty")]
    pub permission: PermissionConfig,
    pub plugins: PluginConfig,
    #[serde(default, skip_serializing_if = "HarnessesConfig::is_empty")]
    pub harnesses: HarnessesConfig,
    pub providers: BTreeMap<String, ResolvedProviderConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TuiUiConfig;
    fn minimal_config() -> ResolvedConfig {
        ResolvedConfig {
            default_selection: ExecutionSelection::default(),
            tracing: RuntimeTracingConfiguration {
                filter: "info".to_owned(),
                database: "error".to_owned(),
                adapter: "off".to_owned(),
            },
            ui: UiConfig {
                locale: None,
                tui: TuiUiConfig::default(),
            },
            runtime: RuntimeConfig::default(),
            session: SessionConfig::default(),
            permission: PermissionConfig::default(),
            plugins: PluginConfig::default(),
            harnesses: HarnessesConfig::default(),
            providers: BTreeMap::new(),
        }
    }

    #[test]
    fn config_resolution_json_exposes_global_default_without_provider_defaults() {
        let mut config = minimal_config();
        config.default_selection = ExecutionSelection {
            provider: Some("openai".to_owned()),
            adapter: Some("responses".to_owned()),
            model: Some("gpt-5".to_owned()),
            ..Default::default()
        };
        let resolution = ConfigResolution {
            config,
            meta: ConfigResolutionMeta {
                config_path: PathBuf::from("/tmp/agena.json"),
                config_found: true,
                project_config_path: PathBuf::from("/tmp/project/agena.json"),
                project_config_found: false,
                applied_layers: vec![],
            },
        };

        let value = config_resolution_json_value(&resolution).expect("serialize");
        assert!(value["config"]["providers"].get("default").is_none());
        assert_eq!(
            value["config"]["providers"]["default_selection"]["model"],
            "gpt-5"
        );
    }
}
