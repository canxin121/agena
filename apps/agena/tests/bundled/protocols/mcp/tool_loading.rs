use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use agena_testkit::{ModelHandle, Reply, RequestMatch, TestCase, TestResult, UserSession};
use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};

use super::{Peer, RpcRequest, allow_permissions, fixture_without_memory, native_fixture_address};

const LOADER: &str = "mcp__direct__load_tools";
const PICK: &str = "mcp__direct__pick";
const BEFORE: &str = "initial_item";
const AFTER: &str = "updated_item";

#[derive(Default)]
struct LoadingPeer {
    generation: AtomicU8,
    rpc: Peer,
}

async fn rpc(
    State(peer): State<Arc<LoadingPeer>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let source = headers
        .get("x-fixture")
        .and_then(|header| header.to_str().ok());
    let generation = peer.generation.load(Ordering::Relaxed);
    let result = match request["method"].as_str() {
        Some("server/discover") => json!({
            "supportedVersions": ["2026-07-28"],
            "capabilities": {"tools": {}},
            "ttlMs": 0,
            "cacheScope": "private"
        }),
        Some("tools/list") => {
            let object = |name| json!({"name":name,"inputSchema":{"type":"object"}});
            let tools = match source {
                Some("direct") => {
                    let mut tools = vec![
                        object("skip"),
                        json!({
                            "name": "load_tools",
                            "description": "reserved-remote-loader",
                            "inputSchema": {"type":"object"}
                        }),
                    ];
                    if generation < 2 {
                        let property = if generation == 0 { BEFORE } else { AFTER };
                        tools.push(json!({
                            "name": "pick",
                            "description": "Pick a banana carefully\nprivate second line",
                            "inputSchema": {
                                "type": "object",
                                "properties": {(property): {"type": "string"}},
                                "required": [property],
                                "additionalProperties": false
                            }
                        }));
                    }
                    tools
                }
                Some("configured") => vec![object("managed_note")],
                Some("registered") => vec![object("registered_note")],
                Some("eager") => vec![object("fixture_eager")],
                _ => return StatusCode::BAD_REQUEST.into_response(),
            };
            json!({"tools": tools, "ttlMs": 0, "cacheScope": "private"})
        }
        Some("tools/call") => json!({
            "content": [{"type":"text", "text":request["params"]["name"]}],
            "structuredContent": {"arguments":request["params"]["arguments"]}
        }),
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    peer.rpc.requests.lock().unwrap().push(RpcRequest {
        headers,
        body: request.clone(),
    });
    let mut result = result;
    result["resultType"] = json!("complete");
    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})).into_response()
}

async fn install_fixture(case: &TestCase, endpoint: &str) -> TestResult {
    let package = case.paths().root.join("deepseek-loading");
    tokio::fs::create_dir(&package).await?;
    tokio::fs::create_dir(case.paths().root.join("empty-bin")).await?;
    tokio::fs::write(
        package.join("package.json"),
        br#"{"name":"@fixture/tool-loading","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    )
    .await?;
    let server = |source: &str| json!({"transport":"streamable-http","url":endpoint,"headers":{"x-fixture":source}});
    let mut direct = server("direct");
    direct["serverName"] = json!("direct");
    direct["toolLoading"] = json!("on-demand");
    direct["catalogDescriptionMaxLength"] = json!(16);
    let mut configured = server("configured");
    configured["toolLoading"] = json!("on-demand");
    let bundle = json!([
        {"id":"mcp-manager","config":{"servers":{"configured":configured,"eager":server("eager")}}},
        {"insert":[
            {"name":"@deepseek-ai/dsh-mcp-client","config":direct},
            {"name":"./register.mjs","config":{"endpoint":endpoint}}
        ]}
    ]);
    tokio::fs::write(package.join("bundle.yml"), serde_json::to_vec(&bundle)?).await?;
    tokio::fs::write(
        package.join("register.mjs"),
        r#"
export const inject = ['mcpManager'];
export const apply = (ctx, config) => ctx.mcpManager.register('registered', {
  transport: 'streamable-http',
  url: config.endpoint,
  headers: { 'x-fixture': 'registered' },
  toolLoading: 'on-demand',
});
"#,
    )
    .await?;
    let package = package.to_str().context("fixture path is not UTF-8")?;
    case.run_cli(["plugin", "install", package])
        .await?
        .require_success()?;
    Ok(())
}

async fn check_tool_help(
    model: &ModelHandle,
    user: &UserSession,
    request: &str,
    call_id: &str,
    expected: &'static str,
) -> TestResult {
    let help = model
        .expect(RequestMatch::user_text(request).declares_function("tools_help"))
        .reply(Reply::function_call(
            call_id,
            "tools_help",
            json!({"tool": PICK}),
        ));
    model
        .expect(RequestMatch::tool_output(call_id))
        .after(&help)
        .check_tool_output_contains(call_id, expected)
        .reply(Reply::text("checked tool help"));
    let result = user.send(request).await?.wait_settled().await?;
    result.assert_succeeded()?;
    result.assert_assistant_text("checked tool help")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_deepseek_mcp_tools_load_in_the_owning_session() -> TestResult {
    let peer = Arc::new(LoadingPeer::default());
    let setup_peer = Arc::clone(&peer);
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .builtin_plugin(
            "agena.mcp",
            json!({"runtime":{"reconnect":{"enabled":false}}}),
        )
        .expect_plugin("deepseek.fixture-tool-loading")
        .before_start(move |case| async move {
            let server = case
                .serve_http_on(
                    native_fixture_address()?,
                    Router::new()
                        .route("/mcp", post(rpc))
                        .with_state(setup_peer),
                )
                .await?;
            install_fixture(&case, &server.url("/mcp")).await
        })
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let model = case.model();
            let listing = model
                .expect(
                    RequestMatch::user_text("inspect-initial-loader")
                        .declares_function("tools_list"),
                )
                .check(|request| {
                    let system = request.body["instructions"]
                        .as_str()
                        .context("missing system prompt")?;
                    ensure!(
                        system.contains("On-demand tools from MCP server direct"),
                        "no on-demand catalog heading in the initial system prompt"
                    );
                    ensure!(
                        system.contains("Pick a banana...")
                            || system.contains("on_demand_catalog: true"),
                        "missing compact catalog and recovery entry point"
                    );
                    ensure!(
                        !system.contains("private second line")
                            && !system.contains(BEFORE)
                            && !system.contains("reserved-remote-loader"),
                        "the initial catalog exposed full or reserved tool metadata"
                    );
                    ensure!(
                        !request.declares_function(LOADER),
                        "raw MCP alias leaked into provider functions"
                    );
                    Ok(())
                })
                .reply(Reply::function_call(
                    "loader-listing",
                    "tools_list",
                    json!({"plugin":"mcp","limit":100}),
                ));
            model
                .expect(RequestMatch::tool_output("loader-listing"))
                .after(&listing)
                .check(|request| {
                    let output = request
                        .tool_output_text("loader-listing")
                        .context("missing tools list")?;
                    for alias in [
                        LOADER,
                        "mcp__configured__load_tools",
                        "mcp__registered__load_tools",
                    ] {
                        ensure!(output.contains(alias), "{output}");
                    }
                    ensure!(!output.contains(PICK), "{output}");
                    Ok(())
                })
                .reply(Reply::text("loader listed"));
            user.send("inspect-initial-loader")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("loader listed")?;

            let search = user.call_tool("mcp.tools.search", json!({})).await?;
            search.assert_succeeded()?;
            let data = search.data()?;
            let tools = data["results"]
                .as_array()
                .context("missing MCP search results")?;
            ensure!(tools.iter().any(|row| row["name"] == "fixture_eager"));
            ensure!(
                tools
                    .iter()
                    .all(|row| row["name"] != "pick" && row["name"] != "skip")
            );
            let server = tools
                .iter()
                .find(|row| {
                    row["aliases"]
                        .as_array()
                        .is_some_and(|aliases| aliases.contains(&json!(LOADER)))
                })
                .and_then(|row| row["server"].as_str())
                .context("missing direct server")?
                .to_owned();
            let listed = user
                .call_tool(
                    "mcp.tools.search",
                    json!({"on_demand_catalog":true,"query":"pick"}),
                )
                .await?;
            listed.assert_succeeded()?;
            ensure!(
                listed.data()?["results"]
                    == json!([{
                        "server": server,
                        "name": "pick",
                        "description": "Pick a banana...",
                        "loader": LOADER,
                        "loaded": false
                    }])
            );
            let first = user
                .call_tool(
                    "mcp.tools.search",
                    json!({"on_demand_catalog":true,"limit":2}),
                )
                .await?;
            let first_page = first.data()?;
            ensure!(first_page["next_offset"] == 2);
            let second = user
                .call_tool(
                    "mcp.tools.search",
                    json!({"on_demand_catalog":true,"limit":2,"catalog_offset":2}),
                )
                .await?;
            let second_page = second.data()?;
            ensure!(second_page["next_offset"].is_null());
            for row in second_page["results"]
                .as_array()
                .context("missing second page")?
            {
                ensure!(
                    !first_page["results"]
                        .as_array()
                        .context("missing first page")?
                        .contains(row)
                );
            }
            model
                .expect(RequestMatch::user_text("inspect-cached-catalog"))
                .check(|request| {
                    let system = request.body["instructions"]
                        .as_str()
                        .context("missing system prompt")?;
                    ensure!(
                        system.contains("Pick a banana..."),
                        "the discovered summary is missing from the prompt"
                    );
                    ensure!(!system.contains("private second line") && !system.contains(BEFORE));
                    Ok(())
                })
                .reply(Reply::text("catalog rendered"));
            user.send("inspect-cached-catalog")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("catalog rendered")?;
            let generic = |raw| json!({"server":server,"name":raw,"arguments":{(BEFORE):"value"}});

            user.call_tool(LOADER, json!({"tools":["pick","load_tools"]}))
                .await?
                .assert_failed()?;
            user.call_tool("mcp.tools.call", generic("pick"))
                .await?
                .assert_failed()?;
            let loaded = user
                .call_tool(LOADER, json!({"tools":["pick","pick"]}))
                .await?;
            loaded.assert_succeeded()?;
            ensure!(loaded.data()?["loaded"] == json!([PICK]));
            let again = user.call_tool(LOADER, json!({"tools":["pick"]})).await?;
            ensure!(again.data()?["already_loaded"] == json!([PICK]));
            check_tool_help(
                &model,
                &user,
                "initial-pick-schema",
                "initial-schema",
                BEFORE,
            )
            .await?;
            user.call_tool(PICK, json!({})).await?.assert_failed()?;
            user.call_tool("mcp.tools.call", generic("skip"))
                .await?
                .assert_failed()?;
            let invoked = user.call_tool(PICK, json!({(BEFORE): "value"})).await?;
            invoked.assert_succeeded()?;
            ensure!(invoked.data()?["structured_content"]["arguments"][BEFORE] == "value");

            let other = case.user().new_session().await?;
            allow_permissions(&other).await?;
            check_tool_help(
                &model,
                &other,
                "isolated-pick-schema",
                "other-schema",
                "unavailable",
            )
            .await?;
            other
                .call_tool("mcp.tools.call", generic("pick"))
                .await?
                .assert_failed()?;
            let managed = other
                .call_tool(
                    "mcp__configured__load_tools",
                    json!({"tools":["managed_note"]}),
                )
                .await?;
            managed.assert_succeeded()?;
            other
                .call_tool("mcp__configured__managed_note", json!({}))
                .await?
                .assert_succeeded()?;
            let cordis = other
                .call_tool(
                    "mcp__registered__load_tools",
                    json!({"tools":["registered_note"]}),
                )
                .await?;
            cordis.assert_succeeded()?;
            other
                .call_tool("mcp__registered__registered_note", json!({}))
                .await?
                .assert_succeeded()?;

            peer.generation.store(1, Ordering::Relaxed);
            check_tool_help(
                &model,
                &user,
                "updated-pick-schema",
                "updated-schema",
                AFTER,
            )
            .await?;
            user.call_tool(PICK, json!({(BEFORE): "old"}))
                .await?
                .assert_failed()?;
            user.call_tool(PICK, json!({(AFTER): "new"}))
                .await?
                .assert_succeeded()?;
            peer.generation.store(2, Ordering::Relaxed);
            check_tool_help(
                &model,
                &user,
                "removed-pick-schema",
                "removed-schema",
                "unavailable",
            )
            .await?;
            peer.generation.store(1, Ordering::Relaxed);
            user.call_tool("mcp.tools.call", generic("pick"))
                .await?
                .assert_failed()?;
            user.call_tool(LOADER, json!({"tools":["pick"]}))
                .await?
                .assert_succeeded()?;
            user.client().clear_session(user.id(), true).await?;
            check_tool_help(
                &model,
                &user,
                "cleared-pick-schema",
                "cleared-schema",
                "unavailable",
            )
            .await?;

            let denied = case.user().new_session().await?;
            allow_permissions(&denied).await?;
            let mut policy =
                serde_json::to_value(denied.snapshot().await?.execution.selected_permission)?;
            policy["network"] = json!({"default":"deny"});
            denied
                .client()
                .set_session_permission(denied.id(), serde_json::from_value(policy)?)
                .await?;
            let before_denial = peer.rpc.requests.lock().unwrap().len();
            denied
                .call_tool(LOADER, json!({"tools":["pick"]}))
                .await?
                .assert_failed()?;
            let requests = peer.rpc.requests.lock().unwrap();
            ensure!(
                requests.len() == before_denial,
                "a denied session contacted the MCP peer"
            );
            let direct_calls = requests
                .iter()
                .filter(|request| {
                    request.headers["x-fixture"] == "direct"
                        && request.body["method"] == "tools/call"
                })
                .map(|request| &request.body["params"])
                .collect::<Vec<_>>();
            ensure!(
                direct_calls.len() == 2
                    && direct_calls.iter().all(|params| params["name"] == "pick")
            );
            ensure!(
                direct_calls[0]["arguments"][BEFORE] == "value"
                    && direct_calls[1]["arguments"][AFTER] == "new"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires native process sandboxing for imported stdio servers"]
async fn imported_deepseek_stdio_tools_load_through_the_native_bridge() -> TestResult {
    fixture_without_memory()
        .env("AGENA_SANDBOX", "enabled")
        .builtin_plugin(
            "agena.mcp",
            json!({"runtime":{"reconnect":{"enabled":false}}}),
        )
        .expect_plugin("deepseek.fixture-stdio-loading")
        .before_start(|case| async move {
            let package = case.paths().root.join("stdio-loading");
            tokio::fs::create_dir(&package).await?;
            tokio::fs::write(
                package.join("package.json"),
                br#"{"name":"@fixture/stdio-loading","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
            )
            .await?;
            tokio::fs::write(
                package.join("bundle.yml"),
                serde_json::to_vec(&json!([
                    {"insert":[{"name":"@deepseek-ai/dsh-mcp-client","config":{
                        "serverName":"stdio",
                        "transport":"stdio",
                        "command":"python3",
                        "args":["${PLUGIN_ROOT}/peer.py","${PLUGIN_DATA}/transcript.jsonl"],
                        "toolLoading":"on-demand"
                    }}]}
                ]))?,
            )
            .await?;
            tokio::fs::write(package.join("peer.py"), include_str!("stdio_peer.py")).await?;
            let package = package.to_str().context("fixture path is not UTF-8")?;
            case.run_cli(["plugin", "install", package])
                .await?
                .require_success()?;
            Ok(())
        })
        .run(|case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let result = user
                .call_tool("mcp__stdio__load_tools", json!({"tools":["echo"]}))
                .await?;
            result.assert_succeeded()?;
            let called = user
                .call_tool("mcp__stdio__echo", json!({"text":"selection"}))
                .await?;
            called.assert_succeeded()?;
            called.assert_text_contains("stdio-peer:selection")?;
            ensure!(called.data()?["structured_content"]["origin"] == "local-stdio");
            Ok(())
        })
        .await
}
