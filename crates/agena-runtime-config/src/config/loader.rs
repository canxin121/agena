use std::path::Path;

use agena_plugin_host::{PluginPackage, PluginProfileEntry};

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
        self.load_scoped(request, false)
    }

    /// Resolve instance settings without reading or merging a project file.
    pub fn load_global(
        &self,
        request: &LoadConfigRequest,
    ) -> Result<ConfigResolution, ConfigError> {
        self.load_scoped(request, true)
    }

    fn load_scoped(
        &self,
        request: &LoadConfigRequest,
        global: bool,
    ) -> Result<ConfigResolution, ConfigError> {
        let config_path = request
            .config_path
            .clone()
            .unwrap_or_else(|| crate::default_config_path(&self.env));
        let workspace_root = if global {
            std::path::PathBuf::new()
        } else {
            match request.workspace_root.clone() {
                Some(workspace_root) => workspace_root,
                None => crate::try_default_workspace_root()?,
            }
        };
        let project_config_path = if global {
            std::path::PathBuf::new()
        } else {
            crate::project_config_path(workspace_root.as_path())
        };

        crate::network_config_migration::migrate_network_config_files(
            &config_path,
            &project_config_path,
        )?;
        let file_state = RawConfigFile::read(&config_path)?;
        let project_file_state = if global {
            RawConfigFile {
                config: RawConfig::default(),
                found: false,
                merge_keys: Default::default(),
            }
        } else {
            RawConfigFile::read(&project_config_path)?
        };
        let env_overlay = RawConfig::from_env(&self.env)?;
        if project_file_state.config.access_control.is_some() {
            return Err(ConfigError::Validation(
                "access_control is instance-owned; edit it in global settings, not a workspace file".into(),
            ));
        }
        let environment_applied = !env_overlay.is_empty();

        if !global {
            super::raw::validate_global_agent_profile_scope(
                &file_state.config,
                &config_path,
                &self.env,
                &request.overrides,
            )?;
        }

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
        resolve_agent_package_paths(&mut merged, &config_path)?;
        if project_file_state.found {
            let mut project = project_file_state.config;
            validate_project_agent_hooks(&project)?;
            resolve_agent_package_paths(&mut project, &project_config_path)?;
            merged.merge_project_from_with_keys(project, project_file_state.merge_keys);
        }

        if environment_applied {
            merged.merge_from(env_overlay);
        }

        if !request.overrides.is_empty() {
            for override_item in &request.overrides {
                apply_config_override(override_item, &mut merged);
            }
        }

        resolve_agent_package_paths(&mut merged, &config_path)?;
        let mut config = merged.resolve()?;
        super::raw::validate_permission_config_in_workspace(
            &config.permission,
            &workspace_root,
            config.sandbox,
        )?;
        config.prompts = PromptLayersConfig {
            global: global_prompts,
            workspace: workspace_prompts,
            global_dir: prompt_dir(config_path.as_path()),
            workspace_dir: if global {
                std::path::PathBuf::new()
            } else {
                workspace_root.join(".agena").join("prompts")
            },
        };
        // Fragment directories are configuration: a referenced id that has no
        // file must fail here instead of silently dropping instructions.
        config.prompts.validate()?;
        config.prompts.check_libraries()?;
        crate::agent_profiles::validate_agent_profiles_in_workspace(&config, &workspace_root)?;
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

fn validate_project_agent_hooks(config: &RawConfig) -> Result<(), ConfigError> {
    let Some(plugins) = &config.plugins else {
        return Ok(());
    };
    for (id, entry) in &plugins.list {
        if matches!(
            entry.package,
            PluginPackage::AgentPlugin {
                enable_hooks: true,
                ..
            }
        ) {
            return Err(ConfigError::Validation(format!(
                "plugins.list.{id}.package.enable_hooks must be set in the instance configuration"
            )));
        }
    }
    for (profile_id, profile) in &plugins.profiles {
        for (id, entry) in &profile.plugins {
            let package = match entry {
                PluginProfileEntry::Replace { plugin } => Some(&plugin.package),
                PluginProfileEntry::Patch { package, .. } => package.as_ref(),
                _ => None,
            };
            if matches!(
                package,
                Some(PluginPackage::AgentPlugin {
                    enable_hooks: true,
                    ..
                })
            ) {
                return Err(ConfigError::Validation(format!(
                    "automatic hooks for {id} in profile {profile_id} must be set in the instance configuration"
                )));
            }
        }
    }
    Ok(())
}

fn resolve_agent_package_paths(config: &mut RawConfig, file: &Path) -> Result<(), ConfigError> {
    let Some(plugins) = config.plugins.as_mut() else {
        return Ok(());
    };
    let resolve = |package: &mut PluginPackage| -> Result<(), ConfigError> {
        if let PluginPackage::AgentPlugin { path, .. } = package
            && path.is_relative()
        {
            let base = file.parent().unwrap_or_else(|| Path::new("."));
            *path = std::path::absolute(base.join(&*path))
                .map_err(|source| ConfigError::CurrentDirectory { source })?;
        }
        Ok(())
    };
    for plugin in plugins.list.values_mut() {
        resolve(&mut plugin.package)?;
    }
    for profile in plugins.profiles.values_mut() {
        for entry in profile.plugins.values_mut() {
            match entry {
                PluginProfileEntry::Replace { plugin } => resolve(&mut plugin.package)?,
                PluginProfileEntry::Patch {
                    package: Some(package),
                    ..
                } => resolve(package)?,
                _ => {}
            }
        }
    }
    Ok(())
}

fn prompt_dir(config_path: &std::path::Path) -> std::path::PathBuf {
    config_path
        .parent()
        .map(|dir| dir.join("prompts"))
        .unwrap_or_else(|| std::path::PathBuf::from("prompts"))
}
