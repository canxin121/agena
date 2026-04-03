use std::path::PathBuf;

use super::{
    AppliedLayer, ConfigError, ConfigModeName, ConfigOverride, ConfigResolution,
    ConfigResolutionMeta, ConfigSource, RawConfig, RawConfigFile,
};

const DEFAULT_CONFIG_DIR_NAME: &str = ".agena";
const DEFAULT_CONFIG_FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone)]
pub struct LoadConfigRequest {
    pub config_path: Option<PathBuf>,
    pub mode: Option<ConfigModeName>,
    pub overrides: Vec<ConfigOverride>,
}

impl Default for LoadConfigRequest {
    fn default() -> Self {
        Self {
            config_path: None,
            mode: None,
            overrides: Vec::new(),
        }
    }
}

pub trait ConfigEnvironment: Send + Sync {
    fn var(&self, key: &str) -> Option<String>;
    fn vars(&self) -> Vec<(String, String)>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessEnvironment;

impl ConfigEnvironment for ProcessEnvironment {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn vars(&self) -> Vec<(String, String)> {
        std::env::vars().collect()
    }
}

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

    pub fn default_config_path(&self) -> PathBuf {
        self.env
            .var("AGENA_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(default_config_path)
    }

    pub fn environment(&self) -> &E {
        &self.env
    }

    pub fn load(&self, request: &LoadConfigRequest) -> Result<ConfigResolution, ConfigError> {
        let config_path = request
            .config_path
            .clone()
            .unwrap_or_else(|| self.default_config_path());

        let file_state = RawConfigFile::read(&config_path)?;
        let env_overlay = RawConfig::from_env(&self.env)?;

        let cli_mode = request
            .overrides
            .iter()
            .find_map(|item| match item {
                ConfigOverride::Mode(mode) => Some(mode.clone()),
                _ => None,
            })
            .or_else(|| request.mode.clone());
        let env_mode = self
            .env
            .var("AGENA_MODE")
            .map(ConfigModeName::try_from)
            .transpose()?;
        let file_mode = file_state
            .config
            .mode
            .clone()
            .map(ConfigModeName::try_from)
            .transpose()?;

        let mut merged = RawConfig::default();
        let mut applied_layers = vec![AppliedLayer {
            source: ConfigSource::Default,
            description: "built-in defaults".to_owned(),
        }];

        if file_state.found {
            merged.merge_from(file_state.config.clone());
            applied_layers.push(AppliedLayer {
                source: ConfigSource::File,
                description: format!("file:{}", config_path.display()),
            });
        }

        let active_mode_source = if cli_mode.is_some() {
            Some(ConfigSource::Cli)
        } else if self.env.var("AGENA_MODE").is_some() {
            Some(ConfigSource::Environment)
        } else if file_state.config.mode.is_some() {
            Some(ConfigSource::File)
        } else {
            None
        };
        let active_mode = cli_mode.clone().or(env_mode).or(file_mode);

        if let Some(mode_name) = active_mode.clone() {
            let mode_overlay = file_state.resolve_mode(&mode_name)?;
            merged.merge_mode(mode_overlay);
            applied_layers.push(AppliedLayer {
                source: ConfigSource::Mode,
                description: format!("mode:{}", mode_name),
            });
        }

        if !env_overlay.is_empty() {
            merged.merge_from(env_overlay);
            applied_layers.push(AppliedLayer {
                source: ConfigSource::Environment,
                description: "process environment".to_owned(),
            });
        }

        if !request.overrides.is_empty() {
            for override_item in &request.overrides {
                override_item.apply_to(&mut merged);
            }
            applied_layers.push(AppliedLayer {
                source: ConfigSource::Cli,
                description: format!("{} cli override(s)", request.overrides.len()),
            });
        }

        let config = merged.resolve()?;
        Ok(ConfigResolution {
            config,
            meta: ConfigResolutionMeta {
                config_path,
                config_found: file_state.found,
                active_mode,
                active_mode_source,
                applied_layers,
            },
        })
    }

    pub fn list_modes(
        &self,
        config_path: Option<PathBuf>,
    ) -> Result<Vec<ConfigModeName>, ConfigError> {
        let path = config_path.unwrap_or_else(|| self.default_config_path());
        let file = RawConfigFile::read(&path)?;
        Ok(file
            .config
            .modes
            .keys()
            .cloned()
            .map(ConfigModeName::try_from)
            .collect::<Result<Vec<_>, _>>()?)
    }
}

fn default_config_path() -> PathBuf {
    let mut base = home_dir().unwrap_or_else(|| PathBuf::from("."));
    base.push(DEFAULT_CONFIG_DIR_NAME);
    base.push(DEFAULT_CONFIG_FILE_NAME);
    base
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}
