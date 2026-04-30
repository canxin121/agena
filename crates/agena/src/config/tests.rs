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

    assert_eq!(resolution.meta.active_mode.unwrap().to_string(), "dev");
    assert!(resolution.config.providers.contains_key("openai"));
    assert!(resolution.config.providers.contains_key("prod-openai"));
}

#[test]
fn provider_capability_overrides_parse_and_merge_from_mode_layers() {
    let path = write_temp_config(
        r#"
mode = "dev"

[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"

[[providers.openai.capability_overrides]]
model = "gpt-4.1-mini"
image_input = "unsupported"

[[modes.dev.providers.openai.capability_overrides]]
model = "gpt-5"
match = "prefix"
file_input = "supported"
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
    assert_eq!(provider.capability_overrides.len(), 2);
    assert_eq!(provider.capability_overrides[0].model, "gpt-4.1-mini");
    assert_eq!(
        provider.capability_overrides[0].capabilities.image_input,
        Some(crate::provider::CapabilitySupport::Unsupported)
    );
    assert_eq!(
        provider.capability_overrides[1].match_mode,
        crate::provider::CapabilityOverrideMatchMode::Prefix
    );
    assert_eq!(
        provider.capability_overrides[1].capabilities.file_input,
        Some(crate::provider::CapabilitySupport::Supported)
    );
}

#[test]
fn provider_capability_overrides_require_model_and_capability_fields() {
    let path = write_temp_config(
        r#"
[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"

[[providers.openai.capability_overrides]]
model = "   "
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
        matches!(err, ConfigError::Validation(message) if message.contains("capability override model matcher cannot be empty"))
    );
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
fn preset_ollama_resolves_to_localhost_with_local_token() {
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
        ProviderDefinition::OpenAiCompatible(config) => {
            assert_eq!(config.base_url, "http://localhost:11434/v1");
            assert_eq!(config.default_model, "qwen2.5-coder:7b");
            assert_eq!(config.api_key.as_deref(), Some("local"));
            assert!(config.api_key_env.is_none());
        }
        other => panic!("expected openai-compatible preset, got {other:?}"),
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
        ProviderDefinition::OpenAiCompatible(config) => {
            assert_eq!(config.base_url, "http://192.168.1.10:11434/v1");
        }
        other => panic!("expected openai-compatible preset, got {other:?}"),
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
fn permission_bash_rules_load_from_toml_and_compile_into_tool_policy() {
    let path = write_temp_config(
        r#"
[permission]
default_read = "allow"
default_write = "deny"

[[permission.bash]]
pattern = "git *"
mode = "allow"

[[permission.bash]]
pattern = "rm *"
mode = "ask"
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

    assert_eq!(resolution.config.permission.bash_rules.len(), 2);
    assert_eq!(resolution.config.permission.bash_rules[0].pattern, "git *");

    let policy = resolution
        .config
        .tool_permission_policy()
        .expect("tool policy compiles");
    assert_eq!(policy.bash_rules().len(), 2);

    let git_status = crate::message::BuiltinToolInput::Bash(crate::message::BashToolInput {
        command: "git status".to_string(),
        description: String::new(),
        timeout_ms: None,
        workdir: None,
    });
    assert_eq!(
        policy.check_builtin(&git_status),
        crate::permission::PermissionDecision::Allow
    );

    let rm = crate::message::BuiltinToolInput::Bash(crate::message::BashToolInput {
        command: "rm -rf node_modules".to_string(),
        description: String::new(),
        timeout_ms: None,
        workdir: None,
    });
    match policy.check_builtin(&rm) {
        crate::permission::PermissionDecision::Ask { .. } => {}
        other => panic!("expected ask decision, got {other:?}"),
    }
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
    assert_eq!(host.plugins().len(), 3);
    assert_eq!(host.plugins()[0].id, "agena-memory");
    assert_eq!(host.plugins()[1].id, crate::hooks::ShellHookPlugin::id());
    assert_eq!(host.plugins()[2].id, crate::tool::builtins_plugin_id());
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
    // The bogus cdylib entry is skipped; only the in-process built-in plugins
    // remain.
    assert_eq!(host.plugins().len(), 3);
    assert_eq!(host.plugins()[0].id, "agena-memory");
    assert_eq!(host.plugins()[1].id, crate::hooks::ShellHookPlugin::id());
    assert_eq!(host.plugins()[2].id, crate::tool::builtins_plugin_id());
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
