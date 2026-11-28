use std::{collections::BTreeMap, sync::Arc, time::Duration};

use agena_testkit::{TestCase, TestResult};
use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};

use super::{
    Peer, RpcRequest, TASKS, allow_permissions, fixture_without_memory, native_fixture_address,
};

const TOKEN_ENV: &str = "AGENA_FIXTURE_MANAGED_MCP_TOKEN";
const MANAGED_TOKEN: &str = "local-fixture-only-mcp-token";

async fn rpc(
    State(peer): State<Arc<Peer>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let source = headers
        .get("x-fixture")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bearer = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if matches!(source.as_str(), "configured" | "registered")
        && bearer != format!("Bearer {MANAGED_TOKEN}")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    peer.requests.lock().unwrap().push(RpcRequest {
        headers,
        body: request.clone(),
    });
    let params = &request["params"];
    let task = json!({
        "taskId": "fixture-stall",
        "createdAt": "2026-07-28T12:00:00Z",
        "lastUpdatedAt": "2026-07-28T12:00:00Z",
        "status": "working",
        "ttlMs": 20_000,
        "pollIntervalMs": 100
    });
    let mut result = match request["method"].as_str() {
        Some("server/discover") => json!({
            "supportedVersions": ["2026-07-28"],
            "capabilities": {"tools": {}, "extensions": {(TASKS): {}}},
            "ttlMs": 0,
            "cacheScope": "private"
        }),
        Some("tools/list") => {
            let names: &[&str] = match source.as_str() {
                "codex" => &["fixture-allowed", "fixture-blocked", "fixture-other"],
                "catalog" => {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                    &["fixture-late"]
                }
                "claude" => &["fixture-stall"],
                "configured" => &["fixture-configured"],
                "registered" => &["fixture-registered"],
                _ => &[],
            };
            let tools: Vec<_> = names
                .iter()
                .map(|name| {
                    json!({
                        "name": name,
                        "inputSchema": {"type":"object"},
                        "annotations": {"readOnlyHint":true,"destructiveHint":false}
                    })
                })
                .collect();
            json!({"tools":tools,"ttlMs":0,"cacheScope":"private"})
        }
        Some("tools/call") if params["name"] == "fixture-stall" => {
            let mut result = task;
            result["resultType"] = json!("task");
            return Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
                .into_response();
        }
        Some("tools/call") => json!({
            "content":[{"type":"text","text":params["name"]}],
            "structuredContent":{"credential": bearer}
        }),
        Some("tasks/get") if params["taskId"] == "fixture-stall" => task,
        Some("tasks/cancel") if params["taskId"] == "fixture-stall" => json!({}),
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    result["resultType"] = json!("complete");
    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})).into_response()
}

async fn install_fixtures(case: &TestCase, endpoint: &str) -> TestResult {
    let root = case.paths().root.join("agent-packages");
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    let http = |source: &str| {
        json!({
            "type": "http",
            "url": endpoint,
            "headers": {"x-fixture":source}
        })
    };
    let codex = root.join("codex");
    let mut active = http("codex");
    active["enabled_tools"] = json!(["fixture-allowed", "fixture-blocked"]);
    active["disabled_tools"] = json!(["fixture-blocked"]);
    let mut delayed = http("catalog");
    delayed["startup_timeout_sec"] = json!(2.5);
    tokio::fs::create_dir_all(codex.join(".codex-plugin")).await?;
    tokio::fs::write(
        codex.join(".codex-plugin/plugin.json"),
        br#"{"name":"mcp-settings-fixture"}"#,
    )
    .await?;
    tokio::fs::write(
        codex.join(".mcp.json"),
        serde_json::to_vec(&json!({
            "mcpServers":{"codex":active,"late":delayed}
        }))?,
    )
    .await?;

    let claude = root.join("claude");
    let mut claude_tool = http("claude");
    claude_tool["timeout"] = json!(1_000);
    tokio::fs::create_dir_all(claude.join(".claude-plugin")).await?;
    tokio::fs::write(
        claude.join(".claude-plugin/plugin.json"),
        br#"{"name":"mcp-settings-fixture"}"#,
    )
    .await?;
    tokio::fs::write(
        claude.join(".mcp.json"),
        serde_json::to_vec(&json!({
            "mcpServers":{"claude":claude_tool}
        }))?,
    )
    .await?;

    let deepseek = root.join("deepseek");
    tokio::fs::create_dir_all(&deepseek).await?;
    tokio::fs::write(
        deepseek.join("package.json"),
        br#"{"name":"@fixture/mcp-settings","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    )
    .await?;
    let mut configured = http("configured");
    configured.as_object_mut().unwrap().remove("type");
    configured["transport"] = json!("streamable-http");
    configured["headers"]["Authorization"] = json!(format!("Bearer ${{credential:{TOKEN_ENV}}}"));
    let bundle = json!([
        {"id":"mcp-manager","config":{"servers":{"configured":configured}}},
        {"insert":[{"name":"./register.mjs","config":{"endpoint":endpoint}}]}
    ]);
    tokio::fs::write(deepseek.join("bundle.yml"), serde_json::to_vec(&bundle)?).await?;
    tokio::fs::write(
        deepseek.join("register.mjs"),
        r#"
export const inject = ['mcpManager'];
export const apply = (ctx, config) => ctx.mcpManager.register('registered', {
  transport: 'streamable-http',
  url: config.endpoint,
  headers: { 'x-fixture': 'registered', Authorization: 'Bearer ${credential:AGENA_FIXTURE_MANAGED_MCP_TOKEN}' },
});
"#,
    ).await?;

    for package in [&codex, &claude, &deepseek] {
        let path = package
            .to_str()
            .context("fixture plugin path is not UTF-8")?;
        let mut args = vec!["plugin", "install", path];
        if package == &deepseek {
            args.extend(["--env", TOKEN_ENV]);
        }
        case.run_cli(args).await?.require_success()?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_mcp_settings_and_cordis_registrations_apply_to_live_calls() -> TestResult {
    let peer = Arc::new(Peer::default());
    let setup_peer = Arc::clone(&peer);
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env(TOKEN_ENV, MANAGED_TOKEN)
        .env_relative("PATH", "empty-bin")
        .builtin_plugin(
            "agena.mcp",
            json!({"runtime":{"reconnect":{"enabled":false}}}),
        )
        .expect_plugin("codex.mcp-settings-fixture")
        .expect_plugin("claude.mcp-settings-fixture")
        .expect_plugin("deepseek.fixture-mcp-settings")
        .before_start(move |case| async move {
            let server = case
                .serve_http_on(
                    native_fixture_address()?,
                    Router::new()
                        .route("/mcp", post(rpc))
                        .with_state(setup_peer),
                )
                .await?;
            install_fixtures(&case, &server.url("/mcp")).await
        })
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let search = user
                .call_tool("mcp.tools.search", json!({"query":"","limit":20}))
                .await?;
            search.assert_succeeded()?;
            let data = search.data()?;
            let entries = data["results"]
                .as_array()
                .context("MCP catalog is missing")?;
            let names: BTreeMap<_, _> = entries
                .iter()
                .filter_map(|entry| Some((entry["name"].as_str()?, entry["server"].as_str()?)))
                .collect();
            ensure!(
                names.keys().copied().eq([
                    "fixture-allowed",
                    "fixture-configured",
                    "fixture-registered",
                    "fixture-stall"
                ]),
                "unexpected imported tools: {names:?}"
            );
            let late_request = peer.requests.lock().unwrap().iter().any(|request| {
                request.headers["x-fixture"] == "catalog" && request.body["method"] == "tools/list"
            });
            ensure!(
                late_request,
                "the startup deadline was not exercised during tool discovery"
            );
            for name in [
                "fixture-allowed",
                "fixture-configured",
                "fixture-registered",
            ] {
                let result = user
                    .call_tool("mcp.tools.call", json!({"server":names[name],"name":name}))
                    .await?;
                result.assert_succeeded()?;
                result.assert_text_contains(name)?;
                if name != "fixture-allowed" {
                    ensure!(
                        result.data()?["structured_content"]["credential"] == "Bearer [redacted]"
                    );
                }
            }
            let blocked = user
                .call_tool(
                    "mcp.tools.call",
                    json!({"server":names["fixture-allowed"],"name":"fixture-blocked"}),
                )
                .await?;
            blocked.assert_failed()?;
            let timeout = tokio::time::timeout(
                Duration::from_secs(15),
                user.call_tool(
                    "mcp.tools.call",
                    json!({"server":names["fixture-stall"],"name":"fixture-stall"}),
                ),
            )
            .await
            .context("the plugin's one-second limit did not end the task")??;
            timeout.assert_failed()?;
            ensure!(
                timeout
                    .output
                    .to_string()
                    .contains("exceeded its active time budget"),
                "declared timeout was not applied: {}",
                timeout.output
            );
            let requests = peer.requests.lock().unwrap();
            ensure!(
                requests
                    .iter()
                    .any(|request| request.body["method"] == "tasks/cancel")
            );
            ensure!(
                requests
                    .iter()
                    .all(|request| request.body["params"]["name"] != "fixture-blocked")
            );
            Ok(())
        })
        .await
}
