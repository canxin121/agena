use super::*;
use agena_provider::{ProviderModelDiscoveryConfig, ProviderSecretSourceConfig};
use agena_runtime_config::{OpenAiResponsesProviderOptions, ProcessEnvironment};

fn gateway() -> ResolvedProviderConfig {
    ResolvedProviderConfig {
        enabled: true,
        auth: ProviderAuthConfig::Api(ProviderApiAuthConfig::custom(
            Some("http://127.0.0.1:8317/gateway".into()),
            ProviderProtocolPathsConfig {
                openai: "/responses-api/v1".into(),
                anthropic: "/messages-api/v1".into(),
                gemini: "/google-api/v1beta".into(),
            },
            Some(ProviderSecretSourceConfig::Inline(
                "test-gateway-key".into(),
            )),
        )),
        network: Default::default(),
        adapters: BTreeMap::new(),
        models: BTreeMap::new(),
    }
}

#[tokio::test]
async fn tool_protocol_uses_current_provider_paths_and_key_without_a_chat_adapter() {
    let provider = gateway();
    for (protocol, path, header, value) in [
        (
            "chatgpt",
            "responses-api/v1",
            "authorization",
            "Bearer test-gateway-key",
        ),
        ("claude", "messages-api/v1", "x-api-key", "test-gateway-key"),
        (
            "gemini",
            "google-api/v1beta",
            "x-goog-api-key",
            "test-gateway-key",
        ),
    ] {
        let connection = resolve_provider_tool_connection(
            "agena-tool-gateway-test",
            &provider,
            protocol,
            None,
            "chat-model",
            &ProcessEnvironment,
            &Default::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            connection.base_url,
            format!("http://127.0.0.1:8317/gateway/{path}")
        );
        assert_eq!(connection.headers[header], value);
        // The current session model is the default even without a matching
        // protocol adapter. Tool input.model can explicitly override it.
        assert_eq!(connection.model.as_deref(), Some("chat-model"));
    }
}

#[tokio::test]
async fn selected_provider_adapter_controls_auth_headers_and_default_model() {
    let mut provider = gateway();
    provider.adapters.insert(
        "responses".into(),
        ResolvedProviderAdapterConfig {
            enabled: true,
            model_discovery: ProviderModelDiscoveryConfig::ConfiguredOnly,
            definition: ProviderAdapterDefinition::OpenAiResponses(HttpProviderAdapterConfig {
                user_agent: Some("gateway-client".into()),
                extra_headers: BTreeMap::from([("X-Tenant".into(), "tenant-a".into())]),
                options: OpenAiResponsesProviderOptions {
                    backend: Default::default(),
                    models_url: None,
                    auth_header: "X-Gateway-Token".into(),
                    auth_scheme: Some("Token".into()),
                    capability_family: None,
                },
            }),
        },
    );
    let connection = resolve_provider_tool_connection(
        "agena-tool-adapter-test",
        &provider,
        "chatgpt",
        Some("responses"),
        "team/model-alias",
        &ProcessEnvironment,
        &Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(connection.model.as_deref(), Some("team/model-alias"));
    assert_eq!(
        connection.headers["x-gateway-token"],
        "Token test-gateway-key"
    );
    assert_eq!(connection.headers["x-tenant"], "tenant-a");
    assert_eq!(connection.headers["user-agent"], "gateway-client");
    assert!(!connection.headers.contains_key("authorization"));

    provider.models.insert(
        "responses/other-configured-model".into(),
        serde_json::from_value(serde_json::json!({})).unwrap(),
    );
    // A tool using another protocol still defaults to the actual session
    // model, even when this protocol has a different configured model.
    for protocol in ["chatgpt", "claude", "gemini"] {
        let connection = resolve_provider_tool_connection(
            "agena-tool-adapter-test",
            &provider,
            protocol,
            Some("another-session-adapter"),
            "current-session-model",
            &ProcessEnvironment,
            &Default::default(),
        )
        .await
        .unwrap();
        assert_eq!(connection.model.as_deref(), Some("current-session-model"));
    }
}

#[tokio::test]
async fn missing_provider_endpoint_never_falls_back_to_a_vendor() {
    let mut provider = gateway();
    provider.auth = ProviderAuthConfig::None;
    let result = resolve_provider_tool_connection(
        "agena-tool-missing-endpoint",
        &provider,
        "chatgpt",
        None,
        "model",
        &ProcessEnvironment,
        &Default::default(),
    )
    .await;
    assert!(result.is_err());
}

#[test]
fn gateway_sdk_url_replaces_the_existing_protocol_suffix() {
    let auth = ProviderAuthConfig::Api(ProviderApiAuthConfig::custom(
        Some("http://127.0.0.1:8317/proxy/v1/".into()),
        Default::default(),
        None,
    ));
    for (kind, expected) in [
        (HttpAdapterKind::OpenAi, "http://127.0.0.1:8317/proxy/v1"),
        (HttpAdapterKind::Anthropic, "http://127.0.0.1:8317/proxy/v1"),
        (
            HttpAdapterKind::Gemini,
            "http://127.0.0.1:8317/proxy/v1beta",
        ),
    ] {
        assert_eq!(
            tool_connection::tool_base_url("gateway", &auth, kind).unwrap(),
            expected
        );
    }
}

#[test]
fn model_discovery_permission_targets_include_custom_models_urls_and_skip_configured_only() {
    let mut provider = gateway();
    provider.adapters.insert(
        "responses".into(),
        ResolvedProviderAdapterConfig {
            enabled: true,
            model_discovery: ProviderModelDiscoveryConfig::default(),
            definition: ProviderAdapterDefinition::OpenAiResponses(HttpProviderAdapterConfig {
                user_agent: None,
                extra_headers: Default::default(),
                options: OpenAiResponsesProviderOptions {
                    backend: Default::default(),
                    models_url: Some("https://catalog.example/models".into()),
                    auth_header: "authorization".into(),
                    auth_scheme: Some("Bearer".into()),
                    capability_family: None,
                },
            }),
        },
    );
    let targets = provider_model_discovery_targets("gateway", &provider).unwrap();
    assert!(
        targets
            .iter()
            .any(|t| t == "https://catalog.example/models")
    );
    assert!(
        targets
            .iter()
            .any(|t| t.starts_with("http://127.0.0.1:8317/"))
    );
    provider
        .adapters
        .get_mut("responses")
        .unwrap()
        .model_discovery = ProviderModelDiscoveryConfig::ConfiguredOnly;
    assert!(
        provider_model_discovery_targets("gateway", &provider)
            .unwrap()
            .is_empty()
    );
}
