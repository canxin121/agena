use super::*;

#[test]
fn codex_discovery_query_and_request_headers_share_the_adapter_identity() {
    let adapter = |version: &str| {
        OpenAiResponsesAdapter::new_managed_with_options(
            "identity-fixture",
            reqwest::Client::new(),
            ManagedCredential::static_value("test", "fixture-key"),
            "https://example.invalid/backend-api/codex",
            "gpt-test",
            OpenAiResponsesAdapterOptions {
                client_identity: crate::ProviderClientIdentity::new(
                    agena_provider::ProviderClientVersions {
                        codex: version.into(),
                        ..Default::default()
                    },
                ),
                backend: OpenAiResponsesBackend::ChatgptCodex,
                ..Default::default()
            },
        )
    };
    let old = adapter("0.111.1");
    let new = adapter("0.222.2");
    for (adapter, version) in [(&old, "0.111.1"), (&new, "0.222.2"), (&old, "0.111.1")] {
        let endpoint = url::Url::parse(&adapter.list_models_endpoint().unwrap()).unwrap();
        assert_eq!(
            endpoint
                .query_pairs()
                .find(|(key, _)| key == "client_version")
                .unwrap()
                .1,
            version
        );
        let headers = adapter.auth_headers(RequestHeaderContext::none(), "fixture-key");
        assert!(headers["user-agent"].starts_with(&format!("codex_cli_rs/{version} ")));
    }
}
