// Pure URL-encoding contract. Runtime provider behavior is exercised by
// apps/agena/tests/bundled/protocols/providers through TestAgena.

#[test]
fn gemini_model_alias_cannot_escape_the_provider_path() {
    let endpoint = super::official_service::gemini_model_endpoint(
        "https://gateway.example/proxy/v1beta",
        "models/team/alias?token=ignored#fragment",
    )
    .unwrap();
    assert_eq!(
        endpoint,
        "https://gateway.example/proxy/v1beta/models/team%2Falias%3Ftoken=ignored%23fragment:generateContent"
    );
}
