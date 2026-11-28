use super::*;
use agena_testkit::{Reply, RequestMatch};

const NAME: &str = "sampler";
const PROMPT: &str = "sample-only model prompt";
const RESULT: &str = "reviewed sampling response";
const PRIVATE_TURN: &str = "previous conversation private turn";

async fn sampling_peer(Json(request): Json<Value>) -> Response {
    let params = &request["params"];
    let mut result = match request["method"].as_str() {
        Some("server/discover") => json!({
            "supportedVersions":["2026-07-28"],"capabilities":{"tools":{}}
        }),
        Some("tools/list") => json!({"tools":[{
            "name":"sample","description":"Request a completion",
            "inputSchema":{"type":"object","properties":{}},
            "annotations":{"readOnlyHint":true,"destructiveHint":false}
        }]}),
        Some("tools/call") if params["requestState"] == "awaiting-sample" => {
            let answer = &params["inputResponses"]["completion"];
            assert_eq!(answer["role"], "assistant");
            assert_eq!(answer["content"]["text"], RESULT);
            assert_eq!(answer["model"], "fake-model");
            json!({"content":[{"type":"text","text":"sampling succeeded"}]})
        }
        Some("tools/call") => {
            assert!(
                params["_meta"]["io.modelcontextprotocol/clientCapabilities"]["sampling"]
                    .is_object()
            );
            return Json(json!({"jsonrpc":"2.0","id":request["id"],"result":{
                "resultType":"input_required",
                "requestState":"awaiting-sample",
                "inputRequests":{"completion":{
                    "method":"sampling/createMessage",
                    "params":{
                        "systemPrompt":"sample-only system prompt",
                        "messages":[{"role":"user","content":{"type":"text","text":PROMPT}}],
                        "maxTokens":48
                    }
                }}
            }}))
            .into_response();
        }
        _ => {
            return Json(json!({"jsonrpc":"2.0","id":request["id"],"error":{
                "code":-32601,"message":"Unexpected sampling fixture request"
            }}))
            .into_response();
        }
    };
    result["resultType"] = json!("complete");
    result["ttlMs"] = json!(0);
    result["cacheScope"] = json!("private");
    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})).into_response()
}

async fn approve(run: &agena_testkit::tool::ToolRun, preview: &str, label: &str) -> TestResult {
    let pending = run.wait_user_input().await?;
    let request = pending
        .request
        .request
        .as_user_input()
        .context("sampling approval")?;
    ensure!(request.title.contains(NAME));
    ensure!(request.questions[0].question.contains(preview));
    pending
        .answer(BTreeMap::from([("0".into(), vec![label.into()])]))
        .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_mcp_sampling_requests_consent_and_isolates_the_model_context() -> TestResult {
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .before_start(|case| async move {
            let peer = case
                .serve_http_on(
                    native_fixture_address()?,
                    Router::new().route("/mcp", post(sampling_peer)),
                )
                .await?;
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["agena.mcp"] = json!({
                "package":{"kind":"static"},
                "settings":{
                    "runtime":{"reconnect":{"enabled":false}},
                    "servers":{(NAME):{"transport":"http","endpoint":{"url":peer.url("/mcp")}}}
                }
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .run(|case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            case.model()
                .expect(RequestMatch::user_text(PRIVATE_TURN))
                .reply(Reply::text("previous answer"));
            user.send(PRIVATE_TURN)
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;

            let input = json!({"server":NAME,"name":"sample"});
            let declined = user.start_tool("mcp.tools.call", input.clone()).await?;
            approve(&declined, PROMPT, "Decline").await?;
            declined.wait_settled().await?.assert_failed()?;

            case.model()
                .expect(RequestMatch::user_text(PROMPT))
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(
                        body.contains("sample-only system prompt") && !body.contains(PRIVATE_TURN),
                        "{body}"
                    );
                    ensure!(request.body["max_output_tokens"] == 48, "{body}");
                    Ok(())
                })
                .reply(Reply::text(RESULT));
            let accepted = user.start_tool("mcp.tools.call", input).await?;
            approve(&accepted, PROMPT, "Allow").await?;
            approve(&accepted, RESULT, "Allow").await?;
            let output = accepted.wait_settled().await?;
            output.assert_succeeded()?;
            output.assert_text_contains("sampling succeeded")?;
            Ok(())
        })
        .await
}

const PNG_2X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DA8J8BAAf/Af8Bf4mnAAAAAElFTkSuQmCC";
const WAV_SAMPLE: &str = "UklGRiQAAABXQVZFZm10IBAAAAABAAEAQB8AAEAfAAABAAgAZGF0YQAAAAA=";
const SHARED_CONTEXT_TURN: &str = "shared session context marker alpha-42";
const MEDIA_PROMPT: &str = "inspect sampling media and invoke lookup_metric";
const FINAL_SUMMARY: &str = "Final metric summary: 42ms";

async fn advanced_sampling_peer(Json(request): Json<Value>) -> Response {
    let params = &request["params"];
    let mut result = match request["method"].as_str() {
        Some("server/discover") => json!({
            "supportedVersions":["2026-07-28"],"capabilities":{"tools":{}}
        }),
        Some("tools/list") => json!({"tools":[{
            "name":"sample_advanced","description":"Request media and tool sampling",
            "inputSchema":{"type":"object","properties":{}},
            "annotations":{"readOnlyHint":true,"destructiveHint":false}
        }]}),
        Some("tools/call") if params["requestState"] == "awaiting-tool" => {
            let answer = &params["inputResponses"]["step1"];
            assert_eq!(answer["role"], "assistant", "{answer}");
            assert_eq!(answer["stopReason"], "toolUse", "{answer}");
            let tool_use = &answer["content"];
            assert_eq!(tool_use["type"], "tool_use", "{answer}");
            assert_eq!(tool_use["name"], "lookup_metric", "{answer}");
            assert_eq!(tool_use["input"]["key"], "latency", "{answer}");
            let tool_id = tool_use["id"].as_str().expect("tool_use id").to_owned();
            return Json(json!({"jsonrpc":"2.0","id":request["id"],"result":{
                "resultType":"input_required",
                "requestState":"awaiting-final",
                "inputRequests":{"step2":{
                    "method":"sampling/createMessage",
                    "params":{
                        "systemPrompt":"advanced sampling system",
                        "tools":[{
                            "name":"lookup_metric",
                            "description":"Lookup a named latency metric",
                            "inputSchema":{
                                "type":"object",
                                "properties":{"key":{"type":"string"}},
                                "required":["key"]
                            }
                        }],
                        "toolChoice":{"mode":"auto"},
                        "messages":[
                            {"role":"user","content":{"type":"text","text":MEDIA_PROMPT}},
                            {"role":"assistant","content":{
                                "type":"tool_use",
                                "id":tool_id,
                                "name":"lookup_metric",
                                "input":{"key":"latency"}
                            }},
                            {"role":"user","content":{
                                "type":"tool_result",
                                "toolUseId":tool_id,
                                "content":[{"type":"text","text":"latency=42ms"}]
                            }}
                        ],
                        "maxTokens":64
                    }
                }}
            }}))
            .into_response();
        }
        Some("tools/call") if params["requestState"] == "awaiting-final" => {
            let answer = &params["inputResponses"]["step2"];
            assert_eq!(answer["role"], "assistant", "{answer}");
            assert_eq!(answer["content"]["text"], FINAL_SUMMARY, "{answer}");
            json!({"content":[{"type":"text","text":"advanced sampling succeeded"}]})
        }
        Some("tools/call") => {
            let caps = &params["_meta"]["io.modelcontextprotocol/clientCapabilities"]["sampling"];
            assert!(caps["tools"].is_object(), "{caps}");
            assert!(caps["context"].is_object(), "{caps}");
            return Json(json!({"jsonrpc":"2.0","id":request["id"],"result":{
                "resultType":"input_required",
                "requestState":"awaiting-tool",
                "inputRequests":{"step1":{
                    "method":"sampling/createMessage",
                    "params":{
                        "systemPrompt":"advanced sampling system",
                        "includeContext":"allServers",
                        "tools":[{
                            "name":"lookup_metric",
                            "description":"Lookup a named latency metric",
                            "inputSchema":{
                                "type":"object",
                                "properties":{"key":{"type":"string"}},
                                "required":["key"]
                            }
                        }],
                        "toolChoice":{"mode":"required"},
                        "messages":[{
                            "role":"user",
                            "content":[
                                {"type":"text","text":MEDIA_PROMPT},
                                {"type":"image","mimeType":"image/png","data":PNG_2X1},
                                {"type":"audio","mimeType":"audio/wav","data":WAV_SAMPLE}
                            ]
                        }],
                        "maxTokens":96
                    }
                }}
            }}))
            .into_response();
        }
        _ => {
            return Json(json!({"jsonrpc":"2.0","id":request["id"],"error":{
                "code":-32601,"message":"Unexpected advanced sampling request"
            }}))
            .into_response();
        }
    };
    result["resultType"] = json!("complete");
    result["ttlMs"] = json!(0);
    result["cacheScope"] = json!("private");
    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})).into_response()
}

async fn approve_all(
    run: &agena_testkit::tool::ToolRun,
    previews: &[&str],
    label: &str,
) -> TestResult {
    let pending = run.wait_user_input().await?;
    let request = pending
        .request
        .request
        .as_user_input()
        .context("sampling approval")?;
    ensure!(request.title.contains(NAME));
    for preview in previews {
        ensure!(
            request.questions[0].question.contains(preview),
            "expected preview to contain {preview:?}, got:\n{}",
            request.questions[0].question
        );
    }
    pending
        .answer(BTreeMap::from([("0".into(), vec![label.into()])]))
        .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_mcp_sampling_supports_media_tools_and_include_context() -> TestResult {
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .configure_global(|config| {
            config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["fake-model"]
                ["input"] = json!(["image", "audio", "text"]);
        })
        .before_start(|case| async move {
            let peer = case
                .serve_http_on(
                    native_fixture_address()?,
                    Router::new().route("/mcp", post(advanced_sampling_peer)),
                )
                .await?;
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["agena.mcp"] = json!({
                "package":{"kind":"static"},
                "settings":{
                    "runtime":{"reconnect":{"enabled":false}},
                    "servers":{(NAME):{"transport":"http","endpoint":{"url":peer.url("/mcp")}}}
                }
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .run(|case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            case.model()
                .expect(RequestMatch::user_text(SHARED_CONTEXT_TURN))
                .reply(Reply::text("context noted"));
            user.send(SHARED_CONTEXT_TURN)
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;

            let first = case
                .model()
                .expect(RequestMatch::user_text(MEDIA_PROMPT))
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(body.contains(SHARED_CONTEXT_TURN), "missing includeContext: {body}");
                    ensure!(body.contains("lookup_metric"), "missing tool declaration: {body}");
                    ensure!(body.contains("tools_call"), "missing gateway tool: {body}");
                    ensure!(body.contains("image/png"), "missing image attachment: {body}");
                    Ok(())
                })
                .reply(Reply::function_call(
                    "call-metric-1",
                    "tools_call",
                    json!({"tool": "mcp_sampling.lookup_metric", "input": {"key": "latency"}}),
                ));
            case.model()
                .expect(RequestMatch::any())
                .after(&first)
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(body.contains("latency=42ms"), "missing tool_result: {body}");
                    Ok(())
                })
                .reply(Reply::text(FINAL_SUMMARY));

            let run = user
                .start_tool("mcp.tools.call", json!({"server":NAME,"name":"sample_advanced"}))
                .await?;
            approve_all(
                &run,
                &[
                    SHARED_CONTEXT_TURN,
                    "Tools (required): lookup_metric",
                    "[Image: image/png",
                    "[Audio: audio/wav",
                ],
                "Allow",
            )
            .await?;
            approve_all(&run, &["Tool call lookup_metric", "latency"], "Allow").await?;
            approve_all(&run, &["latency=42ms"], "Allow").await?;
            approve_all(&run, &[FINAL_SUMMARY], "Allow").await?;
            let output = run.wait_settled().await?;
            output.assert_succeeded()?;
            output.assert_text_contains("advanced sampling succeeded")?;
            Ok(())
        })
        .await
}
