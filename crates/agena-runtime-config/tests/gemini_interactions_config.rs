use agena_runtime_config::{ProviderAdapterDefinition, ProviderAuthConfig, RawConfig};
use serde_json::json;

#[test]
fn resolves_standard_api_key_with_gemini_interactions_adapter() {
    let raw: RawConfig = serde_json::from_value(json!({
        "providers": {
            "gemini": {
                "auth": {
                    "mode": "api",
                    "subtype": "custom",
                    "base_url": "https://generativelanguage.googleapis.com/v1beta",
                    "api_key": { "kind": "env", "value": "GEMINI_API_KEY" }
                },
                "adapters": {
                    "gemini_interactions": {
                        "enabled": true,
                        "auth_header": "x-goog-api-key"
                    }
                }
            }
        }
    }))
    .unwrap();

    let resolved = raw.resolve().unwrap();
    let provider = resolved.providers.get("gemini").unwrap();
    assert!(matches!(provider.auth, ProviderAuthConfig::Api(_)));
    let adapter = provider.adapters.get("gemini_interactions").unwrap();
    assert!(adapter.enabled);
    match &adapter.definition {
        ProviderAdapterDefinition::GeminiInteractions(http) => {
            assert_eq!(http.options.auth_header.as_deref(), Some("x-goog-api-key"));
            assert!(http.options.auth_scheme.is_none());
        }
        other => panic!("unexpected adapter definition: {other:?}"),
    }
}

#[test]
fn rejects_gemini_interactions_without_base_url() {
    let raw: RawConfig = serde_json::from_value(json!({
        "providers": {
            "gemini": {
                "auth": {
                    "mode": "api",
                    "subtype": "custom",
                    "api_key": { "kind": "env", "value": "GEMINI_API_KEY" }
                },
                "adapters": {
                    "gemini_interactions": {
                        "enabled": true
                    }
                }
            }
        }
    }))
    .unwrap();

    let err = raw.resolve().unwrap_err();
    assert!(
        err.to_string().contains("base_url") && err.to_string().contains("gemini_interactions"),
        "unexpected error: {err}"
    );
}

#[test]
fn rejects_adapter_level_base_url_and_live_ws_fields_on_gemini_interactions() {
    let with_base_url: RawConfig = serde_json::from_value(json!({
        "providers": {
            "gemini": {
                "auth": {
                    "mode": "api",
                    "subtype": "custom",
                    "base_url": "https://generativelanguage.googleapis.com/v1beta",
                    "api_key": { "kind": "env", "value": "GEMINI_API_KEY" }
                },
                "adapters": {
                    "gemini_interactions": {
                        "enabled": true,
                        "base_url": "https://example.test/v1beta"
                    }
                }
            }
        }
    }))
    .unwrap();
    assert!(
        with_base_url
            .resolve()
            .unwrap_err()
            .to_string()
            .contains("does not support `base_url`")
    );

    let with_ws: RawConfig = serde_json::from_value(json!({
        "providers": {
            "gemini": {
                "auth": {
                    "mode": "api",
                    "subtype": "custom",
                    "base_url": "https://generativelanguage.googleapis.com/v1beta",
                    "api_key": { "kind": "env", "value": "GEMINI_API_KEY" }
                },
                "adapters": {
                    "gemini_interactions": {
                        "enabled": true,
                        "realtime_ws_url": "wss://example.test/ws"
                    }
                }
            }
        }
    }))
    .unwrap();
    assert!(
        with_ws
            .resolve()
            .unwrap_err()
            .to_string()
            .contains("WebSocket")
    );
}

#[test]
fn rejects_non_api_auth_modes_for_gemini_interactions() {
    let raw: RawConfig = serde_json::from_value(json!({
        "providers": {
            "gemini": {
                "auth": {
                    "mode": "credential",
                    "issuer": "openai_chatgpt"
                },
                "adapters": {
                    "gemini_interactions": {
                        "enabled": true
                    }
                }
            }
        }
    }))
    .unwrap();

    assert!(raw.resolve().is_err());
}
