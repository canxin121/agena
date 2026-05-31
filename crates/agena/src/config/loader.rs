use std::path::{Path, PathBuf};

use super::{
    AppliedLayer, ConfigError, ConfigOverride, ConfigResolution, ConfigResolutionMeta,
    ConfigSource, RawConfig, RawConfigFile,
};

const DEFAULT_CONFIG_DIR_NAME: &str = "agena";
const DEFAULT_CONFIG_FILE_NAME: &str = "agena.json";

#[derive(Debug, Clone, Default)]
pub struct LoadConfigRequest {
    pub overrides: Vec<ConfigOverride>,
    pub workspace_root: Option<PathBuf>,
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
        default_config_path(&self.env)
    }

    pub fn environment(&self) -> &E {
        &self.env
    }

    pub fn load(&self, request: &LoadConfigRequest) -> Result<ConfigResolution, ConfigError> {
        let config_path = self.default_config_path();
        let workspace_root = request
            .workspace_root
            .clone()
            .unwrap_or_else(default_workspace_root);
        let project_config_path = project_config_path(workspace_root.as_path());

        if self.env.var("AGENA_MODE").is_some() {
            return Err(ConfigError::UnsupportedModeEnvironment);
        }

        let file_state = RawConfigFile::read(&config_path)?;
        let project_file_state = RawConfigFile::read(&project_config_path)?;
        let env_overlay = RawConfig::from_env(&self.env)?;

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

        if project_file_state.found {
            merged.merge_project_from_with_keys(
                project_file_state.config.clone(),
                project_file_state.merge_keys,
            );
            applied_layers.push(AppliedLayer {
                source: ConfigSource::Project,
                description: format!("project:{}", project_config_path.display()),
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

        let config = merged.resolve_with_env(&self.env)?;
        Ok(ConfigResolution {
            config,
            meta: ConfigResolutionMeta {
                config_path,
                config_found: file_state.found,
                project_config_path,
                project_config_found: project_file_state.found,
                applied_layers,
            },
        })
    }
}

fn default_config_path(env: &impl ConfigEnvironment) -> PathBuf {
    let mut base = home_dir(env).unwrap_or_else(|| PathBuf::from("."));
    base.push(DEFAULT_CONFIG_DIR_NAME);
    base.push(DEFAULT_CONFIG_FILE_NAME);
    base
}

fn default_workspace_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn project_config_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join(format!(".{DEFAULT_CONFIG_DIR_NAME}"))
        .join(DEFAULT_CONFIG_FILE_NAME)
}

fn home_dir(env: &impl ConfigEnvironment) -> Option<PathBuf> {
    env.var("HOME")
        .or_else(|| env.var("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[derive(Debug, Default)]
    struct TestEnvironment {
        vars: Vec<(String, String)>,
    }

    impl TestEnvironment {
        fn with_var(mut self, key: &str, value: &str) -> Self {
            self.vars.push((key.to_owned(), value.to_owned()));
            self
        }
    }

    impl ConfigEnvironment for TestEnvironment {
        fn var(&self, key: &str) -> Option<String> {
            self.vars
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, value)| value.clone())
        }

        fn vars(&self) -> Vec<(String, String)> {
            self.vars.clone()
        }
    }

    #[test]
    fn default_config_path_uses_single_home_agena_json() {
        let loader = ConfigLoader::new(TestEnvironment::default().with_var("HOME", "/home/user"));

        assert_eq!(
            loader.default_config_path(),
            PathBuf::from("/home/user/agena/agena.json")
        );
    }

    #[test]
    fn default_config_path_falls_back_to_userprofile() {
        let loader =
            ConfigLoader::new(TestEnvironment::default().with_var("USERPROFILE", "C:\\Users\\me"));

        assert_eq!(
            loader.default_config_path(),
            PathBuf::from("C:\\Users\\me")
                .join("agena")
                .join("agena.json")
        );
    }

    #[test]
    fn load_applies_project_config_after_global_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let workspace = temp.path().join("workspace");
        let global_config_dir = home.join("agena");
        let project_config_dir = workspace.join(".agena");
        fs::create_dir_all(&global_config_dir).expect("global config dir");
        fs::create_dir_all(&project_config_dir).expect("project config dir");
        fs::write(
            global_config_dir.join("agena.json"),
            r#"{
              "agents": {
                "default": "review",
                "review": {
                  "description": "Global review agent",
                  "prompt": "Global prompt"
                }
              },
              "tracing": {
                "filter": "info"
              }
            }"#,
        )
        .expect("write global config");
        fs::write(
            project_config_dir.join("agena.json"),
            r#"{
              "agents": {
                "review": {
                  "description": "Project review agent"
                }
              },
              "tracing": {
                "filter": "debug"
              }
            }"#,
        )
        .expect("write project config");

        let loader = ConfigLoader::new(
            TestEnvironment::default().with_var("HOME", home.to_string_lossy().as_ref()),
        );
        let resolution = loader
            .load(&LoadConfigRequest {
                workspace_root: Some(workspace.clone()),
                ..LoadConfigRequest::default()
            })
            .expect("config should load");
        let review = resolution
            .config
            .agents
            .get("review")
            .expect("review agent");

        assert!(resolution.meta.config_found);
        assert!(resolution.meta.project_config_found);
        assert_eq!(
            resolution.meta.project_config_path,
            workspace.join(".agena").join("agena.json")
        );
        assert_eq!(resolution.config.tracing.filter, "debug");
        assert_eq!(review.description, "Project review agent");
        assert!(review.prompt.is_empty());
        assert_eq!(
            resolution
                .meta
                .applied_layers
                .iter()
                .map(|layer| layer.source)
                .collect::<Vec<_>>(),
            vec![
                ConfigSource::Default,
                ConfigSource::File,
                ConfigSource::Project
            ]
        );
    }
}
