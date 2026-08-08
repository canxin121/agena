use crate::{
    ConfigError, ConfigResolution, LoadConfigRequest, RawConfig, RawConfigFile,
    apply_config_override,
};

pub use crate::{ConfigEnvironment, ProcessEnvironment};

/// Loads and layers configuration from defaults, files, and the environment.
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
        let config_path = crate::default_config_path(&self.env);
        let workspace_root = request
            .workspace_root
            .clone()
            .unwrap_or_else(crate::default_workspace_root);
        let project_config_path = crate::project_config_path(workspace_root.as_path());

        crate::reject_unsupported_mode_environment(&self.env)?;

        let file_state = RawConfigFile::read(&config_path)?;
        let project_file_state = RawConfigFile::read(&project_config_path)?;
        let env_overlay = RawConfig::from_env(&self.env)?;
        let environment_applied = !env_overlay.is_empty();

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

        let config = merged.resolve_with_env(&self.env)?;
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

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;
    use crate::{TuiColorSchemeConfig, TuiGraphicsModeConfig};

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
            r#"{"ui":{"tui":{"color_scheme":"light","graphics":"unicode","theme":"paper"}}}"#,
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
        assert_eq!(
            resolution.config.ui.tui.graphics,
            TuiGraphicsModeConfig::Unicode
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
                ("AGENA_TUI_GRAPHICS".to_owned(), "native".to_owned()),
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
        assert_eq!(
            resolution.config.ui.tui.graphics,
            TuiGraphicsModeConfig::Native
        );

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

        let invalid_graphics_env = TestEnvironment {
            values: BTreeMap::from([
                ("HOME".to_owned(), root.display().to_string()),
                ("AGENA_TUI_GRAPHICS".to_owned(), "ansi-art".to_owned()),
            ]),
        };
        assert!(
            ConfigLoader::new(invalid_graphics_env)
                .load(&LoadConfigRequest {
                    workspace_root: Some(root.join("workspace")),
                    ..LoadConfigRequest::default()
                })
                .is_err()
        );

        let cli_override = ConfigLoader::new(TestEnvironment {
            values: BTreeMap::from([("HOME".to_owned(), root.display().to_string())]),
        })
        .load(&LoadConfigRequest {
            workspace_root: Some(root.join("workspace")),
            overrides: vec![
                "ui.tui.graphics=unicode"
                    .parse()
                    .expect("parse graphics override"),
            ],
        })
        .expect("load config with graphics override");
        assert_eq!(
            cli_override.config.ui.tui.graphics,
            TuiGraphicsModeConfig::Unicode
        );
    }

    #[test]
    fn session_max_turns_env_and_default_resolve() {
        let root = test_root();
        let base = TestEnvironment {
            values: BTreeMap::from([("HOME".to_owned(), root.display().to_string())]),
        };

        // `None` when the env var is absent (falls back to the session
        // manager's default).
        let plain = ConfigLoader::new(base.clone())
            .load(&LoadConfigRequest {
                workspace_root: Some(root.join("workspace")),
                ..LoadConfigRequest::default()
            })
            .expect("load config without max_turns env");
        assert_eq!(plain.config.session.max_turns, None);

        // Positive value from env.
        let capped = ConfigLoader::new(TestEnvironment {
            values: BTreeMap::from([
                ("HOME".to_owned(), root.display().to_string()),
                ("AGENA_SESSION_MAX_TURNS".to_owned(), "25".to_owned()),
            ]),
        })
        .load(&LoadConfigRequest {
            workspace_root: Some(root.join("workspace")),
            ..LoadConfigRequest::default()
        })
        .expect("load config with max_turns env");
        assert_eq!(capped.config.session.max_turns, Some(25));

        // `0` means unlimited, surfaced verbatim so the session manager can
        // translate it.
        let unlimited = ConfigLoader::new(TestEnvironment {
            values: BTreeMap::from([
                ("HOME".to_owned(), root.display().to_string()),
                ("AGENA_SESSION_MAX_TURNS".to_owned(), "0".to_owned()),
            ]),
        })
        .load(&LoadConfigRequest {
            workspace_root: Some(root.join("workspace")),
            ..LoadConfigRequest::default()
        })
        .expect("load config with zero max_turns env");
        assert_eq!(unlimited.config.session.max_turns, Some(0));

        // A non-numeric value must be rejected.
        let invalid = ConfigLoader::new(TestEnvironment {
            values: BTreeMap::from([
                ("HOME".to_owned(), root.display().to_string()),
                ("AGENA_SESSION_MAX_TURNS".to_owned(), "many".to_owned()),
            ]),
        })
        .load(&LoadConfigRequest {
            workspace_root: Some(root.join("workspace")),
            ..LoadConfigRequest::default()
        });
        assert!(invalid.is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn client_versions_compaction_and_presentation_policy_resolve() {
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
                            "claude": "2.2.0",
                            "gemini": "0.60.0"
                        }
                    }
                },
                "session": {
                    "compaction": {
                        "auto": false,
                        "reserved_tokens": 8192
                    },
                    "max_turns": 50
                },
                "plugins": {
                    "policy": {
                        "tool_presentation": { "default_mode": "brief" },
                        "ui_presentation": { "default_mode": "summary" }
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
            .expect("restored settings should load");

        assert_eq!(
            resolution.config.runtime.providers.client_versions.codex,
            "0.200.1"
        );
        assert_eq!(
            resolution.config.runtime.providers.client_versions.claude,
            "2.2.0"
        );
        assert_eq!(
            resolution.config.runtime.providers.client_versions.gemini,
            "0.60.0"
        );
        assert!(!resolution.config.session.compaction.auto);
        assert_eq!(
            resolution.config.session.compaction.reserved_tokens,
            Some(8192)
        );
        assert_eq!(resolution.config.session.max_turns, Some(50));
        assert_eq!(
            resolution
                .config
                .plugins
                .policy
                .tool_presentation
                .default_mode,
            agena_plugin_host::ToolDescriptionMode::Brief
        );
        assert_eq!(
            resolution
                .config
                .plugins
                .policy
                .ui_presentation
                .default_mode,
            agena_plugin_host::UiTextDisplayMode::Summary
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn provider_network_timeouts_resolve_and_cli_overrides_apply() {
        let root = test_root();
        let config_dir = root.join("agena");
        std::fs::create_dir_all(&config_dir).expect("create test config directory");
        std::fs::write(
            config_dir.join("agena.json"),
            r#"{
                "providers": {
                    "default": "local",
                    "local": {
                        "defaults": { "adapter": "ollama", "model": "qwen3" },
                        "network": {
                            "request_timeout_secs": 45,
                            "connect_timeout_secs": 6
                        },
                        "adapters": {
                            "ollama": {
                                "enabled": true,
                                "base_url": "http://localhost:11434",
                                "models": { "qwen3": {} }
                            }
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
                overrides: vec![
                    "providers.local.network.request_timeout_secs=75"
                        .parse()
                        .expect("parse provider request timeout override"),
                    "providers.local.network.connect_timeout_secs=9"
                        .parse()
                        .expect("parse provider connect timeout override"),
                ],
            })
            .expect("load provider network config");

        let provider = resolution.config.providers.get("local").expect("provider");
        assert_eq!(provider.network.request_timeout_secs, 75);
        assert_eq!(provider.network.connect_timeout_secs, 9);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn provider_network_timeouts_must_be_positive() {
        let root = test_root();
        let config_dir = root.join("agena");
        std::fs::create_dir_all(&config_dir).expect("create test config directory");
        std::fs::write(
            config_dir.join("agena.json"),
            r#"{
                "providers": {
                    "local": {
                        "defaults": { "adapter": "ollama" },
                        "network": { "request_timeout_secs": 0 },
                        "adapters": {
                            "ollama": {
                                "enabled": true,
                                "base_url": "http://localhost:11434"
                            }
                        }
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
            .expect_err("zero provider network timeout should fail");

        assert!(error.to_string().contains("greater than zero"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn removed_runtime_tuning_and_malformed_plugin_policy_are_rejected() {
        let root = test_root();
        let config_dir = root.join("agena");
        std::fs::create_dir_all(&config_dir).expect("create test config directory");
        let config_path = config_dir.join("agena.json");
        let env = TestEnvironment {
            values: BTreeMap::from([("HOME".to_owned(), root.display().to_string())]),
        };

        for (field, expected_error, document) in [
            (
                "http",
                "http",
                r#"{"runtime":{"providers":{"http":{"timeout_secs":30}}}}"#,
            ),
            (
                "policy",
                "UiPresentationConfig",
                r#"{"plugins":{"policy":{"ui_presentation":"summary"}}}"#,
            ),
        ] {
            std::fs::write(&config_path, document).expect("write invalid config");
            let error = ConfigLoader::new(env.clone())
                .load(&LoadConfigRequest {
                    workspace_root: Some(root.join("workspace")),
                    ..LoadConfigRequest::default()
                })
                .expect_err("unsupported setting should fail validation");
            assert!(
                error.to_string().contains(expected_error),
                "error should name invalid field {field}: {error}"
            );
        }

        let _ = std::fs::remove_dir_all(root);
    }
}
