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
fn cli_override_parser_supports_default_fields() {
    let parsed = "default.model=gpt-5-mini"
        .parse::<ConfigOverride>()
        .expect("override should parse");
    assert!(matches!(
        parsed,
        ConfigOverride::DefaultModel(value) if value == "gpt-5-mini"
    ));
}

#[test]
fn cli_override_parser_supports_provider_auth_protocol_paths() {
    let parsed = "providers.shared.auth.protocol_paths.openai=/api/provider/openai/v1"
        .parse::<ConfigOverride>()
        .expect("override should parse");
    assert!(matches!(
        parsed,
        ConfigOverride::ProviderAuthProtocolPath {
            provider_id,
            protocol,
            value
        } if provider_id == "shared"
            && protocol == "openai"
            && value == "/api/provider/openai/v1"
    ));
}

#[test]
fn loader_applies_file_then_env_then_cli() {
    let path = write_temp_config(
        r#"
[runtime.provider_http]
timeout_secs = 90

[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-test"

[providers.openai.adapters.openai]
enabled = true
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
fn loader_applies_model_catalog_config_from_file_env_and_cli() {
    let path = write_temp_config(
        r#"
[runtime.model_catalog]
cache_max_age_secs = 60

[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-test"

[providers.openai.adapters.openai]
enabled = true
"#,
    );

    let env = TestEnvironment {
        vars: BTreeMap::from([(
            "AGENA_MODEL_CATALOG_CACHE_MAX_AGE_SECS".to_owned(),
            "120".to_owned(),
        )]),
    };
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            overrides: vec![ConfigOverride::ModelCatalogCacheMaxAgeSecs(180)],
        })
        .expect("config should load");

    assert_eq!(
        resolution.config.runtime.model_catalog.cache_max_age_secs,
        180
    );
}

#[test]
fn cli_override_parser_supports_model_catalog_fields() {
    let parsed = "runtime.model_catalog.cache_max_age_secs=3600"
        .parse::<ConfigOverride>()
        .expect("override should parse");
    assert!(matches!(
        parsed,
        ConfigOverride::ModelCatalogCacheMaxAgeSecs(3600)
    ));
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

    assert_eq!(resolution.config.tracing.database, "error");
}

#[test]
fn cli_override_parser_supports_tracing_database() {
    let parsed = "tracing.database=debug"
        .parse::<ConfigOverride>()
        .expect("override should parse");
    assert!(matches!(
        parsed,
        ConfigOverride::TracingDatabase(value) if value == "debug"
    ));
}

#[test]
fn tracing_env_filter_includes_database_targets() {
    let tracing = crate::config::TracingConfig {
        filter: "info".to_string(),
        database: "error".to_string(),
        adapter: "trace".to_string(),
    };

    let filter = tracing.env_filter().expect("env filter should parse");
    let rendered = filter.to_string();

    assert!(rendered.contains("info"));
    assert!(rendered.contains("sqlx=error"));
    assert!(rendered.contains("sea_orm=error"));
    assert!(rendered.contains("sea_orm_migration=error"));
    assert!(rendered.contains("agena::adapter=trace"));
}

#[test]
fn loader_reads_adapter_log_level_from_env() {
    let env = TestEnvironment {
        vars: BTreeMap::from([("AGENA_ADAPTER_LOG".to_owned(), "trace".to_owned())]),
    };
    let path = write_temp_config("");
    let loader = ConfigLoader::new(env);
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("config should load");

    assert_eq!(resolution.config.tracing.adapter, "trace");
}

#[test]
fn cli_override_parser_supports_tracing_adapter() {
    let parsed = "tracing.adapter=debug"
        .parse::<ConfigOverride>()
        .expect("override should parse");
    assert!(matches!(
        parsed,
        ConfigOverride::TracingAdapter(value) if value == "debug"
    ));
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
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"

[providers.openai.adapters.openai]
enabled = true
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
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"

[providers.openai.adapters.openai]
enabled = true

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
fn loader_rejects_provider_level_modes() {
    let path = write_temp_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.thinking_modes.deep]
thinking = { type = "effort", effort = "high" }
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("provider-level modes should fail validation");
    assert!(
        matches!(err, ConfigError::Validation(message) if message.contains("provider-level modes are not supported"))
    );
}

#[test]
fn loader_rejects_preset_provider_kind() {
    let path = write_temp_config(
        r#"
[providers.openrouter]
kind = "preset"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("preset provider kind should be rejected");
    assert!(matches!(err, ConfigError::ParseFile { .. }));
}

#[test]
fn cli_override_parser_rejects_preset_provider_kind() {
    let err = "providers.openrouter.kind=preset"
        .parse::<ConfigOverride>()
        .expect_err("preset provider kind should be rejected");
    assert!(matches!(err, ConfigError::InvalidOverride(_)));
}

#[test]
fn openai_provider_defaults_api_backend_base_url_and_empty_inline_credential() {
    let path = write_temp_config(
        r#"
[providers.api]
default_model = "openai/gpt-5"

[providers.api.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-test"

[providers.api.adapters.openai]
enabled = true
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
        .get("api")
        .expect("api provider should exist");

    assert_eq!(provider.default_model, "openai/gpt-5");
    match &provider.auth {
        ProviderAuthConfig::Api(api) => {
            assert_eq!(api.base_url.as_deref(), Some("https://api.openai.com"));
            assert_eq!(api.api_key.as_deref(), Some("sk-test"));
            assert!(api.api_key_env.is_none());
        }
        other => panic!("expected api auth, got {other:?}"),
    }
    match provider
        .adapters
        .get("openai")
        .expect("openai adapter should exist")
        .definition
    {
        ProviderAdapterDefinition::OpenAi(ref config) => {
            assert_eq!(config.options.backend, OpenAiBackendConfig::Api);
        }
        ref other => panic!("expected openai adapter, got {other:?}"),
    }
}

#[test]
fn openai_chatgpt_codex_backend_defaults_base_url_and_empty_inline_credential() {
    let path = write_temp_config(
        r#"
[providers.chatgpt]
default_model = "openai/gpt-5.3-codex"

[providers.chatgpt.auth]
mode = "credential"
issuer = "openai_chatgpt"

[providers.chatgpt.adapters.openai]
enabled = true
backend = "chatgpt_codex"
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
        .get("chatgpt")
        .expect("chatgpt provider should exist");

    assert_eq!(provider.default_model, "openai/gpt-5.3-codex");
    match &provider.auth {
        ProviderAuthConfig::Credential(config) => {
            assert_eq!(
                config.issuer,
                crate::provider::auth::CredentialIssuer::OpenaiChatgpt
            );
            assert!(config.credential.is_none());
        }
        other => panic!("expected credential auth, got {other:?}"),
    }
    match provider
        .adapters
        .get("openai")
        .expect("openai adapter should exist")
        .definition
    {
        ProviderAdapterDefinition::OpenAi(ref config) => {
            assert_eq!(config.options.backend, OpenAiBackendConfig::ChatgptCodex);
            assert_eq!(config.options.api_mode, OpenAiApiModeConfig::Responses);
            assert_eq!(config.options.stream_mode, StreamTransportMode::Sse);
        }
        ref other => panic!("expected openai adapter, got {other:?}"),
    }
}

#[test]
fn openai_chatgpt_codex_backend_rejects_direct_api_keys() {
    let path = write_temp_config(
        r#"
[providers.chatgpt]
default_model = "openai/gpt-5.3-codex"

[providers.chatgpt.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key_env = "OPENAI_API_KEY"

[providers.chatgpt.adapters.openai]
enabled = true
backend = "chatgpt_codex"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("chatgpt codex backend should reject direct api keys");
    assert!(matches!(
        err,
        ConfigError::InvalidProviderConfig { provider_id, .. } if provider_id == "chatgpt"
    ));
}

#[test]
fn openai_chatgpt_codex_backend_rejects_non_responses_api_mode() {
    let path = write_temp_config(
        r#"
[providers.chatgpt]
default_model = "openai/gpt-5.3-codex"

[providers.chatgpt.auth]
mode = "credential"
issuer = "openai_chatgpt"

[providers.chatgpt.adapters.openai]
enabled = true
backend = "chatgpt_codex"
api_mode = "chat"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("chatgpt codex backend should reject chat api mode");
    assert!(matches!(
        err,
        ConfigError::InvalidProviderConfig { provider_id, .. } if provider_id == "chatgpt"
    ));
}

#[test]
fn openai_provider_with_github_copilot_credential_uses_credential_auth() {
    let path = write_temp_config(
        r#"
[providers."github-copilot"]
default_model = "openai/gpt-4o-mini"

[providers."github-copilot".auth]
mode = "credential"
issuer = "github_copilot"

[providers."github-copilot".adapters.openai]
enabled = true
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
        .get("github-copilot")
        .expect("github-copilot provider should exist");

    match &provider.auth {
        ProviderAuthConfig::Credential(config) => {
            assert_eq!(
                config.issuer,
                crate::provider::auth::CredentialIssuer::GithubCopilot
            );
        }
        other => panic!("expected credential auth, got {other:?}"),
    }
    match provider
        .adapters
        .get("openai")
        .expect("openai adapter should exist")
        .definition
    {
        ProviderAdapterDefinition::OpenAi(ref config) => {
            assert_eq!(config.options.backend, OpenAiBackendConfig::Api);
        }
        ref other => panic!("expected openai adapter, got {other:?}"),
    }
}

#[test]
fn google_adc_legacy_mode_resolves_to_credential_issuer_with_endpoint() {
    let path = write_temp_config(
        r#"
[providers.vertex]
default_model = "openai/google/gemini-2.5-flash"

[providers.vertex.auth]
mode = "google_adc"
base_url = "https://us-central1-aiplatform.googleapis.com"

[providers.vertex.auth.protocol_paths]
openai = "/v1/projects/PROJECT/locations/us-central1/endpoints/openapi"

[providers.vertex.adapters.openai]
enabled = true
capability_family = "gemini"
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
        .get("vertex")
        .expect("vertex provider should exist");

    match &provider.auth {
        ProviderAuthConfig::Credential(config) => {
            assert_eq!(
                config.issuer,
                crate::provider::auth::CredentialIssuer::GoogleAdc
            );
            assert_eq!(
                config.base_url.as_deref(),
                Some("https://us-central1-aiplatform.googleapis.com")
            );
            assert_eq!(
                config.protocol_paths.openai,
                "/v1/projects/PROJECT/locations/us-central1/endpoints/openapi"
            );
            assert!(config.service_key_env.is_none());
            assert!(config.credential.is_none());
        }
        other => panic!("expected credential auth, got {other:?}"),
    }
}

#[test]
fn sap_ai_core_credential_issuer_resolves_service_key_endpoint() {
    let path = write_temp_config(
        r#"
[providers.sap]
default_model = "openai/anthropic/claude-sonnet-4"

[providers.sap.auth]
mode = "credential"
issuer = "sap_ai_core"
base_url = "https://api.example.com/v2"
service_key_env = "AICORE_SERVICE_KEY"

[providers.sap.adapters.openai]
enabled = true
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
        .get("sap")
        .expect("sap provider should exist");

    match &provider.auth {
        ProviderAuthConfig::Credential(config) => {
            assert_eq!(
                config.issuer,
                crate::provider::auth::CredentialIssuer::SapAiCore
            );
            assert_eq!(
                config.base_url.as_deref(),
                Some("https://api.example.com/v2")
            );
            assert_eq!(
                config.service_key_env.as_deref(),
                Some("AICORE_SERVICE_KEY")
            );
            assert!(config.credential.is_none());
        }
        other => panic!("expected credential auth, got {other:?}"),
    }
}

#[test]
fn sap_ai_core_legacy_mode_with_api_key_normalizes_to_api_auth() {
    let path = write_temp_config(
        r#"
[providers.sap]
default_model = "openai/anthropic/claude-sonnet-4"

[providers.sap.auth]
mode = "sap_ai_core"
base_url = "https://api.example.com/v2"
api_key = "sap-api-token"

[providers.sap.adapters.openai]
enabled = true
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
        .get("sap")
        .expect("sap provider should exist");

    match &provider.auth {
        ProviderAuthConfig::Api(config) => {
            assert_eq!(
                config.base_url.as_deref(),
                Some("https://api.example.com/v2")
            );
            assert_eq!(config.api_key.as_deref(), Some("sap-api-token"));
        }
        other => panic!("expected api auth, got {other:?}"),
    }
}

#[test]
fn openai_provider_rejects_mismatched_credential_issuer_and_backend() {
    let path = write_temp_config(
        r#"
[providers.bad]
default_model = "openai/gpt-4o-mini"

[providers.bad.auth]
mode = "credential"
issuer = "github_copilot"

[providers.bad.adapters.openai]
enabled = true
backend = "chatgpt_codex"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("mismatched credential issuer/backend should be rejected");
    assert!(matches!(
        err,
        ConfigError::InvalidProviderConfig { provider_id, .. } if provider_id == "bad"
    ));
}

#[test]
fn anthropic_provider_with_github_copilot_credential_uses_credential_auth() {
    let path = write_temp_config(
        r#"
[providers."github-copilot-claude"]
default_model = "anthropic/claude-sonnet-4"

[providers."github-copilot-claude".auth]
mode = "credential"
issuer = "github_copilot"

[providers."github-copilot-claude".adapters.anthropic]
enabled = true
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
        .get("github-copilot-claude")
        .expect("provider should exist");

    match &provider.auth {
        ProviderAuthConfig::Credential(config) => {
            assert_eq!(
                config.issuer,
                crate::provider::auth::CredentialIssuer::GithubCopilot
            );
        }
        other => panic!("expected credential auth, got {other:?}"),
    }
    match provider
        .adapters
        .get("anthropic")
        .expect("anthropic adapter should exist")
        .definition
    {
        ProviderAdapterDefinition::Anthropic(_) => {}
        ref other => panic!("expected anthropic adapter, got {other:?}"),
    }
}

#[test]
fn anthropic_provider_rejects_non_copilot_credential_issuer() {
    let path = write_temp_config(
        r#"
[providers.bad]
default_model = "anthropic/claude-sonnet-4"

[providers.bad.auth]
mode = "credential"
issuer = "openai_chatgpt"

[providers.bad.adapters.anthropic]
enabled = true
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("mismatched credential issuer should be rejected");
    assert!(matches!(
        err,
        ConfigError::InvalidProviderConfig { provider_id, .. } if provider_id == "bad"
    ));
}

#[test]
fn gemini_adapter_rejects_github_copilot_credential() {
    let path = write_temp_config(
        r#"
[providers.bad]
default_model = "gemini/gemini-2.5-pro"

[providers.bad.auth]
mode = "credential"
issuer = "github_copilot"

[providers.bad.adapters.gemini]
enabled = true
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("copilot credential should reject gemini adapter");
    assert!(matches!(
        err,
        ConfigError::InvalidProviderConfig { provider_id, message }
            if provider_id == "bad" && message.contains("use `openai`")
    ));
}

#[test]
fn multi_adapter_provider_loads_shared_auth_and_routes_models() {
    let path = write_temp_config(
        r#"
[providers.shared]
default_adapter = "openai"
default_model = "gpt-4.1-mini"

[providers.shared.auth]
mode = "api"
base_url = "https://gateway.example.com/v1"
api_key_env = "SHARED_GATEWAY_API_KEY"

[providers.shared.adapters.openai]
enabled = true

[providers.shared.adapters.openai.models."gpt-4.1-mini".thinking_modes.deep]
thinking = { type = "effort", effort = "high" }

[providers.shared.adapters.anthropic]
enabled = true

[providers.shared.adapters.anthropic.models."claude-sonnet-4"]
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
        .get("shared")
        .expect("shared provider should exist");

    assert_eq!(provider.default_adapter, "openai");
    assert_eq!(provider.default_model, "gpt-4.1-mini");
    match &provider.auth {
        ProviderAuthConfig::Api(api) => {
            assert_eq!(api.base_url.as_deref(), Some("https://gateway.example.com"));
            assert_eq!(api.api_key_env.as_deref(), Some("SHARED_GATEWAY_API_KEY"));
        }
        other => panic!("expected shared api auth, got {other:?}"),
    }
    assert_eq!(provider.adapters.len(), 2);
    assert_eq!(
        provider
            .models
            .get("openai/gpt-4.1-mini")
            .map(|model| model.enabled),
        Some(true)
    );
    assert!(
        provider
            .models
            .get("openai/gpt-4.1-mini")
            .and_then(|model| model.definition.thinking_modes.get("deep"))
            .is_some()
    );
    assert!(provider.models.contains_key("anthropic/claude-sonnet-4"));
}

#[test]
fn multi_adapter_provider_supports_shared_protocol_paths() {
    let path = write_temp_config(
        r#"
[providers.shared]
default_adapter = "openai"
default_model = "gpt-4.1-mini"

[providers.shared.auth]
mode = "api"
base_url = "https://gateway.example.com"
api_key_env = "SHARED_GATEWAY_API_KEY"

[providers.shared.auth.protocol_paths]
openai = "/api/provider/openai/v1"
anthropic = "/api/provider/anthropic/v1"
gemini = "/api/provider/google/v1beta"

[providers.shared.adapters.openai]
enabled = true

[providers.shared.adapters.anthropic]
enabled = true

[providers.shared.adapters.gemini]
enabled = true
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
        .get("shared")
        .expect("shared provider should exist");

    match &provider.auth {
        ProviderAuthConfig::Api(api) => {
            assert_eq!(api.protocol_paths.openai, "/api/provider/openai/v1");
            assert_eq!(api.protocol_paths.anthropic, "/api/provider/anthropic/v1");
            assert_eq!(api.protocol_paths.gemini, "/api/provider/google/v1beta");
        }
        other => panic!("expected api auth, got {other:?}"),
    }
}

#[test]
fn opencode_go_provider_config_loads_protocol_routes() {
    let path = write_temp_config(
        r#"
[providers."opencode-go"]
default_adapter = "openai"
default_model = "kimi-k2.6"

[providers."opencode-go".auth]
mode = "api"
base_url = "https://opencode.ai/zen/go"
api_key_env = "OPENCODE_API_KEY"

[providers."opencode-go".adapters.openai]
enabled = true
api_mode = "chat"
models_url = "https://opencode.ai/zen/go/v1/models"

[providers."opencode-go".adapters.openai.models."minimax-m2.7"]
enabled = false

[providers."opencode-go".adapters.anthropic]
enabled = true
messages_url = "https://opencode.ai/zen/go/v1/messages"
models_url = "https://opencode.ai/zen/go/v1/models"

[providers."opencode-go".adapters.anthropic.models."minimax-m2.7"]
enabled = true

[providers."opencode-go".adapters.anthropic.models."kimi-k2.6"]
enabled = false
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
        .get("opencode-go")
        .expect("opencode-go provider should exist");

    assert_eq!(provider.default_adapter, "openai");
    assert_eq!(provider.default_model, "kimi-k2.6");
    match &provider.auth {
        ProviderAuthConfig::Api(api) => {
            assert_eq!(api.base_url.as_deref(), Some("https://opencode.ai/zen/go"));
            assert_eq!(api.protocol_paths.openai, "/v1");
            assert_eq!(api.protocol_paths.anthropic, "/v1");
            assert_eq!(api.protocol_paths.gemini, "/v1beta");
            assert_eq!(api.api_key_env.as_deref(), Some("OPENCODE_API_KEY"));
        }
        other => panic!("expected opencode-go api auth, got {other:?}"),
    }

    match &provider.adapters["openai"].definition {
        ProviderAdapterDefinition::OpenAi(config) => {
            assert_eq!(config.options.api_mode, OpenAiApiModeConfig::Chat);
            assert_eq!(
                config.options.models_url.as_deref(),
                Some("https://opencode.ai/zen/go/v1/models")
            );
            assert_eq!(config.options.auth_header, "authorization");
            assert_eq!(config.options.auth_scheme.as_deref(), Some("Bearer"));
        }
        other => panic!("expected openai adapter, got {other:?}"),
    }

    match &provider.adapters["anthropic"].definition {
        ProviderAdapterDefinition::Anthropic(config) => {
            assert_eq!(
                config.options.messages_url.as_deref(),
                Some("https://opencode.ai/zen/go/v1/messages")
            );
            assert_eq!(
                config.options.models_url.as_deref(),
                Some("https://opencode.ai/zen/go/v1/models")
            );
            assert_eq!(config.options.auth_header, "x-api-key");
            assert_eq!(config.options.auth_scheme, None);
        }
        other => panic!("expected anthropic adapter, got {other:?}"),
    }

    assert_eq!(
        provider
            .models
            .get("openai/minimax-m2.7")
            .map(|model| model.enabled),
        Some(false)
    );
    assert_eq!(
        provider
            .models
            .get("anthropic/minimax-m2.7")
            .map(|model| model.enabled),
        Some(true)
    );
    assert_eq!(
        provider
            .models
            .get("anthropic/kimi-k2.6")
            .map(|model| model.enabled),
        Some(false)
    );
}

#[test]
fn opencode_free_provider_config_uses_public_key_and_configured_only_models() {
    let path = write_temp_config(
        r#"
[providers."opencode-free"]
default_adapter = "openai"
default_model = "deepseek-v4-flash-free"

[providers."opencode-free".auth]
mode = "api"
base_url = "https://opencode.ai/zen"
api_key = "public"

[providers."opencode-free".auth.protocol_paths]
gemini = "/v1"

[providers."opencode-free".adapters.openai]
enabled = true
api_mode = "chat"
model_discovery = "configured_only"

[providers."opencode-free".adapters.openai.models."deepseek-v4-flash-free"]
enabled = true

[providers."opencode-free".adapters.anthropic]
enabled = true
model_discovery = "configured_only"

[providers."opencode-free".adapters.anthropic.models."minimax-m2.5-free"]
enabled = true
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
        .get("opencode-free")
        .expect("opencode-free provider should exist");

    match &provider.auth {
        ProviderAuthConfig::Api(api) => {
            assert_eq!(api.base_url.as_deref(), Some("https://opencode.ai/zen"));
            assert_eq!(api.protocol_paths.openai, "/v1");
            assert_eq!(api.protocol_paths.anthropic, "/v1");
            assert_eq!(api.protocol_paths.gemini, "/v1");
            assert_eq!(api.api_key.as_deref(), Some("public"));
        }
        other => panic!("expected opencode-free api auth, got {other:?}"),
    }

    let openai = provider
        .adapters
        .get("openai")
        .expect("openai adapter should exist");
    assert_eq!(
        openai.model_discovery,
        ProviderModelDiscoveryConfig::ConfiguredOnly
    );
    match &openai.definition {
        ProviderAdapterDefinition::OpenAi(config) => {
            assert_eq!(config.options.api_mode, OpenAiApiModeConfig::Chat);
        }
        other => panic!("expected openai adapter, got {other:?}"),
    }

    let anthropic = provider
        .adapters
        .get("anthropic")
        .expect("anthropic adapter should exist");
    assert_eq!(
        anthropic.model_discovery,
        ProviderModelDiscoveryConfig::ConfiguredOnly
    );

    assert_eq!(
        provider
            .models
            .get("openai/deepseek-v4-flash-free")
            .map(|model| model.enabled),
        Some(true)
    );
    assert_eq!(
        provider
            .models
            .get("anthropic/minimax-m2.5-free")
            .map(|model| model.enabled),
        Some(true)
    );
}

#[test]
fn multi_adapter_provider_allows_passthrough_models_without_explicit_routes() {
    let path = write_temp_config(
        r#"
[providers.shared]
default_adapter = "openai"
default_model = "gpt-4.1"

[providers.shared.auth]
mode = "api"
base_url = "https://gateway.example.com/v1"
api_key_env = "SHARED_GATEWAY_API_KEY"

[providers.shared.adapters.openai]
enabled = true

[providers.shared.adapters.anthropic]
enabled = true

[providers.shared.adapters.anthropic.models."claude-sonnet-4"]
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("multi-adapter provider should allow passthrough models");

    let provider = resolution
        .config
        .providers
        .get("shared")
        .expect("shared provider should exist");
    assert_eq!(provider.default_adapter, "openai");
    assert_eq!(provider.default_model, "gpt-4.1");
    assert!(provider.models.contains_key("anthropic/claude-sonnet-4"));
}

#[test]
fn multiple_providers_keep_distinct_inline_credentials() {
    let path = write_temp_config(
        r#"
[providers.primary]
default_model = "openai/gpt-4.1"

[providers.primary.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-primary"

[providers.primary.adapters.openai]
enabled = true

[providers.secondary]
default_model = "openai/gpt-4.1-mini"

[providers.secondary.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-secondary"

[providers.secondary.adapters.openai]
enabled = true
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("config should load");

    let primary = resolution
        .config
        .providers
        .get("primary")
        .expect("primary provider should exist");
    let secondary = resolution
        .config
        .providers
        .get("secondary")
        .expect("secondary provider should exist");

    match &primary.auth {
        ProviderAuthConfig::Api(api) => {
            assert_eq!(api.api_key.as_deref(), Some("sk-primary"));
        }
        other => panic!("expected api auth, got {other:?}"),
    }

    match &secondary.auth {
        ProviderAuthConfig::Api(api) => {
            assert_eq!(api.api_key.as_deref(), Some("sk-secondary"));
        }
        other => panic!("expected api auth, got {other:?}"),
    }
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
    let err = RawConfig::from_env(&env).expect_err("legacy provider env overrides should fail");
    assert!(
        matches!(err, ConfigError::Validation(message) if message.contains("AGENA_PROVIDER__GOOGLE_VERTEX__KIND is no longer supported"))
    );
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
fn provider_auth_credential_inline_config_loads() {
    let path = write_temp_config(
        r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.adapters.openai]
enabled = true
[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-inline"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("provider auth config should load");

    let provider = resolution
        .config
        .providers
        .get("openai")
        .expect("openai provider should exist");
    match &provider.auth {
        ProviderAuthConfig::Api(api) => {
            assert_eq!(api.base_url.as_deref(), Some("https://api.openai.com"));
            assert_eq!(api.api_key.as_deref(), Some("sk-inline"));
        }
        other => panic!("unexpected auth config: {other:?}"),
    }
}

#[test]
fn provider_adapter_and_model_enable_defaults_are_canonical() {
    let path = write_temp_config(
        r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-test"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.anthropic]

[providers.openai.adapters.anthropic.models."claude-sonnet-4"]
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
    assert!(provider.enabled);
    assert!(
        provider
            .adapters
            .get("openai")
            .expect("openai adapter should exist")
            .enabled
    );
    assert!(
        !provider
            .adapters
            .get("anthropic")
            .expect("anthropic adapter should exist")
            .enabled
    );
    assert_eq!(
        provider
            .models
            .get("anthropic/claude-sonnet-4")
            .map(|model| model.enabled),
        Some(true)
    );
}

#[test]
fn memory_project_instruction_config_loads() {
    let path = write_temp_config(
        r#"
[plugins.list."agena.memory"]
kind = "static"

[plugins.list."agena.memory".options.project_instructions]
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
[plugins.list."agena.hooks"]
kind = "static"

[[plugins.list."agena.hooks".options.hooks]]
event = "user_prompt_submit"
command = "python3 .agena/hooks/enrich.py"
timeout_ms = 3000

[[plugins.list."agena.hooks".options.hooks]]
event = "pre_tool_use"
command = "python3 .agena/hooks/check_tool.py"
timeout_ms = 5000
matcher = { tool = "bash" }

[[plugins.list."agena.hooks".options.hooks]]
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
[plugins.list."agena.hooks"]
kind = "static"

[[plugins.list."agena.hooks".options.hooks]]
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
fn top_level_plugin_backed_config_is_rejected() {
    for (label, raw) in [
        (
            "memory",
            r#"
[memory.project_instructions]
enabled = false
"#,
        ),
        (
            "hooks",
            r#"
[[hooks]]
event = "user_prompt_submit"
command = "echo"
"#,
        ),
        (
            "mcp",
            r#"
[mcp.servers.docs]
transport = "stdio"
command = "mcp-docs"
"#,
        ),
        (
            "lsp",
            r#"
[lsp.servers.rust]
command = "rust-analyzer"
"#,
        ),
        (
            "web",
            r#"
[web.search]
backend = "duckduckgo_html"
"#,
        ),
    ] {
        let path = write_temp_config(raw);
        let loader = ConfigLoader::new(TestEnvironment::default());
        let err = match loader.load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        }) {
            Ok(_) => panic!("{label} top-level config should fail"),
            Err(err) => err,
        };
        assert!(
            matches!(err, ConfigError::Validation(_)),
            "{label} should fail validation, got {err:?}"
        );
    }
}

#[test]
fn plugin_options_load_mcp_lsp_and_web_config() {
    let path = write_temp_config(
        r#"
[plugins.list."agena.mcp"]
kind = "static"

[plugins.list."agena.mcp".options.servers.docs]
transport = "stdio"
command = "mcp-docs"
args = ["--repo", "."]

[plugins.list."agena.lsp"]
kind = "static"

[plugins.list."agena.lsp".options.servers.rust]
command = "rust-analyzer"
file_extensions = ["rs"]
root_markers = ["Cargo.toml"]

[plugins.list."agena.web"]
kind = "static"

[plugins.list."agena.web".options.search]
backend = "brave"
brave_api_key = "secret"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("plugin-backed config should load");

    assert!(resolution.config.mcp.servers.contains_key("docs"));
    assert_eq!(
        resolution
            .config
            .lsp
            .servers
            .get("rust")
            .map(|server| server.command.as_str()),
        Some("rust-analyzer")
    );
    assert_eq!(
        resolution.config.web.search.backend,
        crate::config::WebSearchBackendKind::Brave
    );
}

#[test]
fn runtime_plugin_options_do_not_require_static_kind() {
    let path = write_temp_config(
        r#"
[plugins.list."agena.web"]
kind = "stdio"
command = "web-plugin"

[plugins.list."agena.web".options.search]
backend = "brave"
brave_api_key = "secret"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("runtime-managed plugin options should load");
    assert_eq!(
        resolution.config.web.search.backend,
        crate::config::WebSearchBackendKind::Brave
    );
}

#[test]
fn resolved_config_serializes_plugin_backed_options_under_plugins() {
    let path = write_temp_config(
        r#"
[plugins.list."agena.memory"]
kind = "static"

[plugins.list."agena.memory".options.project_instructions]
enabled = false
include_global = false

[plugins.list."agena.mcp"]
kind = "static"

[plugins.list."agena.mcp".options.servers.docs]
transport = "stdio"
command = "mcp-docs"

[plugins.list."agena.web"]
kind = "static"

[plugins.list."agena.web".options.search]
backend = "duck_duck_go_html"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("plugin-backed config should load");
    let serialized = resolution
        .render(ConfigOutputFormat::Toml)
        .expect("resolved config should serialize");

    assert!(serialized.contains("[config.plugins.list.\"agena.memory\""));
    assert!(serialized.contains("[config.plugins.list.\"agena.mcp\""));
    assert!(serialized.contains("[config.plugins.list.\"agena.web\""));
    assert!(!serialized.contains("[config.memory"));
    assert!(!serialized.contains("[config.mcp"));
    assert!(!serialized.contains("[config.lsp"));
    assert!(!serialized.contains("[config.web"));
    assert!(!serialized.contains("[[config.hooks"));
}

#[test]
fn provider_models_parse() {
    let path = write_temp_config(
        r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-test"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-4.1-mini"]
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
        .get("openai/gpt-4.1-mini")
        .expect("configured model should exist");
    assert_eq!(
        model.definition.capabilities.image_input,
        Some(crate::provider::CapabilitySupport::Unsupported)
    );
}

#[test]
fn provider_models_allow_empty_configuration() {
    let path = write_temp_config(
        r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-test"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-4.1-mini"]
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("empty model config should be allowed");

    let provider = resolution
        .config
        .providers
        .get("openai")
        .expect("openai provider should exist");
    assert!(provider.models.contains_key("openai/gpt-4.1-mini"));
}

#[test]
fn provider_models_reject_overlapping_compact_capabilities() {
    let path = write_temp_config(
        r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-test"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-4.1-mini"]
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
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-test"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-4.1-mini"]
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

    assert!(
        serialized.contains(
            "[config.providers.openai.models.\"openai/gpt-4.1-mini\".definition.capabilities.input]"
        ) || serialized.contains("[config.providers.openai.models.\"openai/gpt-4.1-mini\".input]")
    );
    assert!(serialized.contains("unsupported = [\"image\"]"));
    assert!(serialized.contains("features = [\"tool_calling\"]"));
    assert!(!serialized.contains("image_input = \"unsupported\""));
    assert!(!serialized.contains("tool_calling = \"supported\""));
}

#[test]
fn hook_entries_load_from_toml() {
    let path = write_temp_config(
        r#"
[plugins.list."agena.hooks"]
kind = "static"

[[plugins.list."agena.hooks".options.hooks]]
event = "user_prompt_submit"
command = "echo $AGENA_PROMPT"

[[plugins.list."agena.hooks".options.hooks]]
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
[permission.entries]
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
[permission.entries]
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
fn loader_rejects_removed_plugin_wasm_sandbox_field() {
    let path = write_temp_config(
        r#"
[plugins.list.tool]
kind = "wasm"
path = "./tool.wasm"

[plugins.list.tool.sandbox]
allow_fs_read = ["/repo"]
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("removed wasm sandbox config should fail");
    assert!(matches!(err, ConfigError::ParseFile { .. }));
}

#[test]
fn cli_override_parser_rejects_removed_permission_overrides() {
    let err = "permission.default_write=ask"
        .parse::<ConfigOverride>()
        .expect_err("permission override should be rejected");
    assert!(matches!(err, ConfigError::InvalidOverride(_)));
}

#[tokio::test]
async fn build_plugin_host_with_no_entries_succeeds() {
    let dir = temp_dir("plugins-empty");
    let path = dir.join("config.toml");
    fs::write(
        &path,
        r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-test"

[providers.openai.adapters.openai]
enabled = true

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
    assert_eq!(host.plugins().len(), 10);
    let ids: Vec<&str> = host.plugins().iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&crate::memory::memory_plugin_id()));
    assert!(ids.contains(&crate::hooks::ShellHookPlugin::id()));
    assert!(ids.contains(&crate::tool::skills_plugin_id()));
    assert!(ids.contains(&crate::tool::lsp_plugin_id()));
    assert!(ids.contains(&crate::tool::cron_plugin_id()));
    assert!(ids.contains(&crate::tool::fs_plugin_id()));
    assert!(ids.contains(&crate::tool::settings_plugin_id()));
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
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "sk-test"

[providers.openai.adapters.openai]
enabled = true
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
    // The bogus cdylib entry is skipped; only the provided in-process
    // plugins plus runtime support plugins remain.
    assert_eq!(host.plugins().len(), 10);
    let ids: Vec<&str> = host.plugins().iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&crate::memory::memory_plugin_id()));
    assert!(ids.contains(&crate::hooks::ShellHookPlugin::id()));
    assert!(ids.contains(&crate::tool::skills_plugin_id()));
    assert!(ids.contains(&crate::tool::lsp_plugin_id()));
    assert!(ids.contains(&crate::tool::cron_plugin_id()));
    assert!(ids.contains(&crate::tool::fs_plugin_id()));
    assert!(ids.contains(&crate::tool::settings_plugin_id()));
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
[default]
agent = "planner"

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

[permission.entries.tags]
network = "ask"

[permission.entries.names]
bash = "ask"

[agents.planner]
description = "Planning agent"
prompt = "You are a planner."
allowed_entries = ["read", "grep"]
default = { provider = "openai", adapter = "openai", model = "gpt-5" }
aliases = ["plan"]

[agents.planner.permission.inherit]
path = true
network = true
entries = true

[agents.planner.permission.path]
workspace = { read = "allow", write = "deny" }

[agents.planner.permission.path.rules]
"<cwd>/docs/**" = { read = "allow", write = "ask" }

[agents.planner.permission.entries.names]
todo_write = "allow"

[agents.planner.permission.entries.rules.bash]
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

    assert_eq!(resolution.config.default.agent, "planner");
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
            .and_then(|tools| tools.names.get("todo_write")),
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
    assert_eq!(planner.default.provider.as_deref(), Some("openai"));
    assert_eq!(planner.default.adapter.as_deref(), Some("openai"));
    assert_eq!(planner.default.model.as_deref(), Some("gpt-5"));
    assert_eq!(planner.aliases, vec!["plan"]);
    assert!(!planner.disabled);
}

#[test]
fn default_agent_falls_back_to_build() {
    let path = write_temp_config(
        r#"
[providers.openai]
default_model = "openai/gpt-5"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("config should load");

    assert_eq!(resolution.config.default.agent, "build");
}

#[test]
fn runtime_default_agent_is_rejected() {
    let path = write_temp_config(
        r#"
[runtime]
default_agent = "planner"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("runtime.default_agent should be rejected");

    assert!(matches!(err, ConfigError::ParseFile { .. }));
}

#[test]
fn default_section_sets_global_defaults_and_provider_route() {
    let path = write_temp_config(
        r#"
[default]
provider = "openai"
adapter = "openai"
model = "gpt-5"
agent = "planner"

[providers.openai]

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
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
        resolution.config.default.provider.as_deref(),
        Some("openai")
    );
    assert_eq!(resolution.config.default.adapter.as_deref(), Some("openai"));
    assert_eq!(resolution.config.default.model.as_deref(), Some("gpt-5"));
    assert_eq!(resolution.config.default.agent, "planner");
    assert_eq!(
        resolution
            .config
            .providers
            .get("openai")
            .expect("provider should resolve")
            .default_model,
        "gpt-5"
    );
}

#[test]
fn default_adapter_without_default_model_must_reference_enabled_adapter() {
    let path = write_temp_config(
        r#"
[default]
provider = "openai"
adapter = "anthropic"

[providers.openai]
default_adapter = "openai"
default_model = "gpt-5"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.anthropic]
enabled = false
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("disabled default adapter should be rejected");

    assert!(
        err.to_string()
            .contains("default.adapter `anthropic` references disabled adapter")
    );
}

#[test]
fn legacy_amazon_bedrock_root_shape_is_rejected() {
    let path = write_temp_config(
        r#"
[providers.bedrock]
kind = "amazon_bedrock"
base_url = "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1"
default_model = "amazon_bedrock/amazon.nova-pro-v1:0"
api_key = "bedrock-token"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("legacy bedrock root shape should be rejected");

    assert!(matches!(err, ConfigError::ParseFile { .. }));
}

#[test]
fn unknown_openai_compatible_adapter_is_rejected() {
    let path = write_temp_config(
        r#"
[providers.gateway]
default_model = "openai/gpt-4.1-mini"

[providers.gateway.auth]
mode = "api"
base_url = "https://gateway.example.com/v1"
api_key = "secret"

[providers.gateway.adapters.openai_compatible]
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("openai_compatible adapter should be rejected");

    assert!(matches!(
        err,
        ConfigError::MissingProviderKind { provider_id } if provider_id == "gateway"
    ));
}

#[test]
fn openai_adapter_rejects_removed_openai_compatible_capability_family() {
    let path = write_temp_config(
        r#"
[providers.gateway]
default_model = "openai/gpt-4.1-mini"

[providers.gateway.auth]
mode = "api"
base_url = "https://gateway.example.com/v1"
api_key = "secret"

[providers.gateway.adapters.openai]
enabled = true
capability_family = "openai_compatible"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("removed capability family should be rejected during parse");

    assert!(matches!(err, ConfigError::ParseFile { .. }));
}

#[test]
fn http_adapters_reject_adapter_level_base_url() {
    for adapter in ["openai", "anthropic", "gemini"] {
        let path = write_temp_config(
            format!(
                r#"
[providers.gateway]
default_model = "{adapter}/test-model"

[providers.gateway.auth]
mode = "api"
base_url = "https://gateway.example.com"
api_key = "secret"

[providers.gateway.adapters.{adapter}]
enabled = true
base_url = "https://override.example.com"
"#
            )
            .as_str(),
        );

        let loader = ConfigLoader::new(TestEnvironment::default());
        let err = loader
            .load(&LoadConfigRequest {
                config_path: Some(path),
                ..LoadConfigRequest::default()
            })
            .expect_err("adapter-level base_url should be rejected");

        assert!(matches!(
            err,
            ConfigError::InvalidProviderConfig { provider_id, message }
                if provider_id == "gateway" && message.contains("does not support `base_url`")
        ));
    }
}

#[test]
fn provider_model_rejects_removed_legacy_fields() {
    let path = write_temp_config(
        r#"
[providers.gateway]
default_model = "openai/gpt-4.1-mini"

[providers.gateway.auth]
mode = "api"
base_url = "https://gateway.example.com/v1"
api_key = "secret"

[providers.gateway.adapters.openai]
enabled = true

[providers.gateway.adapters.openai.models."gpt-4.1-mini"]
target_model = "gpt-4.1-mini"
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("legacy model fields should be rejected");

    assert!(
        matches!(err, ConfigError::Validation(message) if message.contains("does not support `target_model`"))
    );
}

#[test]
fn canonical_amazon_bedrock_adapter_keeps_sigv4_auth() {
    let path = write_temp_config(
        r#"
[providers.bedrock]
default_model = "amazon_bedrock/anthropic.claude-3-7-sonnet-20250219-v1:0"

[providers.bedrock.auth]
mode = "bedrock_sigv4"
base_url = "https://bedrock-runtime.us-east-1.amazonaws.com"
region = "us-east-1"
profile = "prod"

[providers.bedrock.adapters.amazon_bedrock]
enabled = true
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
        .get("bedrock")
        .expect("bedrock provider should exist");

    match &provider.auth {
        ProviderAuthConfig::BedrockSigv4(sigv4) => {
            assert_eq!(
                sigv4.base_url,
                "https://bedrock-runtime.us-east-1.amazonaws.com"
            );
            assert_eq!(sigv4.region, "us-east-1");
            assert_eq!(sigv4.profile.as_deref(), Some("prod"));
        }
        other => panic!("expected bedrock sigv4 auth, got {other:?}"),
    }

    let adapter = provider
        .adapters
        .get("amazon_bedrock")
        .expect("amazon_bedrock adapter should exist");
    assert!(matches!(
        adapter.definition,
        ProviderAdapterDefinition::AmazonBedrock(_)
    ));
}

#[test]
fn gitlab_api_auth_allows_missing_base_url() {
    let path = write_temp_config(
        r#"
[providers.gitlab_token]
default_model = "gitlab/duo-chat"

[providers.gitlab_token.auth]
mode = "api"
api_key_env = "GITLAB_TOKEN"

[providers.gitlab_token.adapters.gitlab]
enabled = true
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let resolution = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect("gitlab api auth should load without base_url");

    let provider = resolution
        .config
        .providers
        .get("gitlab_token")
        .expect("gitlab_token provider should exist");

    match &provider.auth {
        ProviderAuthConfig::Api(api) => {
            assert!(api.base_url.is_none());
            assert_eq!(api.api_key_env.as_deref(), Some("GITLAB_TOKEN"));
        }
        other => panic!("expected api auth, got {other:?}"),
    }
    assert!(matches!(
        provider
            .adapters
            .get("gitlab")
            .expect("gitlab adapter should exist")
            .definition,
        ProviderAdapterDefinition::Gitlab(_)
    ));
}

#[test]
fn http_api_auth_requires_base_url() {
    let path = write_temp_config(
        r#"
[providers.gateway]
default_model = "openai/gpt-5.4"

[providers.gateway.auth]
mode = "api"
api_key_env = "OPENAI_API_KEY"

[providers.gateway.adapters.openai]
enabled = true
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("openai api auth should require base_url");

    assert!(matches!(
        err,
        ConfigError::InvalidProviderConfig { provider_id, message }
            if provider_id == "gateway"
                && message.contains("requires `base_url` for `openai` adapters")
    ));
}

#[test]
fn api_auth_rejects_amazon_bedrock_adapter() {
    let path = write_temp_config(
        r#"
[providers.bad_bedrock]
default_model = "amazon_bedrock/anthropic.claude-3-7-sonnet-20250219-v1:0"

[providers.bad_bedrock.auth]
mode = "api"
base_url = "https://bedrock-runtime.us-east-1.amazonaws.com"
api_key_env = "BEDROCK_TOKEN"

[providers.bad_bedrock.adapters.amazon_bedrock]
enabled = true
"#,
    );

    let loader = ConfigLoader::new(TestEnvironment::default());
    let err = loader
        .load(&LoadConfigRequest {
            config_path: Some(path),
            ..LoadConfigRequest::default()
        })
        .expect_err("amazon_bedrock should reject api auth");

    assert!(matches!(
        err,
        ConfigError::InvalidProviderConfig { provider_id, message }
            if provider_id == "bad_bedrock"
                && message.contains("api auth is not supported by `amazon_bedrock` adapters")
    ));
}
