use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use agena_testkit::{SettingsLayer, TestResult};
use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{OriginalUri, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::super::{allow_permissions, fixture};

mod boundaries;
mod catalog;
mod files;
mod peer;
mod realtime;
mod resources;
mod usage;

const API_KEY: &str = "protocol-fixture-credential";
const STABLE_PREFIX: &str = "protocol-fixture-stable-instruction";
const MAX_CAPTURED_REQUESTS: usize = 16;

#[derive(Clone, Copy, Debug)]
enum Vendor {
    ChatGpt,
    Claude,
    Gemini,
}

impl Vendor {
    fn name(self) -> &'static str {
        match self {
            Self::ChatGpt => "chatgpt",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
        }
    }

    fn tool(self) -> &'static str {
        match self {
            Self::ChatGpt => "chatgpt.web_search",
            Self::Claude => "claude.web_search",
            Self::Gemini => "gemini.google_search",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::ChatGpt => "/v1/responses",
            Self::Claude => "/v1/messages",
            Self::Gemini => "/v1beta/interactions",
        }
    }

    fn tool_type(self) -> &'static str {
        match self {
            Self::ChatGpt => "web_search",
            Self::Claude => "web_search_20260318",
            Self::Gemini => "google_search",
        }
    }

    fn auth_header(self) -> &'static str {
        match self {
            Self::ChatGpt => "authorization",
            Self::Claude => "x-api-key",
            Self::Gemini => "x-goog-api-key",
        }
    }

    fn settings(self) -> Value {
        match self {
            Self::ChatGpt => json!({
                "cache_mode": "disabled", "stable_instructions": STABLE_PREFIX
            }),
            Self::Claude => json!({
                "cache_ttl": "disabled", "stable_system": STABLE_PREFIX, "max_tokens": 128
            }),
            Self::Gemini => json!({"stable_system_instruction": STABLE_PREFIX}),
        }
    }

    fn response(self, model: &str) -> Value {
        let marker = format!("{}-local-provider-result", self.name());
        match self {
            Self::ChatGpt => json!({
                "id": "resp-local-hosted",
                "object": "response",
                "status": "completed",
                "model": model,
                "output": [{
                    "type": "message", "id": "msg-local-hosted", "role": "assistant",
                    "status": "completed",
                    "content": [{"type": "output_text", "text": marker, "annotations": []}]
                }],
                "usage": {"input_tokens": 7, "output_tokens": 3, "total_tokens": 10}
            }),
            Self::Claude => json!({
                "id": "msg-local-hosted", "type": "message", "role": "assistant",
                "model": model, "stop_reason": "end_turn", "stop_sequence": null,
                "content": [{"type": "text", "text": marker}],
                "usage": {"input_tokens": 7, "output_tokens": 3}
            }),
            Self::Gemini => json!({
                "id": "interaction-local-hosted", "status": "completed", "model": model,
                "outputs": [{"type": "text", "text": marker}],
                "usage": {"total_input_tokens": 7, "total_output_tokens": 3, "total_tokens": 10}
            }),
        }
    }
}

#[derive(Clone, Debug)]
struct CapturedRequest {
    path: String,
    headers: HeaderMap,
    body: Value,
}

#[derive(Clone)]
struct Gateway {
    vendor: Vendor,
    upstream: String,
    client: reqwest::Client,
    hosted: Arc<Mutex<Vec<CapturedRequest>>>,
}

/// One local gateway serves both the real conversational adapter and the
/// provider tools. Only the main Responses endpoint is forwarded to the owned
/// scripted model; all vendor operations terminate here without external I/O.
async fn gateway(
    State(state): State<Gateway>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let parsed: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let is_hosted = method == Method::POST
        && uri.path() == state.vendor.path()
        && parsed["tools"].as_array().is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool["type"] == state.vendor.tool_type())
        });
    if is_hosted {
        {
            let mut hosted = state.hosted.lock().unwrap();
            if hosted.len() >= MAX_CAPTURED_REQUESTS {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    "fixture capture limit exceeded",
                )
                    .into_response();
            }
            hosted.push(CapturedRequest {
                path: uri.path().to_owned(),
                headers,
                body: parsed.clone(),
            });
        }
        if parsed.to_string().contains("hosted-http-error-marker") {
            // The plugin must report a failed operation rather than treating a
            // non-success HTTP response as provider output or retrying it.
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error": {"message": "fixture upstream unavailable"}})),
            )
                .into_response();
        }
        return (
            [("x-request-id", "request-local-hosted")],
            Json(
                state
                    .vendor
                    .response(parsed["model"].as_str().unwrap_or("")),
            ),
        )
            .into_response();
    }

    if !matches!(
        (method.as_str(), uri.path()),
        ("POST", "/v1/responses") | ("GET", "/v1/models")
    ) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unexpected protocol fixture route"})),
        )
            .into_response();
    }
    let suffix = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or(uri.path());
    let response = async {
        let mut request = state
            .client
            .request(method, format!("{}{suffix}", state.upstream))
            .body(body);
        for name in ["authorization", "content-type", "accept"] {
            if let Some(value) = headers.get(name) {
                request = request.header(name, value);
            }
        }
        let upstream = request.send().await?;
        let status = upstream.status();
        let response_headers = upstream.headers().clone();
        let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
        *response.status_mut() = status;
        for name in ["content-type", "cache-control"] {
            if let Some(value) = response_headers.get(name) {
                response.headers_mut().insert(name, value.clone());
            }
        }
        Ok::<_, reqwest::Error>(response)
    }
    .await;
    response.unwrap_or_else(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("owned scripted model gateway failed: {error}"),
        )
            .into_response()
    })
}

async fn provider_case(vendor: Vendor) -> TestResult {
    let hosted = Arc::new(Mutex::new(Vec::new()));
    let setup_hosted = hosted.clone();
    fixture()
        .expect_plugin(format!("agena.{}", vendor.name()))
        .before_start(move |case| async move {
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            let base = config["providers"]["testkit"]["auth"]["base_url"]
                .as_str()
                .context("local model preset is missing its endpoint")?;
            let upstream = base
                .strip_suffix("/v1")
                .context("the local Responses endpoint must end in /v1")?
                .to_owned();
            let server = case
                .serve_http(
                    Router::new().fallback(gateway).with_state(Gateway {
                        vendor,
                        upstream,
                        client: reqwest::Client::builder()
                            .no_proxy()
                            .redirect(reqwest::redirect::Policy::none())
                            .timeout(Duration::from_secs(30))
                            .build()?,
                        hosted: setup_hosted,
                    }),
                )
                .await?;
            config["providers"]["testkit"]["auth"]["base_url"] = json!(server.url("/v1"));
            config["providers"]["testkit"]["auth"]["api_key"] =
                json!({"kind": "inline", "value": API_KEY});
            config["plugins"]["list"][format!("agena.{}", vendor.name())]["package"] =
                json!({"kind": "static"});
            config["plugins"]["list"][format!("agena.{}", vendor.name())]["settings"] =
                vendor.settings();
            case.write_config(SettingsLayer::Global, config).await
        })
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let selected_model = case.model().model_id().to_owned();
            let override_model = format!("protocol-override-{}", vendor.name());
            for (label, requested, expected) in [
                ("default", None, selected_model.clone()),
                (
                    "override",
                    Some(override_model.clone()),
                    override_model.clone(),
                ),
            ] {
                let mut input = json!({"prompt": format!("hosted-{}-{label}", vendor.name())});
                if let Some(model) = requested {
                    input["model"] = json!(model);
                }
                let result = user.call_tool(vendor.tool(), input).await?;
                result.assert_succeeded()?;
                ensure!(
                    result
                        .text()
                        .contains(&format!("{}-local-provider-result", vendor.name())),
                    "provider response was not presented to the model: {}",
                    result.output
                );
                let data = result.data()?;
                ensure!(data["text"] == format!("{}-local-provider-result", vendor.name()));
                ensure!(
                    data["provider"] == vendor.name(),
                    "wrong provider payload: {data}"
                );
                ensure!(
                    data["provider_id"] == "testkit",
                    "session provider was not used: {data}"
                );
                ensure!(data["model"] == expected, "wrong request model: {data}");
                ensure!(
                    data["outcome"] == "completed",
                    "provider completion was not retained: {data}"
                );
                ensure!(
                    data["local_actions_executed"] == false,
                    "provider output executed local actions"
                );
                ensure!(data["request_id"] == "request-local-hosted");
                let receipt_path = Path::new(
                    data["response_receipt"]["path"]
                        .as_str()
                        .context("provider response receipt was not persisted")?,
                );
                ensure!(
                    receipt_path.starts_with(&case.paths().root),
                    "provider receipt escaped the case directory: {}",
                    receipt_path.display()
                );
                let receipt = tokio::fs::read(receipt_path).await?;
                ensure!(
                    Sha256::digest(&receipt)
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                        == data["response_receipt"]["sha256"].as_str().unwrap_or(""),
                    "receipt bytes do not match the reported SHA-256"
                );
                let saved: Value = serde_json::from_slice(&receipt)?;
                ensure!(
                    saved == vendor.response(&expected),
                    "stored receipt changed the response"
                );
                ensure!(
                    !result.output.to_string().contains(API_KEY),
                    "credential entered model output"
                );
            }

            let failure = user
                .call_tool(vendor.tool(), json!({"prompt": "hosted-http-error-marker"}))
                .await?;
            failure.assert_failed()?;
            ensure!(!failure.output.to_string().contains(API_KEY));

            let records = hosted.lock().unwrap().clone();
            ensure!(
                records.len() == 3,
                "expected exactly 3 hosted HTTP calls, got {records:#?}"
            );
            for (index, request) in records.iter().enumerate() {
                ensure!(
                    request.path == vendor.path(),
                    "wrong vendor endpoint: {request:#?}"
                );
                ensure!(
                    request.body["stream"] == false,
                    "hosted operation unexpectedly streamed"
                );
                ensure!(
                    request.body["tools"][0]["type"] == vendor.tool_type(),
                    "wrong hosted tool declaration: {}",
                    request.body
                );
                let expected_model = if index == 1 {
                    &override_model
                } else {
                    &selected_model
                };
                ensure!(request.body["model"] == expected_model.as_str());
                let expected_prompt = match index {
                    0 => format!("hosted-{}-default", vendor.name()),
                    1 => format!("hosted-{}-override", vendor.name()),
                    _ => "hosted-http-error-marker".to_owned(),
                };
                let sent = request.body.to_string();
                ensure!(
                    sent.contains(&expected_prompt),
                    "hosted tool omitted its prompt"
                );
                ensure!(
                    !sent.contains("agena-testkit-tool-request-"),
                    "provider tool forwarded the surrounding Agena conversation"
                );
                let expected_auth = if matches!(vendor, Vendor::ChatGpt) {
                    format!("Bearer {API_KEY}")
                } else {
                    API_KEY.to_owned()
                };
                ensure!(
                    request
                        .headers
                        .get(vendor.auth_header())
                        .and_then(|value| value.to_str().ok())
                        == Some(expected_auth.as_str()),
                    "the selected provider credential did not reach its vendor protocol"
                );
                ensure!(
                    request
                        .headers
                        .get("x-session-id")
                        .and_then(|value| value.to_str().ok())
                        == Some(user.id().to_string().as_str()),
                    "provider request lost the calling session identity"
                );
                match vendor {
                    Vendor::ChatGpt => {
                        ensure!(request.body["input"][0]["role"] == "developer");
                        ensure!(request.body["input"][0]["content"][0]["text"] == STABLE_PREFIX);
                        ensure!(request.body.get("prompt_cache_key").is_none());
                    }
                    Vendor::Claude => {
                        ensure!(request.body["system"] == STABLE_PREFIX);
                        ensure!(request.body["max_tokens"] == 128);
                        ensure!(request.body["tools"][0]["name"] == "web_search");
                        ensure!(
                            request
                                .headers
                                .get("anthropic-version")
                                .and_then(|value| value.to_str().ok())
                                == Some("2023-06-01")
                        );
                    }
                    Vendor::Gemini => {
                        ensure!(request.body["system_instruction"] == STABLE_PREFIX);
                    }
                }
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub(super) async fn chatgpt_uses_the_session_provider() -> TestResult {
    provider_case(Vendor::ChatGpt).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub(super) async fn claude_uses_the_session_provider() -> TestResult {
    provider_case(Vendor::Claude).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub(super) async fn gemini_uses_the_session_provider() -> TestResult {
    provider_case(Vendor::Gemini).await
}
