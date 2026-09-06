use super::*;

#[test]
fn request_model_updates_generated_identity_and_preserves_custom_user_agent() {
    let options = GeminiAdapterOptions {
        client_identity: crate::ProviderClientIdentity::new(
            agena_provider::ProviderClientVersions {
                gemini: "9.8.7".into(),
                ..Default::default()
            },
        ),
        ..Default::default()
    };
    let adapter = GeminiAdapter::new_managed_with_options(
        reqwest::Client::new(),
        ManagedCredential::static_value("test", "fixture-key"),
        "https://example.invalid/v1beta",
        "default-model",
        options,
    );
    let mut request: CompletionRequest = serde_json::from_value(serde_json::json!({
        "model":"requested-model", "messages":[],
    }))
    .unwrap();
    let headers = adapter.completion_request_headers(&request);
    assert!(headers["user-agent"].starts_with("GeminiCLI/9.8.7/requested-model ("));
    request
        .request_override
        .headers
        .insert("User-Agent".into(), "CustomClient/7".into());
    let headers = adapter.completion_request_headers(&request);
    let agents: Vec<_> = headers
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case("user-agent"))
        .collect();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].1, "CustomClient/7");
}
