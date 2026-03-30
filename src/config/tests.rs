use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

#[derive(Default)]
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
fn cli_override_parser_supports_provider_fields() {
    let parsed = "providers.openai.default_model=gpt-5-mini"
        .parse::<ConfigOverride>()
        .expect("override should parse");
    assert!(matches!(
        parsed,
        ConfigOverride::ProviderDefaultModel { provider_id, value }
            if provider_id == "openai" && value == "gpt-5-mini"
    ));
}

#[test]
fn loader_applies_mode_then_env_then_cli() {
    let path = write_temp_config(
        r#"
mode = "dev"

[runtime.provider_http]
timeout_secs = 90

[modes.dev.runtime.provider_http]
timeout_secs = 45

[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
"#,
    );

    let env = TestEnvironment {
        vars: BTreeMap::from([(
            "AGENA_PROVIDER_HTTP_TIMEOUT_SECS".to_owned(),
            "33".to_owned(),
        )]),
    };
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            mode: None,
            overrides: vec![ConfigOverride::ProviderHttpTimeoutSecs(12)],
        })
        .expect("config should load");

    assert_eq!(resolution.config.runtime.provider_http.timeout_secs, 12);
    assert_eq!(resolution.meta.active_mode.unwrap().to_string(), "dev");
}

#[test]
fn loader_resolves_mode_inheritance() {
    let path = write_temp_config(
        r#"
mode = "prod"

[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"

[modes.shared.runtime.request_retry]
max_retries = 3

[modes.prod]
extends = "shared"

[modes.prod.permission]
default_write = "ask"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("config should load");

    assert_eq!(resolution.config.runtime.request_retry.max_retries, 3);
    assert_eq!(
        resolution.config.permission.default_write,
        crate::permission::PermissionMode::Ask
    );
    assert_eq!(resolution.meta.active_mode.unwrap().to_string(), "prod");
}

#[test]
fn cli_override_wins_over_env_and_file() {
    let path = write_temp_config(
        r#"
[runtime.provider_http]
timeout_secs = 90
"#,
    );

    let env = TestEnvironment {
        vars: BTreeMap::from([(
            "AGENA_PROVIDER_HTTP_TIMEOUT_SECS".to_owned(),
            "44".to_owned(),
        )]),
    };
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            overrides: vec![ConfigOverride::ProviderHttpTimeoutSecs(12)],
            ..LoadConfigRequest::default()
        })
        .expect("config should load");

    assert_eq!(resolution.config.runtime.provider_http.timeout_secs, 12);
}

#[test]
fn env_provider_id_normalization_matches_hyphenated_names() {
    let env = TestEnvironment {
        vars: BTreeMap::from([(
            "AGENA_PROVIDER__GOOGLE_VERTEX__KIND".to_owned(),
            "google_vertex".to_owned(),
        )]),
    };
    let raw = RawConfig::from_env(&env).expect("env config should parse");
    assert!(raw.providers.contains_key("google-vertex"));
}

#[test]
fn example_config_parses_successfully() {
    let path = write_temp_config(include_str!("../../config.example.toml"));
    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("example config should load");

    assert_eq!(resolution.meta.active_mode.unwrap().to_string(), "dev");
    assert!(resolution.config.providers.contains_key("openai"));
    assert!(resolution.config.providers.contains_key("prod-openai"));
}

#[test]
fn build_plugin_manager_uses_config_relative_plugin_directory() {
    let dir = temp_dir("plugins-relative");
    let plugins_dir = dir.join("plugins");
    fs::create_dir_all(&plugins_dir).expect("plugins dir should be created");
    let path = dir.join("config.toml");
    fs::write(
        &path,
        r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
"#,
    )
    .expect("config should be written");

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("config should load");

    let plugins = resolution
        .build_plugin_manager()
        .expect("empty plugin directory should be accepted");
    assert!(plugins.is_empty());
}

#[test]
fn build_plugin_manager_rejects_missing_explicit_path() {
    let dir = temp_dir("plugins-missing");
    let path = dir.join("config.toml");
    fs::write(
        &path,
        r#"
[plugins]
paths = ["missing-plugins"]

[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
"#,
    )
    .expect("config should be written");

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("config should load");

    let err = resolution
        .build_plugin_manager()
        .expect_err("missing explicit plugin path should fail");
    assert!(matches!(err, ConfigError::Validation(message) if message.contains("plugin path does not exist")));
}

fn write_temp_config(content: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("agena-config-{suffix}.toml"));
    fs::write(&path, content).expect("temp config should be written");
    path
}

fn temp_dir(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("agena-{label}-{suffix}"));
    fs::create_dir_all(&path).expect("temp dir should be created");
    path
}
