use super::*;
use reqwest::Method;

const URI: &str = "ui://fixture/app";
const APP_META: &str = "mcp-app-host-only-metadata";

async fn app_peer(Json(request): Json<Value>) -> Response {
    if request.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    let params = &request["params"];
    let descriptor = |name, visibility| {
        json!({
            "name": name,
            "inputSchema": {"type": "object"},
            "_meta": {"ui": {"resourceUri": URI, "visibility": visibility}},
        })
    };
    let result = match request["method"].as_str().unwrap_or_default() {
        "initialize"
            if params["capabilities"]["extensions"]["io.modelcontextprotocol/ui"]["mimeTypes"]
                == json!(["text/html;profile=mcp-app"]) =>
        {
            Some(json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {"tools": {}, "resources": {}},
                "serverInfo": {"name": "local-app-peer", "version": "1"}
            }))
        }
        "tools/list" => Some(json!({"tools": [
            descriptor("render", ["model", "app"].as_slice()),
            descriptor("app_action", ["app"].as_slice()),
            descriptor("model_action", ["model"].as_slice()),
        ]})),
        "tools/call" if params["name"] == "render" => Some(json!({
            "content": [{"type": "text", "text": "Model-visible MCP result"}],
            "structuredContent": {"item": 42},
            "_meta": {"private": APP_META},
        })),
        "tools/call" if params["name"] == "app_action" => Some(json!({
            "content": [{"type": "text", "text": "App action acknowledged"}],
            "structuredContent": {"received": params["arguments"]["value"]},
        })),
        "resources/read" if params["uri"] == URI => Some(json!({"contents": [{
            "uri": URI, "mimeType": "text/html;profile=mcp-app", "text": "<main>Fixture MCP App</main>",
        }]})),
        "resources/read" if params["uri"] == "fixture://data" => Some(json!({"contents": [{
            "uri": "fixture://data", "mimeType": "text/plain", "text": "Fixture data",
        }]})),
        _ => None,
    };
    Json(match result {
        Some(result) => json!({"jsonrpc": "2.0", "id": request["id"], "result": result}),
        None => json!({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32601, "message": "Fixture method unavailable"}}),
    }).into_response()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_app_bridge_uses_live_mcp_policy_and_the_original_tool_result() -> TestResult {
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin("agena.mcp")
        .before_start(|case| async move {
            let server = case.serve_http_on(native_fixture_address()?, Router::new()
                .route("/mcp", post(app_peer).get(|| async { StatusCode::METHOD_NOT_ALLOWED }))).await?;
            let mut config: Value = serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["agena.mcp"] = json!({
                "enabled": true, "package": {"kind":"static"},
                "settings": {
                    "runtime": {"reconnect": {"enabled": false}},
                    "servers": {(SERVER): {"transport": "http", "endpoint": {"url": server.url("/mcp")}}},
                },
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .run(|case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let search = user.call_tool("mcp.tools.search", json!({"query":""})).await?;
            search.assert_succeeded()?;
            assert!(!search.data()?.to_string().contains("app_action"));
            user.call_tool("mcp.tools.call", json!({"server":SERVER,"name":"app_action"})).await?.assert_failed()?;
            let rendered = user.call_tool("mcp.tools.call", json!({
                "server": SERVER, "name": "render", "arguments": {"selection": 1},
            })).await?;
            rendered.assert_succeeded()?;
            assert!(!rendered.output.to_string().contains(APP_META));
            let part = rendered.part.part_id;
            let session = user.id();
            let path = format!("/api/v1/sessions/{session}/parts/{part}/mcp-app");
            let http = case.http().await;
            let document: Value = http.get(&path).await?.require_success()?.json()?;
            assert_eq!(document["html"], "<main>Fixture MCP App</main>");
            assert_eq!(document["arguments"]["selection"], 1);
            assert_eq!(document["tool_result"]["structuredContent"]["item"], 42);
            assert_eq!(document["tool_result"]["_meta"]["private"], APP_META);
            let revision = document["revision"].as_str().context("MCP App revision")?;
            let invoke = |body: Value| http.request(Method::POST, &path).map(|request| {
                request.header("if-match", format!("\"{revision}\"")).json(&body)
            });
            let data: Value = http.execute(invoke(json!({"method":"resources/read","params":{"uri":"fixture://data"}}))?).await?.require_success()?.json()?;
            assert_eq!(data["contents"][0]["text"], "Fixture data");
            let action = json!({"method":"tools/call","params":{"name":"app_action","arguments":{"value":7}}});
            let result: Value = http.execute(invoke(action.clone())?).await?.require_success()?.json()?;
            assert_eq!(result["structuredContent"]["received"], 7);
            assert_eq!(http.execute(invoke(json!({"method":"tools/call","params":{"name":"model_action"}}))?).await?.status, StatusCode::CONFLICT);
            let different = case.user().new_session().await?;
            assert_eq!(http.get(&format!("/api/v1/sessions/{}/parts/{part}/mcp-app", different.id())).await?.status, StatusCode::NOT_FOUND);
            let mut policy = serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
            policy["tools"]["names"]["mcp.tools.call"] = json!("deny");
            policy["tools"]["names"]["agena.mcp.tools.call"] = json!("deny");
            user.client().set_session_permission(session, serde_json::from_value(policy)?).await?;
            assert_eq!(http.execute(invoke(action)?).await?.status, StatusCode::CONFLICT);
            Ok(())
        }).await
}
