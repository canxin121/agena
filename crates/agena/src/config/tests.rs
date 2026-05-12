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
fn loader_applies_file_then_env_then_cli() {
    let path = write_temp_config(
        r#"
[runtime.provider_http]
timeout_secs = 90

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
            overrides: vec![ConfigOverride::ProviderHttpTimeoutSecs(12)],
        })
        .expect("config should load");

    assert_eq!(resolution.config.runtime.provider_http.timeout_secs, 12);
}

#[test]
fn loader_reads_database_log_level_from_env() {
    let env = TestEnvironment {
        vars: BTreeMap::from([("AGENA_DATABASE_LOG".to_owned(), "error".to_owned())]),
    };
    let path = write_temp_config("");
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("config should load");

    assert_eq!(resolution.config.tracing.database_level, "error");
}

#[test]
fn cli_override_parser_supports_tracing_database_level() {
    let parsed = "tracing.database_level=debug"
        .parse::<ConfigOverride>()
        .expect("override should parse");
    assert!(matches!(
        parsed,
        ConfigOverride::TracingDatabaseLevel(value) if value == "debug"
    ));
}

#[test]
fn tracing_env_filter_includes_database_targets() {
    let tracing = crate::config::TracingConfig {
        filter: "info".to_string(),
        database_level: "error".to_string(),
    };

    let filter = tracing.env_filter().expect("env filter should parse");
    let rendered = filter.to_string();

    assert!(rendered.contains("info"));
    assert!(rendered.contains("sqlx=error"));
    assert!(rendered.contains("sea_orm=error"));
    assert!(rendered.contains("sea_orm_migration=error"));
}

#[test]
fn cli_override_parser_rejects_mode_override() {
    let err = "mode=prod"
        .parse::<ConfigOverride>()
        .expect_err("mode override should be rejected");
    assert!(matches!(
        err,
        ConfigError::UnsupportedModeConfig { field: "mode" }
    ));
}

#[test]
fn loader_rejects_legacy_mode_config() {
    let path = write_temp_config(
        r#"
mode = "prod"

[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("legacy mode config should fail");
    assert!(matches!(
        err,
        ConfigError::UnsupportedModeConfig { field: "mode" }
    ));
}

#[test]
fn loader_rejects_legacy_modes_table() {
    let path = write_temp_config(
        r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"

[modes.prod.permission]
default_write = "ask"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("legacy modes table should fail");
    assert!(matches!(
        err,
        ConfigError::UnsupportedModeConfig { field: "modes" }
    ));
}

#[test]
fn loader_rejects_agena_mode_env() {
    let loader = ConfigLoader::new(TestEnvironment {
        vars: BTreeMap::from([("AGENA_MODE".to_owned(), "prod".to_owned())]),
    });
    let err = loader
        .load(&LoadConfigRequest::default())
        .expect_err("AGENA_MODE should be rejected");
    assert!(matches!(err, ConfigError::UnsupportedModeEnvironment));
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
fn env_telemetry_config_enables_otlp_exporter() {
    let env = TestEnvironment {
        vars: BTreeMap::from([
            ("AGENA_TELEMETRY_ENABLED".to_owned(), "true".to_owned()),
            (
                "AGENA_OTEL_SERVICE_NAME".to_owned(),
                "agena-test".to_owned(),
            ),
            (
                "AGENA_OTEL_ENDPOINT".to_owned(),
                "http://127.0.0.1:4318/v1/traces".to_owned(),
            ),
        ]),
    };
    let path = write_temp_config("");
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("telemetry env config should load");

    assert!(resolution.config.telemetry.enabled);
    assert_eq!(resolution.config.telemetry.service_name, "agena-test");
    assert_eq!(
        resolution.config.telemetry.otlp_endpoint.as_deref(),
        Some("http://127.0.0.1:4318/v1/traces")
    );
}

#[test]
fn example_config_parses_successfully() {
    // The shipped minimal example only configures one provider and no
    // modes; the richer assertions are exercised against config.full.toml
    // by the integration test in tests/config_examples.rs.
    let path = write_temp_config(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config.example.toml"
    )));
    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("example config should load");

    assert!(resolution.config.providers.contains_key("anthropic"));
}

#[test]
fn auth_store_backend_config_loads() {
    let path = write_temp_config(
        r#"
[auth]
store_backend = "file"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("auth config should load");

    assert_eq!(resolution.config.auth.store_backend, AuthStoreBackend::File);
}

#[test]
fn memory_project_instruction_config_loads() {
    let path = write_temp_config(
        r#"
[memory.project_instructions]
enabled = false
include_global = false
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("memory config should load");

    assert!(!resolution.config.memory.project_instructions.enabled);
    assert!(!resolution.config.memory.project_instructions.include_global);
}

#[test]
fn loader_uses_provider_runtime_retry_defaults() {
    let path = write_temp_config("");
    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("default config should load");

    let retry_defaults = crate::provider::ProviderRequestRetryConfig::default();
    let replay_defaults = crate::provider::ProviderStreamReplayConfig::default();

    assert_eq!(
        resolution.config.runtime.request_retry.max_retries,
        retry_defaults.max_retries
    );
    assert_eq!(
        resolution.config.runtime.request_retry.base_delay_ms,
        retry_defaults.base_delay.as_millis() as u64
    );
    assert_eq!(
        resolution.config.runtime.request_retry.max_delay_ms,
        retry_defaults.max_delay.as_millis() as u64
    );
    assert_eq!(
        resolution
            .config
            .runtime
            .stream_replay
            .max_retries_after_output,
        replay_defaults.max_retries_after_output
    );
    assert_eq!(
        resolution.config.runtime.stream_replay.max_tracked_events,
        replay_defaults.max_tracked_events
    );
}

#[test]
fn hooks_parse_from_root_config() {
    let path = write_temp_config(
        r#"
[[hooks]]
event = "user_prompt_submit"
command = "python3 .agena/hooks/enrich.py"
timeout_ms = 3000

[[hooks]]
event = "pre_tool_use"
command = "python3 .agena/hooks/check_tool.py"
timeout_ms = 5000
matcher = { tool = "bash" }

[[hooks]]
event = "post_tool_use"
url = "http://127.0.0.1:8080/agena-hook"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("hook config should load");

    let hooks = resolution.config.hooks.entries();
    assert_eq!(hooks.len(), 3);
    assert!(matches!(
        hooks[0].event,
        crate::hooks::HookEvent::UserPromptSubmit
    ));
    assert!(matches!(
        hooks[1].event,
        crate::hooks::HookEvent::ToolBefore
    ));
    assert_eq!(hooks[1].matcher.tool.as_deref(), Some("bash"));
    assert!(matches!(hooks[2].event, crate::hooks::HookEvent::ToolAfter));
    assert_eq!(
        hooks[2].url.as_deref(),
        Some("http://127.0.0.1:8080/agena-hook")
    );
}

#[test]
fn legacy_hook_event_names_still_parse() {
    let path = write_temp_config(
        r#"
[[hooks]]
event = "tool_before"
command = "legacy-hook"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("legacy hook event should load");

    let hooks = resolution.config.hooks.entries();
    assert_eq!(hooks.len(), 1);
    assert!(matches!(
        hooks[0].event,
        crate::hooks::HookEvent::ToolBefore
    ));
}

#[test]
fn provider_models_parse() {
    let path = write_temp_config(
        r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"

[providers.openai.models."gpt-4.1-mini"]
input = { unsupported = ["image"] }
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("config should load");

    let provider = resolution
        .config
        .providers
        .get("openai")
        .expect("openai provider should exist");
    assert_eq!(provider.models.len(), 1);
    let model = provider
        .models
        .get("gpt-4.1-mini")
        .expect("configured model should exist");
    assert_eq!(
        model.capabilities.image_input,
        Some(crate::provider::CapabilitySupport::Unsupported)
    );
}

#[test]
fn provider_models_require_non_empty_configuration() {
    let path = write_temp_config(
        r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"

[providers.openai.models."gpt-4.1-mini"]
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("invalid override should fail validation");

    assert!(
        matches!(err, ConfigError::Validation(message) if message.contains("model `gpt-4.1-mini` must set at least one field"))
    );
}

#[test]
fn provider_models_reject_overlapping_compact_capabilities() {
    let path = write_temp_config(
        r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"

[providers.openai.models."gpt-4.1-mini"]
input = { supported = ["image"], unsupported = ["image"] }
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("overlapping compact patch should fail validation");

    assert!(
        matches!(err, ConfigError::Validation(message) if message.contains("input capability `image` cannot be both supported and unsupported"))
    );
}

#[test]
fn provider_models_resolved_config_serializes_compact_patch_shape() {
    let path = write_temp_config(
        r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"

[providers.openai.models."gpt-4.1-mini"]
input = { unsupported = ["image"] }
features = ["tool_calling"]
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("config should load");

    let serialized = resolution
        .render(ConfigOutputFormat::Toml)
        .expect("resolved config should serialize");

    assert!(serialized.contains("[config.providers.openai.models.\"gpt-4.1-mini\".input]"));
    assert!(serialized.contains("unsupported = [\"image\"]"));
    assert!(serialized.contains("features = [\"tool_calling\"]"));
    assert!(!serialized.contains("image_input = \"unsupported\""));
    assert!(!serialized.contains("tool_calling = \"supported\""));
}

#[test]
fn preset_openrouter_resolves_to_openai_compatible_with_agena_headers() {
    let presets = write_temp_presets(
        r#"[{"id":"openrouter","npm":"@openrouter/ai-sdk-provider","api":"https://openrouter.ai/api/v1","env":["OPENROUTER_API_KEY"],"default_model":"google/gemini-3-pro-preview"}]"#,
    );
    let path = write_temp_config(
        r#"
[providers.openrouter]
kind = "preset"
"#,
    );

    let env = TestEnvironment {
        vars: BTreeMap::from([(
            "AGENA_PROVIDER_PRESETS_PATH".to_owned(),
            presets.display().to_string(),
        )]),
    };
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("preset config should load");

    let provider = resolution
        .config
        .providers
        .get("openrouter")
        .expect("openrouter provider should exist");

    match &provider.definition {
        ProviderDefinition::OpenAiCompatible(config) => {
            assert_eq!(config.base_url, "https://openrouter.ai/api/v1");
            assert_eq!(config.default_model, "google/gemini-3-pro-preview");
            assert_eq!(config.api_key_env.as_deref(), Some("OPENROUTER_API_KEY"));
            assert!(!config.extra_headers.contains_key("HTTP-Referer"));
            assert_eq!(
                config.extra_headers.get("X-Title").map(String::as_str),
                Some("agena")
            );
        }
        other => panic!("expected openai-compatible preset, got {other:?}"),
    }
}

#[test]
fn preset_ollama_resolves_to_native_localhost_provider() {
    let presets = write_temp_presets("[]");
    let path = write_temp_config(
        r#"
[providers.ollama]
kind = "preset"
default_model = "qwen2.5-coder:7b"
"#,
    );

    let env = TestEnvironment {
        vars: BTreeMap::from([(
            "AGENA_PROVIDER_PRESETS_PATH".to_owned(),
            presets.display().to_string(),
        )]),
    };
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("ollama preset should load");

    let provider = resolution
        .config
        .providers
        .get("ollama")
        .expect("ollama provider should exist");

    match &provider.definition {
        ProviderDefinition::Ollama(config) => {
            assert_eq!(config.base_url, "http://localhost:11434");
            assert_eq!(config.default_model, "qwen2.5-coder:7b");
        }
        other => panic!("expected native ollama preset, got {other:?}"),
    }
}

#[test]
fn preset_ollama_honors_ollama_host_env() {
    let presets = write_temp_presets("[]");
    let path = write_temp_config(
        r#"
[providers.ollama]
kind = "preset"
default_model = "llama3"
"#,
    );

    let env = TestEnvironment {
        vars: BTreeMap::from([
            (
                "AGENA_PROVIDER_PRESETS_PATH".to_owned(),
                presets.display().to_string(),
            ),
            ("OLLAMA_HOST".to_owned(), "192.168.1.10:11434".to_owned()),
        ]),
    };
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("ollama preset should load");

    let provider = resolution
        .config
        .providers
        .get("ollama")
        .expect("ollama provider should exist");

    match &provider.definition {
        ProviderDefinition::Ollama(config) => {
            assert_eq!(config.base_url, "http://192.168.1.10:11434");
        }
        other => panic!("expected native ollama preset, got {other:?}"),
    }
}

#[test]
fn builtin_common_presets_resolve_without_models_dev_payload() {
    let presets = write_temp_presets("[]");
    let path = write_temp_config(
        r#"
[providers.lmstudio]
kind = "preset"

[providers.deepseek]
kind = "preset"

[providers.xai]
kind = "preset"

[providers.groq]
kind = "preset"

[providers.mistral]
kind = "preset"
"#,
    );

    let env = TestEnvironment {
        vars: BTreeMap::from([(
            "AGENA_PROVIDER_PRESETS_PATH".to_owned(),
            presets.display().to_string(),
        )]),
    };
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("builtin presets should load");

    for provider_id in ["lmstudio", "deepseek", "xai", "groq", "mistral"] {
        match &resolution
            .config
            .providers
            .get(provider_id)
            .unwrap_or_else(|| panic!("{provider_id} provider should exist"))
            .definition
        {
            ProviderDefinition::OpenAiCompatible(config) => {
                assert!(!config.base_url.is_empty());
                assert!(!config.default_model.is_empty());
            }
            other => panic!("expected openai-compatible preset for {provider_id}, got {other:?}"),
        }
    }
}

#[test]
fn hook_entries_load_from_toml() {
    let path = write_temp_config(
        r#"
[[hooks]]
event = "user_prompt_submit"
command = "echo $AGENA_PROMPT"

[[hooks]]
event = "tool_before"
command = "/usr/local/bin/audit"
matcher = { tool = "bash" }
timeout_ms = 5000
"#,
    );

    let env = TestEnvironment::default();
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("config should load");

    let entries = resolution.config.hooks.entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].event, crate::hooks::HookEvent::UserPromptSubmit);
    assert_eq!(entries[0].command.as_deref(), Some("echo $AGENA_PROMPT"));
    assert!(entries[0].matcher.tool.is_none());
    assert_eq!(entries[1].event, crate::hooks::HookEvent::ToolBefore);
    assert_eq!(entries[1].matcher.tool.as_deref(), Some("bash"));
    assert_eq!(entries[1].timeout_ms, Some(5000));
}

#[test]
fn loader_rejects_legacy_permission_shape() {
    let path = write_temp_config(
        r#"
[permission]
mode = "ask"

[[permission.bash]]
pattern = "git *"
mode = "allow"

[[permission.bash_deny]]
pattern = "rm -rf /*"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("legacy permission config should fail");
    assert!(matches!(err, ConfigError::ParseFile { .. }));
}

#[test]
fn loader_rejects_removed_execution_mode_permission_field() {
    let path = write_temp_config(
        r#"
[permission.tools]
execution_mode = "ask"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("removed execution_mode field should fail");
    assert!(matches!(err, ConfigError::ParseFile { .. }));
}

#[test]
fn loader_rejects_removed_tool_default_permission_fields() {
    let path = write_temp_config(
        r#"
[permission.tools]
read_only_default = "allow"
mutating_default = "ask"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("removed tool default fields should fail");
    assert!(matches!(err, ConfigError::ParseFile { .. }));
}

#[test]
fn cli_override_parser_rejects_removed_permission_overrides() {
    let err = "permission.default_write=ask"
        .parse::<ConfigOverride>()
        .expect_err("permission override should be rejected");
    assert!(matches!(err, ConfigError::InvalidOverride(_)));
}

#[test]
fn preset_opencode_uses_public_key_when_no_api_key_is_available() {
    let presets = write_temp_presets(
        r#"[{"id":"opencode","npm":"@ai-sdk/openai-compatible","api":"https://opencode.ai/zen/v1","env":["OPENCODE_API_KEY"],"default_model":"gemini-3-pro"}]"#,
    );
    let path = write_temp_config(
        r#"
[providers.opencode]
kind = "preset"
"#,
    );

    let env = TestEnvironment {
        vars: BTreeMap::from([(
            "AGENA_PROVIDER_PRESETS_PATH".to_owned(),
            presets.display().to_string(),
        )]),
    };
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("preset config should load");

    let provider = resolution
        .config
        .providers
        .get("opencode")
        .expect("opencode provider should exist");

    match &provider.definition {
        ProviderDefinition::OpenAiCompatible(config) => {
            assert_eq!(config.api_key.as_deref(), Some("public"));
            assert!(config.api_key_env.is_none());
        }
        other => panic!("expected openai-compatible preset, got {other:?}"),
    }
}

#[test]
fn preset_google_vertex_builds_openapi_endpoint_from_project_and_location() {
    let presets = write_temp_presets(
        r#"[{"id":"google-vertex","npm":"@ai-sdk/google-vertex","api":null,"env":["GOOGLE_VERTEX_PROJECT","GOOGLE_VERTEX_LOCATION","GOOGLE_APPLICATION_CREDENTIALS"],"default_model":"gemini-3-pro-preview"}]"#,
    );
    let path = write_temp_config(
        r#"
[providers."google-vertex"]
kind = "preset"
"#,
    );

    let env = TestEnvironment {
        vars: BTreeMap::from([
            (
                "AGENA_PROVIDER_PRESETS_PATH".to_owned(),
                presets.display().to_string(),
            ),
            ("GOOGLE_CLOUD_PROJECT".to_owned(), "demo-project".to_owned()),
            ("VERTEX_LOCATION".to_owned(), "global".to_owned()),
        ]),
    };
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("preset config should load");

    let provider = resolution
        .config
        .providers
        .get("google-vertex")
        .expect("google-vertex provider should exist");

    match &provider.definition {
        ProviderDefinition::GoogleVertex(config) => {
            assert_eq!(
                config.base_url,
                "https://aiplatform.googleapis.com/v1/projects/demo-project/locations/global/endpoints/openapi"
            );
            assert_eq!(config.default_model, "gemini-3-pro-preview");
            assert!(matches!(config.auth, GoogleVertexAuthConfig::Adc));
        }
        other => panic!("expected google-vertex preset, got {other:?}"),
    }
}

#[test]
fn preset_github_copilot_resolves_to_copilot_provider() {
    let presets = write_temp_presets(
        r#"[{"id":"github-copilot","npm":"@ai-sdk/openai-compatible","api":"https://api.githubcopilot.com","env":["GITHUB_TOKEN"],"default_model":"gemini-3-pro-preview"}]"#,
    );
    let path = write_temp_config(
        r#"
[providers."github-copilot"]
kind = "preset"
"#,
    );

    let env = TestEnvironment {
        vars: BTreeMap::from([(
            "AGENA_PROVIDER_PRESETS_PATH".to_owned(),
            presets.display().to_string(),
        )]),
    };
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("preset config should load");

    let provider = resolution
        .config
        .providers
        .get("github-copilot")
        .expect("github-copilot provider should exist");

    match &provider.definition {
        ProviderDefinition::Copilot(config) => {
            assert_eq!(config.base_url, "https://api.githubcopilot.com");
            assert_eq!(config.default_model, "gemini-3-pro-preview");
            assert_eq!(config.auth_provider_id, "github-copilot");
        }
        other => panic!("expected copilot preset, got {other:?}"),
    }
}

#[test]
fn preset_sap_ai_core_resolves_to_runtime_managed_provider() {
    let presets = write_temp_presets(
        r#"[{"id":"sap-ai-core","npm":"@jerome-benoit/sap-ai-provider-v2","api":null,"env":["AICORE_SERVICE_KEY","AICORE_RESOURCE_GROUP"],"default_model":"anthropic/claude-sonnet-4"}]"#,
    );
    let path = write_temp_config(
        r#"
[providers."sap-ai-core"]
kind = "preset"
"#,
    );

    let env = TestEnvironment {
        vars: BTreeMap::from([
            (
                "AGENA_PROVIDER_PRESETS_PATH".to_owned(),
                presets.display().to_string(),
            ),
            (
                "AICORE_SERVICE_KEY".to_owned(),
                r#"{"clientid":"client","clientsecret":"secret","url":"https://auth.example.com","serviceurls":{"AI_API_URL":"https://api.example.com/v2"}}"#
                    .to_owned(),
            ),
            (
                "AICORE_RESOURCE_GROUP".to_owned(),
                "default-group".to_owned(),
            ),
        ]),
    };
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("preset config should load");

    let provider = resolution
        .config
        .providers
        .get("sap-ai-core")
        .expect("sap-ai-core provider should exist");

    match &provider.definition {
        ProviderDefinition::SapAiCore(config) => {
            assert_eq!(config.base_url, "https://api.example.com/v2");
            assert_eq!(config.default_model, "anthropic/claude-sonnet-4");
            assert!(
                config.api_key.is_none() && config.api_key_env.is_none(),
                "sap-ai-core preset should defer token exchange to runtime"
            );
            assert_eq!(
                config
                    .extra_headers
                    .get("AI-Resource-Group")
                    .map(String::as_str),
                Some("default-group")
            );
        }
        other => panic!("expected sap-ai-core preset, got {other:?}"),
    }
}

#[tokio::test]
async fn build_plugin_host_with_no_entries_succeeds() {
    let dir = temp_dir("plugins-empty");
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

    let host = resolution
        .build_plugin_host()
        .await
        .expect("plugin host should build");
    assert_eq!(host.plugins().len(), 9);
    let ids: Vec<&str> = host.plugins().iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&"agena-memory"));
    assert!(ids.contains(&crate::hooks::ShellHookPlugin::id()));
    assert!(ids.contains(&crate::tool::skills_fs_plugin_id()));
    assert!(ids.contains(&crate::tool::lsp_plugin_id()));
    assert!(ids.contains(&crate::tool::cron_plugin_id()));
    assert!(ids.contains(&crate::tool::fs_plugin_id()));
    assert!(ids.contains(&crate::tool::shell_plugin_id()));
    assert!(ids.contains(&crate::tool::web_plugin_id()));
    assert!(ids.contains(&crate::tool::workflow_plugin_id()));
}

#[tokio::test]
async fn build_plugin_host_rejects_missing_cdylib_path() {
    let dir = temp_dir("plugins-missing");
    let path = dir.join("config.toml");
    fs::write(
        &path,
        r#"
[plugins.list.bogus]
kind = "cdylib"
path = "missing-plugins/libfoo.so"

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

    let host = resolution
        .build_plugin_host()
        .await
        .expect("host build accepts but skips broken plugins");
    // The bogus cdylib entry is skipped; only the in-process first-party
    // plugins plus runtime support plugins remain.
    assert_eq!(host.plugins().len(), 9);
    let ids: Vec<&str> = host.plugins().iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&"agena-memory"));
    assert!(ids.contains(&crate::hooks::ShellHookPlugin::id()));
    assert!(ids.contains(&crate::tool::skills_fs_plugin_id()));
    assert!(ids.contains(&crate::tool::lsp_plugin_id()));
    assert!(ids.contains(&crate::tool::cron_plugin_id()));
    assert!(ids.contains(&crate::tool::fs_plugin_id()));
    assert!(ids.contains(&crate::tool::shell_plugin_id()));
    assert!(ids.contains(&crate::tool::web_plugin_id()));
    assert!(ids.contains(&crate::tool::workflow_plugin_id()));
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

fn write_temp_presets(content: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("agena-presets-{suffix}.json"));
    fs::write(&path, content).expect("temp preset file should be written");
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

#[test]
fn agent_config_and_default_agent_parse() {
    let path = write_temp_config(
        r#"
[runtime]
default_agent = "planner"

[permission.path]
workspace = { read = "allow", write = "ask" }
external = { read = "ask", write = "deny" }

[permission.path.rules]
"<cwd>/.env*" = { read = "ask", write = "deny" }

[permission.network]
internet = "ask"
private = "deny"
loopback = "deny"

[permission.network.rules]
"github.com:443" = "allow"

[permission.tools.tags]
read_only = "allow"
network = "ask"
mutating = "ask"

[permission.tools.first_party]
bash = "ask"

[agents.planner]
description = "Planning agent"
prompt = "You are a planner."
allowed_tools = ["read", "grep"]
model = "openai/gpt-5"
aliases = ["plan"]

[agents.planner.permission.inherit]
path = true
network = true
tools = true
plugin_tools = true

[agents.planner.permission.path]
workspace = { read = "allow", write = "deny" }

[agents.planner.permission.path.rules]
"<cwd>/docs/**" = { read = "allow", write = "ask" }

[agents.planner.permission.tools.first_party]
todo_write = "allow"

[agents.planner.permission.tools.rules.bash]
"git push *" = "deny"
"git *" = "allow"
"*" = "ask"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("agent config should load");

    assert_eq!(
        resolution.config.runtime.default_agent.as_deref(),
        Some("planner")
    );
    let planner = resolution
        .config
        .agents
        .get("planner")
        .expect("planner agent should exist");
    assert_eq!(planner.description, "Planning agent");
    assert_eq!(planner.prompt, "You are a planner.");
    assert_eq!(planner.allowed_tools, vec!["read", "grep"]);
    assert_eq!(
        resolution
            .config
            .permission
            .path
            .workspace
            .as_ref()
            .and_then(|modes| modes.write),
        Some(crate::permission::PermissionMode::Ask)
    );
    assert!(
        resolution
            .config
            .permission
            .path
            .rules
            .contains_key("<cwd>/.env*")
    );
    assert_eq!(
        resolution.config.permission.network.internet,
        Some(crate::permission::PermissionMode::Ask)
    );
    assert_eq!(
        resolution
            .config
            .permission
            .network
            .rules
            .get("github.com:443"),
        Some(&crate::permission::PermissionMode::Allow)
    );
    assert_eq!(
        resolution.config.permission.tools.tags.get("network"),
        Some(&crate::permission::PermissionMode::Ask)
    );
    assert_eq!(
        planner
            .permission
            .path
            .as_ref()
            .and_then(|path| path.workspace.as_ref())
            .and_then(|modes| modes.read),
        Some(crate::permission::PermissionMode::Allow)
    );
    assert_eq!(
        planner
            .permission
            .path
            .as_ref()
            .and_then(|path| path.workspace.as_ref())
            .and_then(|modes| modes.write),
        Some(crate::permission::PermissionMode::Deny)
    );
    assert_eq!(
        planner
            .permission
            .tools
            .as_ref()
            .and_then(|tools| tools.first_party.get("todo_write")),
        Some(&crate::permission::PermissionMode::Allow)
    );
    match planner
        .permission
        .tools
        .as_ref()
        .and_then(|tools| tools.rules.get("bash"))
    {
        Some(crate::agent::ToolPermissionRules::Ordered(entries)) => {
            let collected = entries
                .iter()
                .map(|(pattern, mode)| (pattern.as_str(), *mode))
                .collect::<Vec<_>>();
            assert_eq!(collected.len(), 3);
            assert!(collected.contains(&("git push *", crate::permission::PermissionMode::Deny)));
            assert!(collected.contains(&("git *", crate::permission::PermissionMode::Allow)));
            assert!(collected.contains(&("*", crate::permission::PermissionMode::Ask)));
        }
        other => panic!("expected ordered bash tool rules, got {other:?}"),
    }
    assert_eq!(planner.model.as_deref(), Some("openai/gpt-5"));
    assert_eq!(planner.aliases, vec!["plan"]);
    assert!(!planner.disabled);
}

#[test]
fn runtime_default_agent_falls_back_to_build() {
    let path = write_temp_config(
        r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-5"
api_key = "dummy"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("config should load");

    assert_eq!(
        resolution.config.runtime.default_agent.as_deref(),
        Some("build")
    );
}
