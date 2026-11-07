mod connections;

use std::sync::{Arc, Mutex};

use agena_testkit::{SettingsLayer, TestResult};
use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};

use super::super::{allow_permissions, fixture};

const TOKEN: &str = "local-mcp-fixture-token";
const SERVER: &str = "loopback-peer";

#[derive(Clone, Debug)]
struct RpcRequest {
    headers: HeaderMap,
    body: Value,
}

#[derive(Default)]
struct Peer {
    requests: Mutex<Vec<RpcRequest>>,
}

async fn rpc(
    State(peer): State<Arc<Peer>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    {
        let mut records = peer.requests.lock().unwrap();
        if records.len() >= 128 {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                "MCP fixture request budget exceeded",
            )
                .into_response();
        }
        records.push(RpcRequest {
            headers: headers.clone(),
            body: request.clone(),
        });
    }
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Bearer local-mcp-fixture-token")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let method = request["method"].as_str().unwrap_or("");
    if request.get("id").is_none() {
        return if matches!(
            method,
            "notifications/initialized" | "notifications/cancelled"
        ) {
            StatusCode::ACCEPTED.into_response()
        } else {
            StatusCode::BAD_REQUEST.into_response()
        };
    }
    let params = &request["params"];
    let result = match method {
        // Exercise the production client's negotiated legacy fallback. This is
        // a real JSON-RPC rejection, not a test-only client shortcut.
        "server/discover" => None,
        "initialize" => Some(json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}, "resources": {}, "prompts": {}},
            "serverInfo": {"name": "agena-system-fixture", "version": "1.0"}
        })),
        "tools/list" => Some(json!({
            "tools": [
                {
                    "name": "echo",
                    "description": "fixture echo returns supplied text",
                    "inputSchema": {
                        "type": "object", "properties": {"text": {"type": "string"}},
                        "required": ["text"], "additionalProperties": false
                    },
                    "annotations": {"readOnlyHint": true, "destructiveHint": false}
                },
                {
                    "name": "fail",
                    "description": "fixture operation reports an MCP tool failure",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "hidden",
                    "description": "fixture operation excluded by product configuration",
                    "inputSchema": {"type": "object", "properties": {}}
                }
            ]
        })),
        "tools/call" if params["name"] == "echo" => {
            let text = params["arguments"]["text"].as_str().unwrap_or("missing");
            Some(json!({
                "content": [{"type": "text", "text": format!("mcp-peer:{text}")}],
                "structuredContent": {"echo": text, "origin": "local-http"},
                "isError": false
            }))
        }
        "tools/call" if params["name"] == "fail" => Some(json!({
            "content": [{"type": "text", "text": "deliberate MCP peer failure"}],
            "isError": true
        })),
        "resources/list" => Some(json!({
            "resources": [{
                "uri": "fixture://notes", "name": "Local notes",
                "description": "mcp-resource-list-marker", "mimeType": "text/plain"
            }]
        })),
        "resources/templates/list" => Some(json!({
            "resourceTemplates": [{
                "uriTemplate": "fixture://notes/{id}", "name": "Local note template"
            }]
        })),
        "resources/read" if params["uri"] == "fixture://notes" => Some(json!({
            "contents": [{
                "uri": "fixture://notes", "mimeType": "text/plain",
                "text": "mcp-resource-read-marker"
            }]
        })),
        "prompts/list" => Some(json!({
            "prompts": [{
                "name": "review", "description": "mcp-prompt-list-marker",
                "arguments": [{"name": "topic", "description": "Review topic", "required": true}]
            }]
        })),
        "prompts/get" if params["name"] == "review" => Some(json!({
            "description": "Local review prompt",
            "messages": [{
                "role": "user",
                "content": {
                    "type": "text",
                    "text": format!(
                        "mcp-prompt-review:{}",
                        params["arguments"]["topic"].as_str().unwrap_or("missing")
                    )
                }
            }]
        })),
        "ping" => Some(json!({})),
        _ => None,
    };
    Json(match result {
        Some(result) => json!({"jsonrpc": "2.0", "id": request["id"], "result": result}),
        None => json!({
            "jsonrpc": "2.0", "id": request["id"],
            "error": {"code": -32601, "message": "Method not supported by local fixture"}
        }),
    })
    .into_response()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub(super) async fn http_peer_handshake_discovery_and_calls() -> TestResult {
    let peer = Arc::new(Peer::default());
    let setup_peer = peer.clone();
    fixture()
        .expect_plugin("agena.mcp")
        .before_start(move |case| async move {
            let server = case
                .serve_http(
                    Router::new()
                        .route(
                            "/mcp",
                            post(rpc).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
                        )
                        .with_state(setup_peer),
                )
                .await?;
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["agena.mcp"]["package"] = json!({"kind": "static"});
            config["plugins"]["list"]["agena.mcp"]["settings"] = json!({
                "runtime": {"reconnect": {"enabled": false}},
                "servers": {
                    (SERVER): {
                        "transport": "http",
                        "endpoint": {
                            "url": server.url("/mcp"),
                            "headers": {"x-mcp-fixture": "configured-header"}
                        },
                        "auth": {"kind": "bearer", "token": TOKEN},
                        "timeouts": {"connect_ms": 10000, "request_ms": 10000},
                        "tools": {"exclude": ["hidden"]}
                    }
                }
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let search = user
                .call_tool("mcp.tools.search", json!({"query": "fixture", "limit": 10}))
                .await?;
            search.assert_succeeded()?;
            let found = search.data()?["results"]
                .as_array()
                .context("MCP tool search did not return an index")?;
            ensure!(found.len() == 2, "MCP configured tool filter was not applied: {found:?}");
            ensure!(found.iter().any(|tool| tool["name"] == "echo" && tool["server"] == SERVER));
            ensure!(found.iter().all(|tool| tool["name"] != "hidden"));

            let echo = user
                .call_tool(
                    "mcp.tools.call",
                    json!({
                        "server": SERVER, "name": "echo",
                        "arguments": {"text": "roundtrip-value"}
                    }),
                )
                .await?;
            echo.assert_succeeded()?;
            echo.assert_text_contains("mcp-peer:roundtrip-value")?;
            ensure!(echo.data()?["structured_content"]["echo"] == "roundtrip-value");
            ensure!(echo.data()?["structured_content"]["origin"] == "local-http");
            ensure!(!echo.output.to_string().contains(TOKEN));

            user.call_tool(
                "mcp.tools.call",
                json!({"server": SERVER, "name": "fail", "arguments": {}}),
            )
            .await?
            .assert_failed()?;
            user.call_tool(
                "mcp.tools.call",
                json!({"server": SERVER, "name": "hidden", "arguments": {}}),
            )
            .await?
            .assert_failed()?;

            for (tool, input, marker) in [
                (
                    "mcp.resources.list",
                    json!({"server": SERVER}),
                    "mcp-resource-list-marker",
                ),
                (
                    "mcp.resources.templates.list",
                    json!({"server": SERVER}),
                    "fixture://notes/{id}",
                ),
                (
                    "mcp.resources.read",
                    json!({"server": SERVER, "uri": "fixture://notes"}),
                    "mcp-resource-read-marker",
                ),
                (
                    "mcp.prompts.list",
                    json!({"server": SERVER}),
                    "mcp-prompt-list-marker",
                ),
                (
                    "mcp.prompts.get",
                    json!({"server": SERVER, "name": "review", "arguments": {"topic": "wire-test"}}),
                    "mcp-prompt-review:wire-test",
                ),
            ] {
                let result = user.call_tool(tool, input).await?;
                result.assert_succeeded()?;
                result.assert_text_contains(marker)?;
            }

            let requests = peer.requests.lock().unwrap().clone();
            let methods: Vec<_> = requests
                .iter()
                .filter_map(|request| request.body["method"].as_str())
                .collect();
            let initialized = methods.iter().position(|method| *method == "initialize")
                .context("the product never initialized the MCP peer")?;
            let notified = methods.iter().position(|method| *method == "notifications/initialized")
                .context("the initialized notification never reached the MCP peer")?;
            let listed = methods.iter().position(|method| *method == "tools/list")
                .context("the product never discovered the remote tool catalog")?;
            ensure!(initialized < notified && notified < listed, "invalid MCP lifecycle: {methods:?}");
            let calls: Vec<_> = requests
                .iter()
                .filter(|request| request.body["method"] == "tools/call")
                .collect();
            ensure!(calls.len() == 2, "a hidden tool was invoked or a call was retried: {calls:?}");
            ensure!(calls[0].body["params"]["name"] == "echo");
            ensure!(calls[0].body["params"]["arguments"]["text"] == "roundtrip-value");
            ensure!(calls[1].body["params"]["name"] == "fail");
            for request in &requests {
                ensure!(
                    request.headers.get("authorization").and_then(|value| value.to_str().ok())
                        == Some("Bearer local-mcp-fixture-token")
                );
                ensure!(
                    request.headers.get("x-mcp-fixture").and_then(|value| value.to_str().ok())
                        == Some("configured-header")
                );
                if request.body["method"] == "tools/call" {
                    ensure!(
                        request.headers.get("mcp-protocol-version")
                            .and_then(|value| value.to_str().ok()) == Some("2025-11-25"),
                        "negotiated MCP protocol version was not propagated"
                    );
                }
            }
            Ok(())
        })
        .await
}
