use super::{
    BTreeMap, HarnessesConfig, PathBuf, ResolvedProviderConfig, RuntimeConfig, Serialize,
    SessionConfig, UiConfig,
};
use crate::RuntimeTracingConfiguration;
use agena_domain::PermissionConfig;
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
    serde_json::to_value(resolution)
}

pub fn resolved_config_json_value(
    config: &ResolvedConfig,
) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(config)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
/// Validated configuration used at runtime.
pub struct ResolvedConfig {
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
    fn config_resolution_json_does_not_expose_provider_defaults() {
        let config = minimal_config();
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
        assert!(
            value["config"]["providers"]
                .get("default_selection")
                .is_none()
        );
    }
}
