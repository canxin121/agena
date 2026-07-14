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

        let mut merged = if file_state.found {
            file_state.config.clone()
        } else {
            RawConfig::default()
        };
        let mut applied_layers = vec![AppliedLayer {
            source: ConfigSource::Default,
            description: "built-in defaults".to_owned(),
        }];

        if file_state.found {
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
    use std::{
        collections::BTreeMap,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;
    use crate::{
        config::{RawRuntimeConfig, RuntimeConfig, TuiColorSchemeConfig},
        provider::{
            DEFAULT_CLAUDE_CLIENT_VERSION, DEFAULT_CODEX_CLIENT_VERSION,
            DEFAULT_GEMINI_CLIENT_VERSION,
        },
    };

    #[derive(Clone, Default)]
    struct TestEnvironment {
        values: BTreeMap<String, String>,
    }

    impl ConfigEnvironment for TestEnvironment {
        fn var(&self, key: &str) -> Option<String> {
            self.values.get(key).cloned()
        }

        fn vars(&self) -> Vec<(String, String)> {
            self.values
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        }
    }

    fn test_root() -> PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "agena-tui-theme-config-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn loads_tui_appearance_from_canonical_config() {
        let root = test_root();
        let config_dir = root.join("agena");
        std::fs::create_dir_all(&config_dir).expect("create test config directory");
        std::fs::write(
            config_dir.join("agena.json"),
            r#"{"ui":{"tui":{"color_scheme":"light","theme":"paper"}}}"#,
        )
        .expect("write test config");
        let env = TestEnvironment {
            values: BTreeMap::from([("HOME".to_owned(), root.display().to_string())]),
        };
        let resolution = ConfigLoader::new(env)
            .load(&LoadConfigRequest {
                workspace_root: Some(root.join("workspace")),
                ..LoadConfigRequest::default()
            })
            .expect("load config");
        assert_eq!(
            resolution.config.ui.tui.color_scheme,
            TuiColorSchemeConfig::Light
        );
        assert_eq!(resolution.config.ui.tui.theme.as_deref(), Some("paper"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tui_environment_overrides_are_validated_and_resolved() {
        let root = test_root();
        let env = TestEnvironment {
            values: BTreeMap::from([
                ("HOME".to_owned(), root.display().to_string()),
                ("AGENA_TUI_COLOR_SCHEME".to_owned(), "dark".to_owned()),
                ("AGENA_TUI_THEME".to_owned(), "night-owl".to_owned()),
            ]),
        };
        let resolution = ConfigLoader::new(env)
            .load(&LoadConfigRequest {
                workspace_root: Some(root.join("workspace")),
                ..LoadConfigRequest::default()
            })
            .expect("load config");
        assert_eq!(
            resolution.config.ui.tui.color_scheme,
            TuiColorSchemeConfig::Dark
        );
        assert_eq!(resolution.config.ui.tui.theme.as_deref(), Some("night-owl"));

        let invalid_env = TestEnvironment {
            values: BTreeMap::from([
                ("HOME".to_owned(), root.display().to_string()),
                ("AGENA_TUI_COLOR_SCHEME".to_owned(), "sepia".to_owned()),
            ]),
        };
        assert!(
            ConfigLoader::new(invalid_env)
                .load(&LoadConfigRequest {
                    workspace_root: Some(root.join("workspace")),
                    ..LoadConfigRequest::default()
                })
                .is_err()
        );
    }

    #[test]
    fn provider_client_versions_migrate_legacy_auto_to_the_pinned_default() {
        let root = test_root();
        let config_dir = root.join("agena");
        std::fs::create_dir_all(&config_dir).expect("create test config directory");
        std::fs::write(
            config_dir.join("agena.json"),
            r#"{
                "runtime": {
                    "providers": {
                        "client_versions": {
                            "codex": "0.200.1",
                            "claude": "auto",
                            "gemini": "0.60.0-preview.1"
                        }
                    }
                }
            }"#,
        )
        .expect("write test config");
        let env = TestEnvironment {
            values: BTreeMap::from([("HOME".to_owned(), root.display().to_string())]),
        };

        let resolution = ConfigLoader::new(env)
            .load(&LoadConfigRequest {
                workspace_root: Some(root.join("workspace")),
                ..LoadConfigRequest::default()
            })
            .expect("load config");
        let versions = resolution.config.runtime.providers.client_versions;

        assert_eq!(versions.codex, "0.200.1");
        assert_eq!(versions.claude, DEFAULT_CLAUDE_CLIENT_VERSION);
        assert_eq!(versions.gemini, "0.60.0-preview.1");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn provider_client_versions_default_to_current_pinned_versions() {
        let versions = RuntimeConfig::from_raw(RawRuntimeConfig::default())
            .expect("default runtime config")
            .providers
            .client_versions;

        assert_eq!(versions.codex, DEFAULT_CODEX_CLIENT_VERSION);
        assert_eq!(versions.claude, DEFAULT_CLAUDE_CLIENT_VERSION);
        assert_eq!(versions.gemini, DEFAULT_GEMINI_CLIENT_VERSION);
        assert_ne!(versions.codex, "auto");
        assert_ne!(versions.claude, "auto");
        assert_ne!(versions.gemini, "auto");
    }

    #[test]
    fn provider_client_version_rejects_header_unsafe_values() {
        let root = test_root();
        let config_dir = root.join("agena");
        std::fs::create_dir_all(&config_dir).expect("create test config directory");
        std::fs::write(
            config_dir.join("agena.json"),
            r#"{
                "runtime": {
                    "providers": {
                        "client_versions": { "codex": "1.0.0 invalid" }
                    }
                }
            }"#,
        )
        .expect("write test config");
        let env = TestEnvironment {
            values: BTreeMap::from([("HOME".to_owned(), root.display().to_string())]),
        };

        let error = ConfigLoader::new(env)
            .load(&LoadConfigRequest {
                workspace_root: Some(root.join("workspace")),
                ..LoadConfigRequest::default()
            })
            .expect_err("unsafe client version should fail validation");

        assert!(error.to_string().contains("client_versions.codex"));
        let _ = std::fs::remove_dir_all(root);
    }
}
