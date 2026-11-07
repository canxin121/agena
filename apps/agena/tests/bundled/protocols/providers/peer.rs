//! Script only the remote vendor. Agena, its provider adapter, permission checks,
//! tool host, persisted output and the AI's second turn all remain production code.
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use agena_testkit::{SettingsLayer, TestAgenaBuilder, TestCase, ToolResult, UserSession};
use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{OriginalUri, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

use super::{API_KEY, Vendor, allow_permissions, fixture};

const MAX_EXCHANGES: usize = 128;

#[derive(Clone, Debug)]
pub(super) struct Request {
    pub method: Method,
    pub target: String,
    pub headers: HeaderMap,
    pub bytes: Vec<u8>,
}
impl Request {
    pub fn json(&self) -> Result<Value> {
        Ok(serde_json::from_slice(&self.bytes)?)
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

pub(super) struct ResponseSpec {
    pub status: StatusCode,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
    pub headers: Vec<(String, String)>,
    pub delay: Duration,
}
impl ResponseSpec {
    pub fn json(value: Value) -> Self {
        Self::bytes("application/json", value.to_string().into_bytes())
    }
    pub fn bytes(mime: &'static str, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            status: StatusCode::OK,
            mime,
            bytes: bytes.into(),
            headers: Vec::new(),
            delay: Duration::ZERO,
        }
    }
    pub fn status(mut self, status: u16) -> Self {
        self.status = StatusCode::from_u16(status).expect("fixture status");
        self
    }
    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
    pub fn delay(mut self, duration: Duration) -> Self {
        self.delay = duration;
        self
    }
}
struct Exchange {
    method: Method,
    target: String,
    response: ResponseSpec,
}
#[derive(Default)]
struct Script {
    expected: VecDeque<Exchange>,
    requests: Vec<Request>,
    failures: Vec<String>,
}
#[derive(Clone, Default)]
pub(super) struct Peer {
    script: Arc<Mutex<Script>>,
    base: Arc<OnceLock<String>>,
}
impl Peer {
    pub fn expect(&self, method: Method, target: impl Into<String>, response: ResponseSpec) {
        let mut script = self.script.lock().unwrap();
        assert!(script.expected.len() + script.requests.len() < MAX_EXCHANGES);
        script.expected.push_back(Exchange {
            method,
            target: target.into(),
            response,
        });
    }
    pub fn json(&self, method: Method, target: impl Into<String>, value: Value) {
        self.expect(method, target, ResponseSpec::json(value));
    }
    pub fn url(&self, suffix: &str) -> String {
        format!("{}{suffix}", self.base.get().expect("peer started"))
    }
    pub fn requests(&self) -> Vec<Request> {
        self.script.lock().unwrap().requests.clone()
    }
    pub fn last(&self) -> Result<Request> {
        self.requests()
            .last()
            .cloned()
            .context("vendor received no request")
    }
    pub fn assert_finished(&self) -> Result<()> {
        let script = self.script.lock().unwrap();
        ensure!(
            script.failures.is_empty(),
            "vendor protocol errors: {:?}",
            script.failures
        );
        ensure!(
            script.expected.is_empty(),
            "{} vendor requests were not made",
            script.expected.len()
        );
        Ok(())
    }
}

#[derive(Clone)]
struct Gateway {
    peer: Peer,
    upstream: String,
    client: reqwest::Client,
}

async fn serve(
    State(state): State<Gateway>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let parsed = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
    let main = method == Method::GET && uri.path().ends_with("/models")
        || method == Method::POST
            && parsed["tools"].as_array().is_some_and(|tools| {
                tools
                    .iter()
                    .any(|tool| tool["type"] == "function" && tool["name"] == "tools_call")
            })
            && !headers.contains_key("x-agena-provider-api");
    if main {
        let target = if method == Method::GET {
            "/v1/models"
        } else {
            "/v1/responses"
        };
        let result = async {
            let upstream = state
                .client
                .request(method, format!("{}{target}", state.upstream))
                .header("content-type", "application/json")
                .body(bytes)
                .send()
                .await?;
            let status = upstream.status();
            let content_type = upstream.headers().get("content-type").cloned();
            let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
            *response.status_mut() = status;
            if let Some(content_type) = content_type {
                response.headers_mut().insert("content-type", content_type);
            }
            Ok::<_, reqwest::Error>(response)
        }
        .await;
        return result
            .unwrap_or_else(|error| (StatusCode::BAD_GATEWAY, error.to_string()).into_response());
    }
    let target = uri
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(uri.path())
        .to_owned();
    let response = {
        let mut script = state.peer.script.lock().unwrap();
        let actual = Request {
            method: method.clone(),
            target: target.clone(),
            headers,
            bytes: bytes.to_vec(),
        };
        if script.requests.len() >= MAX_EXCHANGES {
            script
                .failures
                .push("request capture exceeded its bound".into());
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        }
        script.requests.push(actual);
        match script.expected.pop_front() {
            Some(expected) if expected.method == method && expected.target == target => {
                expected.response
            }
            expected => {
                let wanted = expected.map(|e| format!("{} {}", e.method, e.target));
                script
                    .failures
                    .push(format!("unexpected {method} {target}, expected {wanted:?}"));
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "unexpected vendor request",
                )
                    .into_response();
            }
        }
    };
    if !response.delay.is_zero() {
        tokio::time::sleep(response.delay).await;
    }
    let mut reply = Response::new(Body::from(response.bytes));
    *reply.status_mut() = response.status;
    reply
        .headers_mut()
        .insert("content-type", response.mime.parse().unwrap());
    for (name, value) in response.headers {
        let value = value.replace("$BASE", state.peer.base.get().unwrap());
        reply.headers_mut().insert(
            axum::http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    reply
}

pub(super) fn scripted(peer: &Peer) -> TestAgenaBuilder {
    scripted_routes(peer, Router::new())
}

pub(super) fn scripted_routes(peer: &Peer, routes: Router) -> TestAgenaBuilder {
    let peer = peer.clone();
    fixture()
        .builtin_plugin("agena.chatgpt", Vendor::ChatGpt.settings())
        .builtin_plugin("agena.claude", Vendor::Claude.settings())
        .builtin_plugin("agena.gemini", Vendor::Gemini.settings())
        .before_start(move |case| async move {
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            let upstream = config["providers"]["testkit"]["auth"]["base_url"]
                .as_str()
                .context("scripted model endpoint")?
                .trim_end_matches("/v1")
                .to_owned();
            let server = case
                .serve_http(
                    Router::new()
                        .fallback(serve)
                        .with_state(Gateway {
                            peer: peer.clone(),
                            upstream,
                            client: reqwest::Client::builder()
                                .no_proxy()
                                .timeout(Duration::from_secs(30))
                                .build()?,
                        })
                        .merge(routes),
                )
                .await?;
            peer.base
                .set(server.url("").trim_end_matches('/').to_owned())
                .map_err(|_| anyhow::anyhow!("peer started twice"))?;
            config["providers"]["testkit"]["auth"]["base_url"] = json!(server.url("/v1"));
            config["providers"]["testkit"]["auth"]["api_key"] =
                json!({"kind":"inline","value":API_KEY});
            case.write_config(SettingsLayer::Global, config).await?;
            let cancel = case.service_cancellation_token();
            // Run after the product has stopped, including when the scenario
            // returned early. Protocol mismatches must survive as diagnostics.
            case.spawn_service(async move {
                cancel.cancelled().await;
                peer.assert_finished()
            });
            Ok(())
        })
}

pub(super) async fn session(case: &TestCase) -> Result<UserSession> {
    let user = case.user().new_session().await?;
    allow_permissions(&user).await?;
    Ok(user)
}

pub(super) fn assert_request(request: &Request, vendor: Vendor, session: i64) -> Result<()> {
    let expected = if matches!(vendor, Vendor::ChatGpt) {
        format!("Bearer {API_KEY}")
    } else {
        API_KEY.into()
    };
    ensure!(
        request
            .headers
            .get(vendor.auth_header())
            .and_then(|v| v.to_str().ok())
            == Some(expected.as_str()),
        "wrong provider authentication: {request:?}"
    );
    ensure!(
        request
            .headers
            .get("x-session-id")
            .and_then(|v| v.to_str().ok())
            == Some(session.to_string().as_str()),
        "lost calling session identity: {request:?}"
    );
    Ok(())
}

pub(super) async fn attachment_bytes(
    user: &UserSession,
    result: &ToolResult,
    index: usize,
    mime: &str,
) -> Result<Vec<u8>> {
    ensure!(
        result.run.status.session_id == user.id(),
        "tool result belongs to another session"
    );
    let attachment = result.attachment(index)?;
    ensure!(
        attachment.mime == mime,
        "wrong attachment MIME: {attachment:?}"
    );
    Ok(user.download_attachment(attachment).await?.1)
}
