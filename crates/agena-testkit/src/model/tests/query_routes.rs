use anyhow::{Result, ensure};
use serde_json::json;

use super::{client, request};
use crate::model::{ModelOptions, ModelProtocol, ModelRoute, Reply, RequestMatch, ScriptedModel};

#[tokio::test]
async fn query_matchers_keep_order_and_duplicates_without_exposing_credentials() -> Result<()> {
    let owner =
        ScriptedModel::start_with_options(ModelOptions::for_protocol(ModelProtocol::Gemini))
            .await?;
    let url = format!(
        "{}/v1beta/models/fake-model:streamGenerateContent?alt=sse&key=secret-DO-NOT-LOG&alt=sse",
        owner.base_url().trim_end_matches("/v1")
    );
    let response = client()?
        .post(&url)
        .json(&json!({"contents":[] }))
        .send()
        .await?;
    ensure!(response.status().is_client_error());
    // No expectation means a strict fixture error; the request is still
    // captured for structural checks, including duplicates and raw URI.
    let observed = owner.handle().requests().pop().expect("recorded request");
    assert_eq!(observed.query_values("alt"), vec!["sse", "sse"]);
    assert_eq!(observed.query_values("key"), vec!["secret-DO-NOT-LOG"]);
    assert!(
        observed
            .raw_uri()
            .ends_with("?alt=sse&key=secret-DO-NOT-LOG&alt=sse")
    );
    assert!(!format!("{observed:?}").contains("secret-DO-NOT-LOG"));
    owner.shutdown().await
}

#[tokio::test]
async fn custom_gateway_route_keeps_protocol_model_streaming_and_query() -> Result<()> {
    let owner = ScriptedModel::start_with_options(ModelOptions {
        protocol: ModelProtocol::Gemini,
        models: vec!["fake-model".into()],
        routes: vec![
            ModelRoute::post_prefix("/gateway/models/").for_protocol(ModelProtocol::Gemini),
        ],
        auto_openai_models: false,
        ..ModelOptions::default()
    })
    .await?;
    let model = owner.handle();
    let step = model
        .expect(
            RequestMatch::user_text("custom-gemini")
                .protocol(ModelProtocol::Gemini)
                .streaming(true)
                .model("fake-model")
                .query_parameter("alt", "sse"),
        )
        .reply(Reply::text_chunks(["gateway", "-verified"]));
    let response = client()?
        .post(format!(
            "{}/gateway/models/fake-model:streamGenerateContent?alt=sse&key=hidden",
            owner.base_url().trim_end_matches("/v1")
        ))
        .json(&json!({"contents":[{"role":"user","parts":[{"text":"custom-gemini"}]}]}))
        .send()
        .await?
        .error_for_status()?;
    assert!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/event-stream"))
    );
    assert!(response.text().await?.contains("gateway"));
    let observed = step.observed().await?;
    assert_eq!(observed.protocol(), Some(ModelProtocol::Gemini));
    assert_eq!(observed.decoded_model().as_deref(), Some("fake-model"));
    assert!(observed.streaming());
    step.finished().await?;
    model.assert_finished()?;
    owner.shutdown().await
}

#[tokio::test]
async fn a_single_protocol_fixture_rejects_wrong_protocol_endpoints() -> Result<()> {
    let owner =
        ScriptedModel::start_with_options(ModelOptions::for_protocol(ModelProtocol::Anthropic))
            .await?;
    let response = client()?
        .post(format!(
            "{}/responses?key=uri-secret-DO-NOT-LOG",
            owner.base_url()
        ))
        .header("Authorization", "Bearer header-secret-DO-NOT-LOG")
        .json(&request("anthropic-only", true))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let diagnostic = owner.handle().failures().join("\n");
    assert!(!diagnostic.contains("uri-secret-DO-NOT-LOG"));
    assert!(!diagnostic.contains("header-secret-DO-NOT-LOG"));
    assert!(
        owner
            .handle()
            .failures()
            .iter()
            .any(|failure| failure.contains("unexpected model HTTP request"))
    );
    owner.shutdown().await
}

#[tokio::test]
async fn query_matcher_rejects_a_wrong_alt_without_echoing_a_query_secret() -> Result<()> {
    let owner =
        ScriptedModel::start_with_options(ModelOptions::for_protocol(ModelProtocol::Gemini))
            .await?;
    owner
        .handle()
        .expect(RequestMatch::user_text("gemini-query-mismatch").query_parameter("alt", "sse"))
        .reply(Reply::text("should-not-match"));
    let url = format!(
        "{}/v1beta/models/fake-model:streamGenerateContent?alt=broken&key=secret-DO-NOT-LOG",
        owner.base_url().trim_end_matches("/v1")
    );
    let response = client()?
        .post(&url)
        .json(&json!({"contents":[{"role":"user","parts":[{"text":"gemini-query-mismatch"}]}]}))
        .send()
        .await?;
    assert!(response.status().is_client_error());
    let failures = owner.handle().failures().join("\n");
    assert!(failures.contains("query parameter"), "{failures}");
    assert!(
        !failures.contains("secret-DO-NOT-LOG"),
        "query secret appeared in diagnostics"
    );
    owner.shutdown().await
}
