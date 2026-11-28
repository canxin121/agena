use std::{convert::Infallible, time::Duration};

use agena_testkit::{
    Reply, RequestMatch, TestAgenaBuilder, TestCase, TestHttpClient, TestHttpResponse,
};
use axum::response::{Sse, sse::Event};
use futures_util::{StreamExt, stream};
use tokio::sync::oneshot;

use super::*;

const RESOURCE: &str = "fixture://interactive";
const MODERN_PEER: &str = "interactive-modern-peer";
const FORM_CALLBACK: &str = "interactive-form";
const FORM_MESSAGE: &str = "Choose the multiplier.";
const SAMPLE_PROMPT: &str = "Work only with this App prompt.";
const SAMPLE_RESULT: &str = "The App's approved response.";
const PRIVATE_TURN: &str = "This conversation is private to the session.";

#[derive(Default)]
struct InteractivePeer {
    app: AppPeer,
    modern: bool,
    callback: Mutex<Option<oneshot::Sender<()>>>,
}

fn form() -> Value {
    json!({
        "method":"elicitation/create",
        "params":{
            "mode":"form","message":FORM_MESSAGE,
            "requestedSchema":{
                "type":"object","required":["factor"],"additionalProperties":false,
                "properties":{"factor":{"type":"string","enum":["Double","Triple"]}},
            },
        },
    })
}

async fn peer(
    State(peer): State<Arc<InteractivePeer>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let id = &request["id"];
    let params = &request["params"];
    let session = if peer.modern {
        Some(MODERN_PEER)
    } else {
        headers
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
    };
    let owns_app = session.is_some() && session == peer.app.app_session.lock().unwrap().as_deref();
    if owns_app && !peer.modern && id == FORM_CALLBACK {
        assert_eq!(
            request["result"],
            json!({"action":"accept","content":{"factor":"Double"}})
        );
        let answer = peer
            .callback
            .lock()
            .unwrap()
            .take()
            .expect("pending callback");
        let _ = answer.send(());
        return StatusCode::ACCEPTED.into_response();
    }

    let mut result = match request["method"].as_str() {
        Some("server/discover") if peer.modern => json!({
            "supportedVersions":["2026-07-28"],
            "capabilities":{"tools":{}, "resources":{}},
            "ttlMs":0, "cacheScope":"private",
            "_meta":{"io.modelcontextprotocol/serverInfo":{"name":"interactive-app-fixture","version":"1"}},
        }),
        Some("tools/call")
            if peer.modern
                && owns_app
                && params["name"] == "app_action"
                && params["arguments"]["sample"] == true =>
        {
            if params["requestState"].is_null() {
                json!({
                    "resultType":"input_required","requestState":"tool/sample",
                    "inputRequests":{"native-sample":{"method":"sampling/createMessage","params":{
                        "messages":[{"role":"user","content":{"type":"text","text":SAMPLE_PROMPT}}],
                        "maxTokens":48,
                    }}},
                })
            } else {
                assert_eq!(params["requestState"], "tool/sample");
                let sample = &params["inputResponses"]["native-sample"];
                assert_eq!(sample["model"], "fake-model");
                assert_eq!(sample["content"]["text"], SAMPLE_RESULT);
                json!({"content":[{"type":"text","text":"Sample received by the MCP server"}]})
            }
        }
        Some("tools/call") if owns_app && params["name"] == "app_action" => {
            if !peer.modern {
                let (send, receive) = oneshot::channel();
                assert!(peer.callback.lock().unwrap().replace(send).is_none());
                let mut callback = form();
                callback["jsonrpc"] = json!("2.0");
                callback["id"] = json!(FORM_CALLBACK);
                let result = json!({"jsonrpc":"2.0","id":id,"result":{
                    "content":[{"type":"text","text":"Legacy form accepted"}],"structuredContent":{"factor":"Double"},
                }});
                let start = stream::iter([Ok::<_, Infallible>(
                    Event::default().data(callback.to_string()),
                )]);
                let end = stream::once(async move {
                    receive
                        .await
                        .ok()
                        .map(|()| Ok(Event::default().data(result.to_string())))
                })
                .filter_map(std::future::ready);
                return Sse::new(start.chain(end)).into_response();
            }
            match params["requestState"].as_str() {
                None => json!({
                    "resultType":"input_required",
                    "requestState":"tool/form",
                    "inputRequests":{
                        "native-confirm":form(), "native-roots":{"method":"roots/list"},
                    },
                }),
                Some("tool/form") => {
                    assert_eq!(
                        params["inputResponses"]["native-confirm"],
                        json!({
                            "action":"accept","content":{"factor":"Double"},
                        })
                    );
                    assert!(params["inputResponses"]["native-roots"]["roots"].is_array());
                    json!({"resultType":"input_required","requestState":"tool/poll"})
                }
                Some("tool/poll") => {
                    assert!(params.get("inputResponses").is_none());
                    json!({"content":[{"type":"text","text":"Modern form accepted"}],"structuredContent":{"factor":"Double"}})
                }
                state => panic!("unexpected tool continuation {state:?}"),
            }
        }
        Some("resources/read") if owns_app && params["uri"] == RESOURCE => {
            assert!(peer.modern);
            if params["requestState"].is_null() {
                json!({
                    "resultType":"input_required",
                    "requestState":"resource/url",
                    "inputRequests":{"native-link":{
                        "method":"elicitation/create",
                        "params":{
                            "mode":"url", "message":"Authorize this resource",
                            "url":"https://example.invalid/resource",
                            "elicitationId":"read-resource",
                        },
                    }},
                })
            } else {
                assert_eq!(params["requestState"], "resource/url");
                assert_eq!(
                    params["inputResponses"]["native-link"],
                    json!({"action":"accept"})
                );
                json!({"contents":[{"uri":RESOURCE,"mimeType":"text/plain","text":"Approved resource"}]})
            }
        }
        _ if !peer.modern => return legacy_reply(&peer.app, headers, request),
        _ => {
            return match app_reply(&peer.app, &request, session, "2026-07-28") {
                Some(mut reply) => {
                    reply["result"]["resultType"] = json!("complete");
                    Json(reply).into_response()
                }
                None => StatusCode::ACCEPTED.into_response(),
            };
        }
    };
    if result.get("resultType").is_none() {
        result["resultType"] = json!("complete");
    }
    Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response()
}

fn interactive_fixture(modern: bool) -> TestAgenaBuilder {
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin("agena.mcp")
        .case_timeout(Duration::from_secs(90))
        .before_start(move |case| async move {
            let fixture = case
                .serve_http_on(
                    native_fixture_address()?,
                    Router::new()
                        .route(
                            "/mcp",
                            post(peer).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
                        )
                        .with_state(Arc::new(InteractivePeer {
                            modern,
                            ..Default::default()
                        })),
                )
                .await?;
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["agena.mcp"] = json!({
                "enabled":true,"package":{"kind":"static"},"settings":{
                    "runtime":{"reconnect":{"enabled":false}},
                    "servers":{(SERVER):{
                        "transport":"http","endpoint":{"url":fixture.url("/mcp")},
                        "timeouts":{"request_ms":3_000},
                    }},
                }
            });
            case.write_config(SettingsLayer::Global, config).await
        })
}

struct AppDocument {
    http: TestHttpClient,
    path: String,
    revision: String,
}

impl AppDocument {
    async fn mount(case: &TestCase, user: &agena_testkit::UserSession) -> anyhow::Result<Self> {
        let rendered = user
            .call_tool("mcp.tools.call", json!({"server":SERVER,"name":"render"}))
            .await?;
        rendered.assert_succeeded()?;
        let http = case.http().await;
        let path = format!(
            "/api/v1/sessions/{}/parts/{}/mcp-app",
            user.id(),
            rendered.part.part_id
        );
        let document: Value = http.get(&path).await?.require_success()?.json()?;
        let revision = document["revision"]
            .as_str()
            .context("App revision")?
            .to_owned();
        Ok(Self {
            http,
            path,
            revision,
        })
    }

    async fn open(&self, extension: bool) -> anyhow::Result<BrowserView<'_>> {
        let opened: Value = self
            .http
            .execute(
                self.request(Method::POST, "/views")?
                    .json(&json!({"tools":[],"task_extension":extension})),
            )
            .await?
            .require_success()?
            .json()?;
        let id = opened["view"].as_str().context("App view ID")?;
        Ok(BrowserView {
            document: self,
            path: format!("/views/{id}"),
        })
    }

    fn request(&self, method: Method, suffix: &str) -> anyhow::Result<reqwest::RequestBuilder> {
        Ok(self
            .http
            .request(method, &format!("{}{suffix}", self.path))?
            .header("if-match", format!("\"{}\"", self.revision)))
    }
}

struct BrowserView<'a> {
    document: &'a AppDocument,
    path: String,
}

impl BrowserView<'_> {
    async fn rpc(&self, request: &Value) -> anyhow::Result<TestHttpResponse> {
        self.document
            .http
            .execute(
                self.document
                    .request(Method::POST, &format!("{}/server", self.path))?
                    .json(request),
            )
            .await
    }

    async fn expect_prompt(&self, text: &str, previous: Option<&str>) -> anyhow::Result<Value> {
        let calls = format!("{}/calls", self.path);
        tokio::time::timeout(Duration::from_secs(8), async {
            let mut path = calls.clone();
            loop {
                let snapshot: Value = self
                    .document
                    .http
                    .execute(self.document.request(Method::GET, &path)?)
                    .await?
                    .require_success()?
                    .json()?;
                for pending in snapshot["calls"].as_array().context("App calls")? {
                    if pending["id"].as_str() == previous {
                        continue;
                    }
                    assert!(pending.get("method").is_none(), "prompt is for the host UI");
                    let question = pending["prompt"]["question"]["question"]
                        .as_str()
                        .context("host question")?;
                    ensure!(question.contains(text), "expected {text:?} in {question:?}");
                    return Ok::<_, anyhow::Error>(pending.clone());
                }
                path = format!("{calls}?after={}", snapshot["revision"]);
            }
        })
        .await
        .context("waiting for the App user prompt")?
    }

    async fn reply(&self, id: &str, reply: Value) -> anyhow::Result<TestHttpResponse> {
        self.document
            .http
            .execute(
                self.document
                    .request(Method::POST, &format!("{}/calls/{id}", self.path))?
                    .json(&reply),
            )
            .await
    }

    async fn answer(&self, prompt: &Value, choice: &str) -> TestResult {
        self.reply(
            prompt["id"].as_str().context("prompt ID")?,
            json!({"status":"input","answer":choice}),
        )
        .await?
        .require_success()?;
        Ok(())
    }

    async fn close(&self) -> TestResult {
        self.document
            .http
            .execute(self.document.request(Method::DELETE, &self.path)?)
            .await?
            .require_success()?;
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_app_calls_handle_modern_input_rounds_and_view_cancellation() -> TestResult {
    interactive_fixture(true)
        .run(|case| async move {
            super::super::super::cordis::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let app = AppDocument::mount(&case, &user).await?;
            let view = app.open(true).await?;
            let other = app.open(false).await?;
            let request = json!({"method":"tools/call","params":{"name":"app_action"}});
            let (result, ()) = tokio::try_join!(view.rpc(&request), async {
                let prompt = view.expect_prompt(FORM_MESSAGE, None).await?;
                assert!(!prompt.to_string().contains("tool/form"));
                let id = prompt["id"].as_str().context("form prompt ID")?;
                other.answer(&prompt, "Triple").await?;
                assert_eq!(
                    view.reply(id, json!({"status":"result","result":{"answer":"Triple"}}))
                        .await?
                        .status,
                    StatusCode::BAD_REQUEST
                );
                assert_eq!(
                    view.reply(id, json!({"status":"input","answer":"Other"}))
                        .await?
                        .status,
                    StatusCode::BAD_REQUEST
                );
                // Waiting longer than the server's active request timeout keeps the same call alive.
                tokio::time::sleep(Duration::from_millis(3100)).await;
                view.answer(&prompt, "Double").await
            })?;
            let result: Value = result.require_success()?.json()?;
            assert_eq!(result["structuredContent"]["factor"], "Double");

            let read = json!({"method":"resources/read","params":{"uri":RESOURCE}});
            let (result, ()) = tokio::try_join!(view.rpc(&read), async {
                let prompt = view
                    .expect_prompt("https://example.invalid/resource", None)
                    .await?;
                view.answer(&prompt, "Continue").await
            })?;
            let result: Value = result.require_success()?.json()?;
            assert_eq!(result["contents"][0]["text"], "Approved resource");

            case.model()
                .expect(RequestMatch::user_text(SAMPLE_PROMPT))
                .reply(Reply::text(SAMPLE_RESULT));
            let sample = json!({"method":"tools/call","params":{"name":"app_action","arguments":{"sample":true}}});
            let (result, ()) = tokio::try_join!(view.rpc(&sample), async {
                let run = view.expect_prompt(SAMPLE_PROMPT, None).await?;
                view.answer(&run, "Allow").await?;
                let release = view.expect_prompt(SAMPLE_RESULT, run["id"].as_str()).await?;
                ensure!(release["prompt"]["question"]["question"].as_str().unwrap().contains("MCP server"));
                view.answer(&release, "Allow").await
            })?;
            let result: Value = result.require_success()?.json()?;
            assert_eq!(result["content"][0]["text"], "Sample received by the MCP server");

            let (cancelled, ()) = tokio::try_join!(other.rpc(&request), async {
                other.expect_prompt(FORM_MESSAGE, None).await?;
                other.close().await
            })?;
            assert_eq!(cancelled.status, StatusCode::CONFLICT);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_legacy_callbacks_and_app_sampling_use_the_host_view_input() -> TestResult {
    interactive_fixture(false)
        .run(|case| async move {
            super::super::super::cordis::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            case.model()
                .expect(RequestMatch::user_text(PRIVATE_TURN))
                .reply(Reply::text("private answer"));
            user.send(PRIVATE_TURN)
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            let app = AppDocument::mount(&case, &user).await?;
            let view = app.open(false).await?;

            let call = json!({"method":"tools/call","params":{"name":"app_action"}});
            let (result, ()) = tokio::try_join!(view.rpc(&call), async {
                let form = view.expect_prompt(FORM_MESSAGE, None).await?;
                view.answer(&form, "Double").await
            })?;
            let result: Value = result.require_success()?.json()?;
            assert_eq!(result["structuredContent"]["factor"], "Double");

            let sample = json!({"method":"sampling/createMessage","params":{
                "messages":[{"role":"user","content":{"type":"text","text":SAMPLE_PROMPT}}],
                "systemPrompt":"App sample system prompt", "maxTokens":48,
            }});
            let mut invalid_choice = sample.clone();
            invalid_choice["params"]["toolChoice"] = json!({"mode":"required"});
            assert_eq!(
                view.rpc(&invalid_choice).await?.status,
                StatusCode::BAD_REQUEST
            );
            let mut with_tools = sample.clone();
            with_tools["params"]["tools"] =
                json!([{"name":"lookup","inputSchema":{"type":"object"}}]);
            let (result, ()) = tokio::try_join!(view.rpc(&with_tools), async {
                let prompt = view.expect_prompt("Tools (auto): lookup", None).await?;
                view.answer(&prompt, "Decline").await
            })?;
            assert_eq!(result.status, StatusCode::CONFLICT);
            let (result, ()) = tokio::try_join!(view.rpc(&sample), async {
                let prompt = view.expect_prompt(SAMPLE_PROMPT, None).await?;
                view.answer(&prompt, "Decline").await
            })?;
            assert_eq!(result.status, StatusCode::CONFLICT);

            case.model()
                .expect(RequestMatch::user_text(SAMPLE_PROMPT))
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(
                        body.contains("App sample system prompt") && !body.contains(PRIVATE_TURN),
                        "{body}"
                    );
                    assert_eq!(request.body["max_output_tokens"], 48);
                    Ok(())
                })
                .reply(Reply::text(SAMPLE_RESULT));
            let (result, ()) = tokio::try_join!(view.rpc(&sample), async {
                let run = view.expect_prompt(SAMPLE_PROMPT, None).await?;
                view.answer(&run, "Allow").await?;
                let release = view
                    .expect_prompt(SAMPLE_RESULT, run["id"].as_str())
                    .await?;
                assert!(
                    release["prompt"]["question"]["question"]
                        .as_str()
                        .unwrap()
                        .contains("MCP App")
                );
                view.answer(&release, "Allow").await
            })?;
            let result: Value = result.require_success()?.json()?;
            assert_eq!(result["model"], "fake-model");
            assert_eq!(result["role"], "assistant");
            assert_eq!(result["content"]["text"], SAMPLE_RESULT);

            let withheld = "Withheld sampling output.";
            case.model()
                .expect(RequestMatch::user_text(SAMPLE_PROMPT))
                .reply(Reply::text(withheld));
            let (result, ()) = tokio::try_join!(view.rpc(&sample), async {
                let run = view.expect_prompt(SAMPLE_PROMPT, None).await?;
                view.answer(&run, "Allow").await?;
                let release = view.expect_prompt(withheld, run["id"].as_str()).await?;
                view.answer(&release, "Decline").await
            })?;
            assert_eq!(result.status, StatusCode::CONFLICT);
            assert!(!result.text()?.contains(withheld));

            let (cancelled, ()) = tokio::try_join!(view.rpc(&sample), async {
                view.expect_prompt(SAMPLE_PROMPT, None).await?;
                view.close().await
            })?;
            assert_eq!(cancelled.status, StatusCode::CONFLICT);
            Ok(())
        })
        .await
}
