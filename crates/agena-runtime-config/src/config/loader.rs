use crate::{
    ConfigError, ConfigResolution, LoadConfigRequest, PromptLayersConfig, RawConfig, RawConfigFile,
    apply_config_override,
};

pub use crate::{ConfigEnvironment, ProcessEnvironment};

/// Loads and layers configuration from defaults, files, and the environment.
#[derive(Clone)]
pub struct ConfigLoader<E = ProcessEnvironment> {
    env: E,
}

impl Default for ConfigLoader<ProcessEnvironment> {
    fn default() -> Self {
        Self::new(ProcessEnvironment)
    }
}

impl<E> ConfigLoader<E>
where
    E: ConfigEnvironment,
{
    pub fn new(env: E) -> Self {
        Self { env }
    }

    pub fn load(&self, request: &LoadConfigRequest) -> Result<ConfigResolution, ConfigError> {
        let config_path = request
            .config_path
            .clone()
            .unwrap_or_else(|| crate::default_config_path(&self.env));
        let workspace_root = match request.workspace_root.clone() {
            Some(workspace_root) => workspace_root,
            None => crate::try_default_workspace_root()?,
        };
        let project_config_path = crate::project_config_path(workspace_root.as_path());

        let file_state = RawConfigFile::read(&config_path)?;
        let project_file_state = RawConfigFile::read(&project_config_path)?;
        let env_overlay = RawConfig::from_env(&self.env)?;
        let environment_applied = !env_overlay.is_empty();

        // Prompt layers stay separate per file: the global file owns the global
        // layer, the project file owns the workspace layer, so they compose
        // rather than the project silently replacing the global layer.
        let global_prompts = file_state.config.prompts.clone().unwrap_or_default();
        let workspace_prompts = project_file_state
            .config
            .prompts
            .clone()
            .unwrap_or_default();

        let mut merged = if file_state.found {
            file_state.config.clone()
        } else {
            RawConfig::default()
        };
        if project_file_state.found {
            merged.merge_project_from_with_keys(
                project_file_state.config.clone(),
                project_file_state.merge_keys,
            );
        }

        if environment_applied {
            merged.merge_from(env_overlay);
        }

        if !request.overrides.is_empty() {
            for override_item in &request.overrides {
                apply_config_override(override_item, &mut merged);
            }
        }

        let mut config = merged.resolve()?;
        config.prompts = PromptLayersConfig {
            global: global_prompts,
            workspace: workspace_prompts,
            global_dir: prompt_dir(config_path.as_path()),
            workspace_dir: workspace_root.join(".agena").join("prompts"),
        };
        // Fragment directories are configuration: a referenced id that has no
        // file must fail here instead of silently dropping instructions.
        config.prompts.validate()?;
        config.prompts.check_libraries()?;
        Ok(ConfigResolution {
            config,
            meta: crate::ConfigResolutionMeta::from_layer_presence(
                config_path,
                file_state.found,
                project_config_path,
                project_file_state.found,
                environment_applied,
                request.overrides.len(),
            ),
        })
    }
}

fn prompt_dir(config_path: &std::path::Path) -> std::path::PathBuf {
    config_path
        .parent()
        .map(|dir| dir.join("prompts"))
        .unwrap_or_else(|| std::path::PathBuf::from("prompts"))
}
