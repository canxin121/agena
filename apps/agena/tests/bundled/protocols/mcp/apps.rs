use super::*;
use reqwest::Method;
use std::sync::atomic::{AtomicU64, Ordering};

const URI: &str = "ui://fixture/app";
const APP_META: &str = "mcp-app-host-only-metadata";

mod interactive;
mod legacy;
mod stdio;
mod streamable;
mod view_tools;

#[derive(Default)]
struct AppPeer {
    next_session: AtomicU64,
    app_session: Mutex<Option<String>>,
    frame_origin: Option<String>,
    app_reads: AtomicU64,
    catalog_version: AtomicU64,
    resource_catalog_version: AtomicU64,
    sends_catalog_notifications: bool,
    offers_task: bool,
}

fn task_info(id: &str, status: &str) -> Value {
    json!({
        "taskId": id, "status": status, "createdAt": "2026-11-27T00:00:00Z",
        "lastUpdatedAt": "2026-11-27T00:00:00Z", "ttl": 300_000, "pollInterval": 20,
    })
}

async fn app_peer(
    State(peer): State<Arc<AppPeer>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    legacy_reply(&peer, headers, request)
}

fn legacy_reply(peer: &AppPeer, headers: HeaderMap, request: Value) -> Response {
    let session = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok());
    let Some(result) = app_reply(peer, &request, session, "2025-11-25") else {
        return StatusCode::ACCEPTED.into_response();
    };
    let mut response = Json(result).into_response();
    if request["method"] == "initialize" {
        let id = peer.next_session.fetch_add(1, Ordering::Relaxed) + 1;
        response.headers_mut().insert(
            "mcp-session-id",
            format!("app-fixture-{id}").parse().unwrap(),
        );
    }
    response
}

fn app_reply(
    peer: &AppPeer,
    request: &Value,
    session: Option<&str>,
    version: &str,
) -> Option<Value> {
    let id = request.get("id")?;
    let params = &request["params"];
    let owns_app = || {
        peer.app_session
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|owner| session == Some(owner))
    };
    let descriptor = |name: &str, visibility: &[&str]| {
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
            let mut initialized = json!({
                "protocolVersion": version,
                "capabilities": {
                    "tools": {"listChanged": peer.sends_catalog_notifications},
                    "resources": {"listChanged": peer.sends_catalog_notifications},
                },
                "serverInfo": {"name": "local-app-peer", "version": "1"}
            });
            if peer.offers_task {
                initialized["capabilities"]["tasks"] = json!({"cancel":{},"requests":{"tools":{"call":{}}}});
            }
            Some(initialized)
        }
        "tools/list" => {
            let mut tools = vec![
                descriptor("render", ["model"].as_slice()),
                descriptor("app_action", ["app"].as_slice()),
                descriptor("model_action", ["model"].as_slice()),
            ];
            if owns_app() {
                let version = peer.catalog_version.load(Ordering::Relaxed);
                if version >= 1 {
                    tools[2]["description"] = json!("Changed model-only tool");
                }
                if version >= 2 {
                    tools.push(descriptor("app_refresh", &["app"]));
                }
            }
            if peer.frame_origin.is_some() {
                tools.push(descriptor("disabled_action", ["app"].as_slice()));
                tools.extend((1..=50).map(|index| {
                    descriptor(&format!("view_action_{index:02}"), ["app"].as_slice())
                }));
            }
            if peer.offers_task {
                tools.push(json!({
                    "name": "app_task", "inputSchema": {"type": "object"},
                    "execution": {"taskSupport": "required"},
                    "_meta": {"ui":{"visibility":["app"]}},
                }));
            }
            Some(json!({"tools": tools}))
        }
        "tools/call" if params["name"] == "render" && session.is_some() => {
            *peer.app_session.lock().unwrap() = session.map(str::to_owned);
            Some(json!({
                "content": [{"type": "text", "text": "Model-visible MCP result"}],
                "structuredContent": {"item": 42},
                "_meta": {"private": APP_META},
            }))
        }
        "tools/call" if params["name"] == "app_action" && owns_app() => Some(json!({
            "content": [{"type": "text", "text": "App action acknowledged"}],
            "structuredContent": {"received": params["arguments"]["value"]},
        })),
        "tools/call" if peer.offers_task && params["name"] == "app_task"
            && params["task"].is_object() && owns_app() =>
        {
            Some(json!({"task":task_info("originating-peer-task", "working")}))
        }
        "tasks/get" if peer.offers_task && params["taskId"] == "originating-peer-task"
            && owns_app() =>
        {
            Some(task_info("originating-peer-task", "completed"))
        }
        "tasks/result" if peer.offers_task && params["taskId"] == "originating-peer-task"
            && owns_app() =>
        {
            Some(json!({
                "content": [{"type":"text", "text":"Originating MCP task completed"}],
                "_meta": {"io.modelcontextprotocol/related-task":{"taskId":"originating-peer-task"}},
            }))
        }
        "resources/list" if owns_app() => {
            if params["cursor"] == "app-page" {
                let mut resources = vec![json!({
                    "uri": URI, "name": "Fixture App", "mimeType": "text/html;profile=mcp-app",
                    "_meta": {"ui": {"prefersBorder": true, "csp": {
                        "frameDomains": [peer.frame_origin, "https://192.0.2.111", "http://*.fixture.example"]
                    }}}
                })];
                if peer.resource_catalog_version.load(Ordering::Relaxed) == 2 {
                    resources.push(json!({"uri":"fixture://later-report","name":"Second page report"}));
                }
                Some(json!({"resources": resources}))
            } else {
                let mut resources = vec![
                    json!({"uri":"fixture://data","name":"App data","mimeType":"text/plain"}),
                    json!({"uri":"ui://fixture/other","name":"Another App","mimeType":"text/html;profile=mcp-app"}),
                ];
                if peer.resource_catalog_version.load(Ordering::Relaxed) == 1 {
                    resources.push(json!({"uri":"fixture://report","name":"New report"}));
                }
                let mut result = json!({"resources": resources});
                if peer.frame_origin.is_some() {
                    result["nextCursor"] = json!("app-page");
                }
                Some(result)
            }
        }
        "resources/templates/list" if owns_app() => Some(json!({
            "resourceTemplates": [
                {"uriTemplate":"fixture://report/{name}","name":"Report","mimeType":"text/plain"},
                {"uriTemplate":"ui://fixture/{screen}","name":"Other views","mimeType":"text/html;profile=mcp-app"}
            ]
        })),
        "resources/read" if params["uri"] == URI && owns_app() => {
            let mut document = json!({
                "uri": URI, "mimeType": "text/html;profile=mcp-app",
                "text": "<main>Fixture MCP App</main>",
            });
            if peer.frame_origin.is_none() || peer.app_reads.fetch_add(1, Ordering::Relaxed) != 0 {
                document["_meta"] = json!({"ui": {"prefersBorder": false, "csp": {
                    "connectDomains": ["*; script-src *"], "frameDomains": ["https://192.0.2.111"]
                }}});
            }
            Some(json!({"contents": [document]}))
        }
        "resources/read" if params["uri"] == "fixture://data" && owns_app() => {
            Some(json!({"contents": [{
                "uri": "fixture://data", "mimeType": "text/plain", "text": "Fixture data",
            }]}))
        }
        _ => None,
    };
    Some(match result {
        Some(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        None => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Fixture method unavailable"}})
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_app_bridge_uses_live_mcp_policy_and_the_original_tool_result() -> TestResult {
    let address = native_fixture_address()?;
    let peer = Arc::new(AppPeer {
        frame_origin: Some(format!("http://{address}")),
        sends_catalog_notifications: true,
        ..Default::default()
    });
    let setup = Arc::clone(&peer);
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin("agena.mcp")
        .before_start(move |case| async move {
            let server = case.serve_http_on(address, Router::new()
                .route("/mcp", post(app_peer).get(|| async { StatusCode::METHOD_NOT_ALLOWED }))
                .with_state(setup)).await?;
            let mut config: Value = serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["agena.mcp"] = json!({
                "enabled": true, "package": {"kind":"static"},
                "settings": {
                    "runtime": {"reconnect": {"enabled": false}},
                    "servers": {(SERVER): {
                        "transport": "http", "endpoint": {"url": server.url("/mcp")},
                        "tools": {"exclude": ["disabled_action"]},
                    }},
                },
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .run(move |case| async move {
            super::super::cordis::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let mut policy = serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
            policy["network"]["deny"] = json!([{"to": "192.0.2.111"}]);
            user.client().set_session_permission(user.id(), serde_json::from_value(policy)?).await?;
            let search = user.call_tool("mcp.tools.search", json!({"query":""})).await?;
            search.assert_succeeded()?;
            assert!(!search.data()?.to_string().contains("app_action"));
            user.call_tool("mcp.tools.call", json!({"server":SERVER,"name":"app_action"})).await?.assert_failed()?;
            user.call_tool("mcp.resources.read", json!({"server":SERVER,"uri": URI})).await?.assert_failed()?;
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
            assert_eq!(document["ui"], json!({
                "csp": {"frameDomains": [format!("http://{}", native_fixture_address()?)]},
                "prefersBorder": true,
            }));
            let current: Value = http.get(&path).await?.require_success()?.json()?;
            assert_eq!(current["ui"], json!({"csp": {}, "prefersBorder": false}));
            assert_eq!(document["arguments"]["selection"], 1);
            assert_eq!(document["tool_result"]["structuredContent"]["item"], 42);
            assert_eq!(document["tool_result"]["_meta"]["private"], APP_META);
            let revision = document["revision"].as_str().context("MCP App revision")?;
            let catalog_revision = document["tool_catalog_revision"].as_str().context("MCP App tool catalog revision")?;
            let resource_revision = document["resource_catalog_revision"].as_str().context("MCP App resource catalog revision")?;
            let changes = |after: &str| {
                http.request(Method::GET, &format!("{path}/tools/changes?after={after}"))
                    .map(|request| request.header("if-match", format!("\"{revision}\"")))
            };
            let invoke = |body: Value| http.request(Method::POST, &path).map(|request| {
                request.header("if-match", format!("\"{revision}\"")).json(&body)
            });
            let list = |params: Value| invoke(json!({"method":"tools/list", "params": params}));
            let first: Value = http.execute(list(json!({}))?).await?.require_success()?.json()?;
            let tools = first["tools"].as_array().context("MCP App tool descriptors")?;
            assert_eq!(tools.len(), 50);
            assert_eq!(tools[0]["name"], "app_action");
            assert_eq!(tools[0]["inputSchema"]["type"], "object");
            assert!(tools[1..].iter().enumerate().all(|(index, tool)| {
                tool["name"] == format!("view_action_{:02}", index + 1)
            }));
            let last: Value = http.execute(list(json!({"cursor": first["nextCursor"]}))?).await?.require_success()?.json()?;
            assert_eq!(last["tools"].as_array().context("final MCP App tool page")?.len(), 1);
            assert_eq!(last["tools"][0]["name"], "view_action_50");
            assert!(last.get("nextCursor").is_none());
            assert_eq!(http.execute(list(json!({"cursor":"unexpected"}))?).await?.status, StatusCode::BAD_REQUEST);
            let resources = |params: Value| invoke(json!({"method":"resources/list","params":params}));
            let available: Value = http.execute(resources(json!({}))?).await?.require_success()?.json()?;
            assert_eq!(available["resources"], json!([
                {"uri":"fixture://data","name":"App data","mimeType":"text/plain"}
            ]));
            assert_eq!(available["nextCursor"], "app-page");
            let available: Value = http.execute(resources(json!({"cursor":available["nextCursor"]}))?).await?.require_success()?.json()?;
            assert_eq!(available["resources"].as_array().context("listed App resource")?.len(), 1);
            assert_eq!(available["resources"][0]["uri"], URI);
            let resource_changes = |after: &str| {
                http.request(Method::GET, &format!("{path}/resources/changes?after={after}"))
                    .map(|request| request.header("if-match", format!("\"{revision}\"")))
            };
            let unchanged: Value = http.execute(resource_changes(resource_revision)?).await?.require_success()?.json()?;
            assert_eq!(unchanged["revision"], resource_revision);
            assert!(unchanged["retry_after_ms"].as_u64().is_some_and(|delay| delay >= 1000));
            peer.resource_catalog_version.store(2, Ordering::Relaxed);
            let updated: Value = http.execute(resource_changes(resource_revision)?).await?.require_success()?.json()?;
            assert_ne!(updated["revision"], resource_revision);
            let templates: Value = http.execute(invoke(json!({"method":"resources/templates/list","params":{}}))?)
                .await?.require_success()?.json()?;
            assert_eq!(templates["resourceTemplates"], json!([
                {"uriTemplate":"fixture://report/{name}","name":"Report","mimeType":"text/plain"}
            ]));
            assert_eq!(http.execute(resources(json!({"cursor":"bad\ncursor"}))?).await?.status, StatusCode::BAD_REQUEST);
            let data: Value = http.execute(invoke(json!({"method":"resources/read","params":{"uri":"fixture://data"}}))?).await?.require_success()?.json()?;
            assert_eq!(data["contents"][0]["text"], "Fixture data");
            let action = json!({"method":"tools/call","params":{"name":"app_action","arguments":{"value":7}}});
            let result: Value = http.execute(invoke(action.clone())?).await?.require_success()?.json()?;
            assert_eq!(result["structuredContent"]["received"], 7);
            peer.catalog_version.store(1, Ordering::Relaxed);
            let unchanged: Value = http.execute(changes(catalog_revision)?).await?.require_success()?.json()?;
            assert_eq!(unchanged["revision"], catalog_revision);
            assert!(unchanged["retry_after_ms"].as_u64().is_some_and(|delay| delay >= 1000));
            peer.catalog_version.store(2, Ordering::Relaxed);
            let updated: Value = http.execute(changes(catalog_revision)?).await?.require_success()?.json()?;
            assert_ne!(updated["revision"], catalog_revision);
            let list_after: Value = http.execute(list(json!({}))?).await?.require_success()?.json()?;
            assert_eq!(list_after["tools"][1]["name"], "app_refresh");
            assert_eq!(http.execute(invoke(json!({"method":"tools/call","params":{"name":"render"}}))?).await?.status, StatusCode::CONFLICT);
            assert_eq!(http.execute(invoke(json!({"method":"tools/call","params":{"name":"model_action"}}))?).await?.status, StatusCode::CONFLICT);
            let wrong_revision = http.request(Method::POST, &path)?
                .header("if-match", format!("\"{}\"", "0".repeat(64)))
                .json(&action);
            assert_eq!(http.execute(wrong_revision).await?.status, StatusCode::CONFLICT);
            let different = case.user().new_session().await?;
            assert_eq!(http.get(&format!("/api/v1/sessions/{}/parts/{part}/mcp-app", different.id())).await?.status, StatusCode::NOT_FOUND);
            let mut policy = serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
            policy["tools"]["names"]["mcp.tools.call"] = json!("deny");
            policy["tools"]["names"]["agena.mcp.tools.call"] = json!("deny");
            policy["tools"]["names"]["mcp.tools.search"] = json!("deny");
            policy["tools"]["names"]["agena.mcp.tools.search"] = json!("deny");
            policy["tools"]["names"]["mcp.resources.list"] = json!("deny");
            policy["tools"]["names"]["agena.mcp.resources.list"] = json!("deny");
            user.client().set_session_permission(session, serde_json::from_value(policy)?).await?;
            assert_eq!(http.execute(invoke(action)?).await?.status, StatusCode::CONFLICT);
            assert_eq!(http.execute(list(json!({}))?).await?.status, StatusCode::CONFLICT);
            assert_eq!(http.execute(changes(catalog_revision)?).await?.status, StatusCode::CONFLICT);
            assert_eq!(http.execute(resource_changes(resource_revision)?).await?.status, StatusCode::CONFLICT);
            let restricted: Value = http.get(&path).await?.require_success()?.json()?;
            assert!(restricted["resource_catalog_revision"].is_null());
            assert_eq!(http.execute(resources(json!({}))?).await?.status, StatusCode::CONFLICT);
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_app_bridge_negotiates_wildcard_csp_and_device_permissions() -> TestResult {
    let address = native_fixture_address()?;
    let owner = Arc::new(Mutex::new(None::<String>));
    let next_session = Arc::new(AtomicU64::new(0));
    let owner_route = Arc::clone(&owner);
    let next_route = Arc::clone(&next_session);
    let handler = move |headers: HeaderMap, Json(request): Json<Value>| {
        let owner = Arc::clone(&owner_route);
        let next_session = Arc::clone(&next_route);
        async move {
            let session = headers
                .get("mcp-session-id")
                .and_then(|value| value.to_str().ok());
            let Some(id) = request.get("id").cloned() else {
                return StatusCode::ACCEPTED.into_response();
            };
            let params = &request["params"];
            let owns = || {
                owner
                    .lock()
                    .unwrap()
                    .as_deref()
                    .is_some_and(|s| session == Some(s))
            };
            let result = match request["method"].as_str().unwrap_or_default() {
                "initialize" => Some(json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {"tools": {}, "resources": {}},
                    "serverInfo": {"name": "wildcard-app-peer", "version": "1"}
                })),
                "tools/list" => Some(json!({"tools": [{
                    "name": "render_media_app",
                    "inputSchema": {"type": "object"},
                    "_meta": {"ui": {"resourceUri": URI, "visibility": ["model"]}}
                }]})),
                "tools/call" if params["name"] == "render_media_app" && session.is_some() => {
                    *owner.lock().unwrap() = session.map(str::to_owned);
                    Some(json!({"content": [{"type": "text", "text": "Opened wildcard app"}]}))
                }
                "resources/list" if owns() => Some(json!({"resources": []})),
                "resources/read" if params["uri"] == URI && owns() => Some(json!({
                    "contents": [{
                        "uri": URI,
                        "mimeType": "text/html;profile=mcp-app",
                        "text": "<main>Wildcard MCP App</main>",
                        "_meta": {"ui": {
                            "prefersBorder": true,
                            "csp": {
                                "connectDomains": [
                                    "wss://*.events.example",
                                    "https://*.api.example:8443",
                                    "http://*.unsafe.example",
                                    "https://*"
                                ],
                                "resourceDomains": [
                                    "https://*.cdn.example",
                                    "wss://*.wrong.example",
                                    "https://*.localhost"
                                ],
                                "frameDomains": [
                                    "https://*.embed.example"
                                ]
                            },
                            "permissions": {
                                "camera": {},
                                "clipboardWrite": true,
                                "geolocation": false,
                                "usb": {}
                            }
                        }}
                    }]
                })),
                _ => None,
            };
            let body = match result {
                Some(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                None => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Unavailable"}}),
            };
            let mut response = Json(body).into_response();
            if request["method"] == "initialize" {
                let n = next_session.fetch_add(1, Ordering::Relaxed) + 1;
                response
                    .headers_mut()
                    .insert("mcp-session-id", format!("wildcard-{n}").parse().unwrap());
            }
            response
        }
    };
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin("agena.mcp")
        .before_start(move |case| async move {
            let server = case
                .serve_http_on(
                    address,
                    Router::new().route(
                        "/mcp",
                        post(handler).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
                    ),
                )
                .await?;
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["agena.mcp"] = json!({
                "enabled": true, "package": {"kind": "static"},
                "settings": {
                    "runtime": {"reconnect": {"enabled": false}},
                    "servers": {(SERVER): {"transport": "http", "endpoint": {"url": server.url("/mcp")}}}
                }
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .run(move |case| async move {
            super::super::cordis::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            // Explicitly restrict outbound network to local fixture address only.
            let mut restricted_policy =
                serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
            restricted_policy["network"] = json!({
                "default": "deny",
                "allow": [{"to": address.to_string()}]
            });
            user.client()
                .set_session_permission(user.id(), serde_json::from_value(restricted_policy)?)
                .await?;

            let rendered = user
                .call_tool(
                    "mcp.tools.call",
                    json!({"server": SERVER, "name": "render_media_app"}),
                )
                .await?;
            rendered.assert_succeeded()?;
            let part = rendered.part.part_id;
            let path = format!("/api/v1/sessions/{}/parts/{part}/mcp-app", user.id());
            let http = case.http().await;

            // Under default:deny network policy, wildcard subdomains must be filtered out,
            // while declared supported device permissions are surfaced for browser consent.
            let doc_restricted: Value = http.get(&path).await?.require_success()?.json()?;
            assert_eq!(
                doc_restricted["ui"],
                json!({
                    "csp": {},
                    "prefersBorder": true,
                    "permissions": {
                        "camera": {},
                        "clipboardWrite": {}
                    }
                })
            );

            // Under default:allow network policy, strict TLS subdomain wildcards are admitted,
            // while insecure/malformed wildcards remain rejected.
            let mut open_policy =
                serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
            open_policy["network"] = json!({
                "default": "allow",
                "allow": [{"to": address.to_string()}]
            });
            user.client()
                .set_session_permission(user.id(), serde_json::from_value(open_policy)?)
                .await?;
            let rendered_open = user
                .call_tool(
                    "mcp.tools.call",
                    json!({"server": SERVER, "name": "render_media_app"}),
                )
                .await?;
            rendered_open.assert_succeeded()?;
            let open_path = format!(
                "/api/v1/sessions/{}/parts/{}/mcp-app",
                user.id(),
                rendered_open.part.part_id
            );
            let doc_open: Value = http.get(&open_path).await?.require_success()?.json()?;
            assert_eq!(
                doc_open["ui"],
                json!({
                    "csp": {
                        "connectDomains": ["wss://*.events.example", "https://*.api.example:8443"],
                        "resourceDomains": ["https://*.cdn.example"],
                        "frameDomains": ["https://*.embed.example"]
                    },
                    "prefersBorder": true,
                    "permissions": {
                        "camera": {},
                        "clipboardWrite": {}
                    }
                })
            );
            Ok(())
        })
        .await
}
