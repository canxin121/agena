use super::*;
use agena_provider::{
    AdapterDiscoveryStatus, ModelDiscoverySource, ProviderModelCatalog,
    ProviderModelDiscoveryConfig, ResolvedProviderModelConfig,
};
use agena_runtime_config::{
    AnthropicProviderOptions, GeminiProviderOptions, OpenAiResponsesProviderOptions,
    ProcessEnvironment,
};
use serde_json::json;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn discovery_follows_vendor_pages_and_preserves_all_routes_and_failures() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let count = socket.read(&mut chunk).await.unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            let path = request.split_whitespace().nth(1).unwrap().to_owned();
            observed.lock().unwrap().push(path.clone());
            let (status, body) = if path.starts_with("/failed/") {
                (
                    "503 Service Unavailable",
                    json!({"error":{"message":"do-not-expose-provider-secret"}}),
                )
            } else if path.starts_with("/custom/models") {
                if path.contains("after=") {
                    (
                        "200 OK",
                        json!({"data":[{"id":"gpt-image-2"}],"has_more":false}),
                    )
                } else {
                    (
                        "200 OK",
                        json!({"data":[{"id":"shared-model"},{"id":"disabled-model"}],"has_more":true,"last_id":"shared-model"}),
                    )
                }
            } else if path.starts_with("/messages/models") {
                if path.contains("after=") {
                    (
                        "200 OK",
                        json!({"data":[{"id":"claude-extra"}],"has_more":false}),
                    )
                } else {
                    (
                        "200 OK",
                        json!({"data":[{"id":"shared-model"}],"has_more":true,"last_id":"shared-model"}),
                    )
                }
            } else if path.starts_with("/v1beta/models") {
                if path.contains("pageToken=") {
                    ("200 OK", json!({"models":[{"name":"models/voice-model"}]}))
                } else {
                    (
                        "200 OK",
                        json!({"models":[{"name":"models/shared-model"}],"nextPageToken":"second-page"}),
                    )
                }
            } else {
                panic!("unexpected discovery request: {path}");
            };
            let body = body.to_string();
            socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
    });
    let openai = |url: String, configured_only: bool| ResolvedProviderAdapterConfig {
        enabled: true,
        model_discovery: if configured_only {
            ProviderModelDiscoveryConfig::ConfiguredOnly
        } else {
            Default::default()
        },
        definition: ProviderAdapterDefinition::OpenAiResponses(HttpProviderAdapterConfig {
            user_agent: None,
            extra_headers: Default::default(),
            options: OpenAiResponsesProviderOptions {
                backend: Default::default(),
                models_url: Some(url),
                auth_header: "authorization".into(),
                auth_scheme: Some("Bearer".into()),
                capability_family: None,
            },
        }),
    };
    let mut config = ResolvedProviderConfig {
        enabled: true,
        auth: ProviderAuthConfig::Api(ProviderApiAuthConfig::custom(
            Some(base.clone()),
            ProviderProtocolPathsConfig {
                openai: "/v1".into(),
                anthropic: "/v1".into(),
                gemini: "/v1beta".into(),
            },
            Some(agena_provider::ProviderSecretSourceConfig::Inline(
                "test-key".into(),
            )),
        )),
        network: Default::default(),
        models: BTreeMap::new(),
        adapters: BTreeMap::from([
            (
                "openai".into(),
                openai(format!("{base}/custom/models?tenant=a"), false),
            ),
            (
                "failed".into(),
                openai(format!("{base}/failed/models"), false),
            ),
            (
                "local".into(),
                openai("http://127.0.0.1:1/must-not-query".into(), true),
            ),
            (
                "anthropic".into(),
                ResolvedProviderAdapterConfig {
                    enabled: true,
                    model_discovery: Default::default(),
                    definition: ProviderAdapterDefinition::Anthropic(HttpProviderAdapterConfig {
                        user_agent: None,
                        extra_headers: Default::default(),
                        options: AnthropicProviderOptions {
                            models_url: Some(format!("{base}/messages/models?tenant=a")),
                            messages_url: None,
                            auth_header: "x-api-key".into(),
                            auth_scheme: None,
                            extra_beta_header: None,
                            eager_input_streaming: None,
                        },
                    }),
                },
            ),
            (
                "google".into(),
                ResolvedProviderAdapterConfig {
                    enabled: true,
                    model_discovery: Default::default(),
                    definition: ProviderAdapterDefinition::Gemini(HttpProviderAdapterConfig {
                        user_agent: None,
                        extra_headers: Default::default(),
                        options: GeminiProviderOptions {
                            auth_header: None,
                            auth_scheme: None,
                            stream_mode: agena_provider::StreamTransportMode::Sse,
                            realtime_ws_url: None,
                        },
                    }),
                },
            ),
        ]),
    };
    for route in [
        "local/private-model",
        "failed/configured-fallback",
        "openai/manual-model",
    ] {
        config
            .models
            .insert(route.into(), ResolvedProviderModelConfig::default());
    }
    config.models.insert(
        "openai/disabled-model".into(),
        ResolvedProviderModelConfig {
            enabled: false,
            ..Default::default()
        },
    );
    let mut registry = build_provider_registry_from_configs(
        &BTreeMap::from([("gateway".into(), config)]),
        None,
        &ProcessEnvironment,
        None,
        &Default::default(),
    )
    .unwrap();
    // Catalog-only choices must not be reported as discovered account models.
    let mut catalog = ProviderModelCatalog::default();
    catalog
        .appendable_model_ids
        .insert("catalog-only-model".into());
    registry.register_arc(CatalogedModelsProvider::new(
        registry.get("gateway").unwrap(),
        catalog,
    ));
    let report = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        registry.discover_models("gateway"),
    )
    .await
    .unwrap()
    .unwrap();
    server.abort();
    assert!(!report.complete);
    assert_eq!(report.adapters.len(), 5);
    assert_eq!(report.models.len(), 9);
    assert_eq!(
        report
            .models
            .iter()
            .filter(|entry| entry.model.id.as_ref() == "shared-model")
            .count(),
        3
    );
    assert!(
        report
            .models
            .iter()
            .any(|entry| entry.model.id.as_ref() == "gpt-image-2"
                && entry.source == ModelDiscoverySource::ProviderDiscovery)
    );
    assert!(
        report
            .models
            .iter()
            .any(|entry| entry.model.id.as_ref() == "voice-model")
    );
    assert_eq!(
        report
            .models
            .iter()
            .filter(|entry| entry.source == ModelDiscoverySource::Configured)
            .count(),
        3
    );
    assert!(!report.models.iter().any(|entry| matches!(
        entry.model.id.as_ref(),
        "catalog-only-model" | "disabled-model"
    )));
    assert_eq!(
        report
            .adapters
            .iter()
            .find(|adapter| adapter
                .adapter_id
                .as_ref()
                .is_some_and(|id| id.as_ref() == "local"))
            .unwrap()
            .status,
        AdapterDiscoveryStatus::ConfiguredOnly
    );
    assert_eq!(
        report
            .adapters
            .iter()
            .find(|adapter| adapter
                .adapter_id
                .as_ref()
                .is_some_and(|id| id.as_ref() == "failed"))
            .unwrap()
            .status,
        AdapterDiscoveryStatus::Failed
    );
    assert!(
        !serde_json::to_string(&report)
            .unwrap()
            .contains("do-not-expose-provider-secret")
    );
    let paths = requests.lock().unwrap();
    assert_eq!(paths.len(), 7);
    assert!(paths.iter().any(|path| path.starts_with("/custom/models?")
        && path.contains("tenant=a")
        && path.contains("after=shared-model")));
    assert!(
        paths
            .iter()
            .any(|path| path.starts_with("/messages/models?")
                && path.contains("tenant=a")
                && path.contains("after=shared-model"))
    );
    assert!(
        paths
            .iter()
            .any(|path| path.contains("pageToken=second-page"))
    );
}
