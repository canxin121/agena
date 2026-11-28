use std::convert::Infallible;
use std::{sync::atomic::AtomicBool, time::Duration};

use agena_testkit::{ModelHandle, Reply, RequestMatch, TestAgenaBuilder, UserSession};
use axum::response::{Sse, sse::Event};
use futures_util::{StreamExt, stream};
use tokio::sync::oneshot;

use super::*;

async fn app_tools(
    model: &ModelHandle,
    user: &UserSession,
    label: &str,
) -> anyhow::Result<Vec<String>> {
    let query = format!("list MCP App tools {label}");
    let call_id = format!("list-view-tools-{label}");
    let listing = model
        .expect(RequestMatch::user_text(&query))
        .reply(Reply::function_call(
            &call_id,
            "tools_list",
            json!({"plugin":"mcp", "limit":100}),
        ));
    let names = Arc::new(Mutex::new(Vec::new()));
    let output = names.clone();
    let observed_call = call_id.clone();
    model
        .expect(RequestMatch::tool_output(&call_id))
        .after(&listing)
        .check(move |request| {
            let text = request
                .tool_output_text(&observed_call)
                .context("tools_list output")?;
            let text = serde_json::from_str::<String>(&text).unwrap_or(text);
            ensure!(
                !text
                    .lines()
                    .any(|line| line.starts_with("- mcp.apps.call "))
            );
            *output.lock().unwrap() = text
                .lines()
                .filter_map(|line| line.strip_prefix("- "))
                .filter_map(|line| line.split_ascii_whitespace().next())
                .filter(|name| name.starts_with("mcp_app__"))
                .map(str::to_owned)
                .collect();
            Ok(())
        })
        .reply(Reply::text("App tools inspected."));
    user.send(query)
        .await?
        .wait_settled()
        .await?
        .assert_assistant_text("App tools inspected.")?;
    Ok(std::mem::take(&mut *names.lock().unwrap()))
}

fn publication(name: &str) -> Value {
    json!({"tools":[{
        "name":name, "title":"Double a value", "description":"Read the current App calculation.",
        "inputSchema":{"type":"object", "properties":{"value":{"type":"integer"}}, "required":["value"]},
        "outputSchema":{"type":"object", "properties":{"doubled":{"type":"integer"}}, "required":["doubled"]}
    }]})
}

fn task_input(id: &str) -> Value {
    let mut input = task_info(id, "input_required");
    input["resultType"] = json!("complete");
    input["inputRequests"] = json!({
        "confirm": {
            "method":"elicitation/create",
            "params":{
                "mode":"form", "message":"Choose how the App should calculate.",
                "requestedSchema":{
                    "type":"object", "required":["method"], "additionalProperties":false,
                    "properties":{"method":{"type":"string","enum":["Double","Triple"]}},
                }
            }
        }
    });
    input
}

fn view_fixture() -> TestAgenaBuilder {
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin("agena.mcp")
        .before_start(|case| async move {
            let server = case.serve_http_on(native_fixture_address()?, Router::new()
                .route("/mcp", post(app_peer).get(|| async { StatusCode::METHOD_NOT_ALLOWED }))
                .with_state(Arc::new(AppPeer { offers_task: true, ..Default::default() }))).await?;
            let mut config: Value = serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["agena.mcp"] = json!({
                "enabled":true, "package":{"kind":"static"},
                "settings":{
                    "runtime":{"reconnect":{"enabled":false}},
                    "servers":{(SERVER):{"transport":"http", "endpoint":{"url":server.url("/mcp")}}},
                }
            });
            case.write_config(SettingsLayer::Global, config).await
        })
}

struct TaskPeer {
    app: AppPeer,
    modern: bool,
    extension_view: bool,
    release: AtomicBool,
    answers: Mutex<BTreeMap<String, BTreeMap<String, Value>>>,
    legacy_inputs: Mutex<BTreeMap<String, oneshot::Sender<()>>>,
    started: AtomicU64,
    cancelled: tokio::sync::Notify,
}

fn server_task_inputs(id: &str) -> Value {
    let mut input = task_input(id);
    input["inputRequests"]["site"] = json!({
        "method":"elicitation/create",
        "params":{
            "mode":"url", "message":"Open the provider", "url":"https://example.invalid/consent",
            "elicitationId":"confirm-provider",
            "_meta":{"private":"connection-metadata"},
        }
    });
    if id == "server-native-1" {
        input["inputRequests"]["workspace"] = json!({"method":"roots/list"});
        input["inputRequests"]["unsafe_url"] = json!({
            "method":"elicitation/create",
            "params":{
                "mode":"url", "message":"Invalid URL", "url":"https://user:pass@example.invalid",
                "elicitationId":"unsafe-url",
            }
        });
        input["inputRequests"]["sample"] = json!({
            "method":"sampling/createMessage",
            "params":{
                "messages":[{"role":"user","content":{"type":"text","text":"Provide a short answer."}}],
                "maxTokens":20, "metadata":{"private":"connection-metadata"},
            }
        });
    }
    input
}

fn app_input_key<'a>(
    status: &'a Value,
    method: &str,
    mode: Option<&str>,
) -> anyhow::Result<&'a str> {
    status["inputRequests"]
        .as_object()
        .context("App input requests")?
        .iter()
        .find(|(_, request)| {
            request["method"] == method && request["params"]["mode"].as_str() == mode
        })
        .map(|(id, _)| id.as_str())
        .with_context(|| format!("App input for {method} {mode:?}"))
}

async fn relay_peer(
    State(peer): State<Arc<TaskPeer>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let id = &request["id"];
    let params = &request["params"];
    let session = if peer.modern {
        Some("modern-app-peer")
    } else {
        headers
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
    };
    let owns_peer =
        || session.is_some() && session == peer.app.app_session.lock().unwrap().as_deref();
    if !peer.modern
        && owns_peer()
        && let Some(task) = id.as_str().and_then(|id| id.strip_prefix("legacy-input:"))
        && let Some(send) = peer.legacy_inputs.lock().unwrap().remove(task)
    {
        let mut reply = request["result"].clone();
        let metadata = reply.as_object_mut().unwrap().remove("_meta").unwrap();
        assert_eq!(
            metadata["io.modelcontextprotocol/related-task"]["taskId"],
            task
        );
        if task == "server-native-2" {
            // Closing the view may cancel the outstanding callback before
            // the task driver sends tasks/cancel itself.
            assert_eq!(reply, json!({"action":"cancel"}));
            return StatusCode::ACCEPTED.into_response();
        }
        assert_eq!(
            reply,
            json!({"action":"accept","content":{"method":"Double"}}),
            "legacy callback for {task}",
        );
        peer.answers
            .lock()
            .unwrap()
            .insert(task.into(), BTreeMap::from([("confirm".into(), reply)]));
        let _ = send.send(());
        return StatusCode::ACCEPTED.into_response();
    }
    let final_result = || {
        json!({
            "content":[{"type":"text","text":"The detached server task finished"}],
            "structuredContent":{"doubled":14},
            "_meta":{"io.modelcontextprotocol/related-task":{"taskId":params["taskId"]}},
        })
    };
    let result = match request["method"].as_str() {
        Some("server/discover") if peer.modern => json!({
            "resultType":"complete", "supportedVersions":["2026-07-28"],
            "capabilities":{"tools":{}, "resources":{}, "extensions":{"io.modelcontextprotocol/tasks":{}}},
            "ttlMs":0, "cacheScope":"private",
            "_meta":{"io.modelcontextprotocol/serverInfo":{"name":"relay-fixture","version":"1"}},
        }),
        Some("tools/call") if params["name"] == "app_task" && owns_peer() => {
            let index = peer.started.fetch_add(1, Ordering::SeqCst);
            let task = format!("server-native-{index}");
            let mut info = task_info(&task, "working");
            info["pollInterval"] = json!(75);
            if peer.modern {
                assert!(params.get("task").is_none());
                info["resultType"] = json!("task");
                info
            } else {
                assert_eq!(
                    params["task"],
                    if peer.extension_view {
                        json!({})
                    } else {
                        json!({"ttl":45_000})
                    }
                );
                json!({"task":info})
            }
        }
        Some("tasks/get") if owns_peer() => {
            let task = params["taskId"].as_str().unwrap();
            assert!(task.starts_with("server-native-"));
            if !peer.release.load(Ordering::SeqCst) {
                let mut info = task_info(task, "working");
                info["pollInterval"] = json!(75);
                if peer.modern {
                    info["resultType"] = json!("complete");
                }
                info
            } else if peer.modern
                && !peer.extension_view
                && peer.answers.lock().unwrap().get(task).is_none()
            {
                task_input(task)
            } else if peer.modern
                && peer.extension_view
                && peer
                    .answers
                    .lock()
                    .unwrap()
                    .get(task)
                    .map_or(0, BTreeMap::len)
                    < if task == "server-native-1" { 5 } else { 2 }
            {
                // Repeat acknowledged upstream input IDs to exercise deduplication.
                server_task_inputs(task)
            } else if !peer.modern {
                task_info(task, "input_required")
            } else {
                let mut info = task_info(task, "completed");
                if peer.modern {
                    info["resultType"] = json!("complete");
                    info["result"] = final_result();
                }
                info
            }
        }
        Some("tasks/update") if peer.modern && owns_peer() => {
            let task = params["taskId"].as_str().unwrap();
            assert!(matches!(task, "server-native-0" | "server-native-1"));
            let mut answers = peer.answers.lock().unwrap();
            let answers = answers.entry(task.into()).or_default();
            for (key, answer) in params["inputResponses"].as_object().unwrap() {
                match key.as_str() {
                    "confirm" => assert_eq!(
                        *answer,
                        json!({"action":"accept","content":{"method":"Double"}})
                    ),
                    "site" if task == "server-native-1" => {
                        assert_eq!(*answer, json!({"action":"accept"}))
                    }
                    "site" | "unsafe_url" => assert_eq!(*answer, json!({"action":"decline"})),
                    "workspace" => assert!(answer["roots"].is_array()),
                    "sample" => assert_eq!(
                        *answer,
                        json!({
                            "model":"app-model", "role":"assistant", "content":{"type":"text","text":"App answer"},
                        })
                    ),
                    _ => panic!("unexpected MCP task input key: {key}"),
                }
                assert!(
                    answers.insert(key.clone(), answer.clone()).is_none(),
                    "duplicate answer: {key}"
                );
            }
            json!({"resultType":"complete"})
        }
        Some("tasks/result") if !peer.modern && owns_peer() => {
            let task = params["taskId"].as_str().unwrap();
            let (send, receive) = oneshot::channel();
            assert!(
                peer.legacy_inputs
                    .lock()
                    .unwrap()
                    .insert(task.into(), send)
                    .is_none()
            );
            let mut elicitation = task_input(task)["inputRequests"]["confirm"].clone();
            elicitation["jsonrpc"] = json!("2.0");
            elicitation["id"] = json!(format!("legacy-input:{task}"));
            elicitation["params"]["_meta"] =
                json!({"io.modelcontextprotocol/related-task":{"taskId":task}});
            let response = json!({"jsonrpc":"2.0","id":id,"result":final_result()});
            let start = stream::iter([Ok::<_, Infallible>(
                Event::default().data(elicitation.to_string()),
            )]);
            let end = stream::once(async move {
                let result = receive.await.ok().map(|_| response);
                result.map(|result| Ok(Event::default().data(result.to_string())))
            })
            .filter_map(std::future::ready);
            return Sse::new(start.chain(end)).into_response();
        }
        Some("tasks/cancel") if owns_peer() => {
            let task = params["taskId"].as_str().unwrap();
            peer.legacy_inputs.lock().unwrap().remove(task);
            if task == "server-native-2" {
                peer.cancelled.notify_one();
            }
            if peer.modern {
                json!({"resultType":"complete"})
            } else {
                task_info(task, "cancelled")
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
    Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_app_server_task_transfers_from_the_view_to_the_live_tool() -> TestResult {
    server_task_handoff(false, false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn modern_app_server_task_transfers_and_requests_input_after_view_closes() -> TestResult {
    server_task_handoff(true, true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn modern_app_tasks_translate_legacy_server_callbacks_on_the_original_connection()
-> TestResult {
    server_task_handoff(false, true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_app_tasks_translate_modern_server_inputs_on_the_original_connection() -> TestResult
{
    server_task_handoff(true, false).await
}

async fn server_task_handoff(modern: bool, extension_view: bool) -> TestResult {
    let peer = Arc::new(TaskPeer {
        app: AppPeer {
            offers_task: true,
            ..Default::default()
        },
        modern,
        extension_view,
        release: AtomicBool::new(false),
        answers: Mutex::new(BTreeMap::new()),
        legacy_inputs: Mutex::new(BTreeMap::new()),
        started: AtomicU64::new(0),
        cancelled: tokio::sync::Notify::new(),
    });
    let setup = peer.clone();
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin("agena.mcp")
        .case_timeout(Duration::from_secs(90))
        .before_start(move |case| async move {
            let server = case.serve_http_on(native_fixture_address()?, Router::new()
                .route("/mcp", post(relay_peer).get(|| async { StatusCode::METHOD_NOT_ALLOWED }))
                .with_state(setup)).await?;
            let mut config: Value = serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["agena.mcp"] = json!({
                "enabled":true, "package":{"kind":"static"},
                "settings":{
                    "runtime":{"reconnect":{"enabled":false}},
                    "servers":{(SERVER):{"transport":"http", "endpoint":{"url":server.url("/mcp")}}},
                }
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .run(move |case| async move {
            super::super::super::cordis::wait_for_startup_catalog(&case).await?;
            let owner = case.user().new_session().await?;
            allow_permissions(&owner).await?;
            let rendered = owner.call_tool("mcp.tools.call", json!({"server":SERVER,"name":"render"})).await?;
            rendered.assert_succeeded()?;
            let http = case.http().await;
            let path = format!("/api/v1/sessions/{}/parts/{}/mcp-app", owner.id(), rendered.part.part_id);
            let document: Value = http.get(&path).await?.require_success().context("open the retained App document")?.json()?;
            let revision = document["revision"].as_str().context("App revision")?;
            let request = |method, target: &str| {
                http.request(method, target).map(|request| request.header("if-match", format!("\"{revision}\"")))
            };
            let open_view = async |publication: Value| {
                let opened: Value = http.execute(request(Method::POST, &format!("{path}/views"))?.json(&publication))
                    .await?.require_success().context("register an App view")?.json()?;
                let id = opened["view"].as_str().context("view ID")?;
                let view = format!("{path}/views/{id}");
                http.execute(request(Method::GET, &format!("{view}/calls"))?).await?.require_success()?;
                Ok::<_, anyhow::Error>(view)
            };
            let mut tools = publication("delegate_calculation");
            if extension_view { tools["task_extension"] = json!(true); }
            else { tools["tools"][0]["execution"] = json!({"taskSupport":"required"}); }
            let view = open_view(tools).await?;
            let leftover = open_view(json!({"tools":[],"task_extension":extension_view})).await?;
            let names = app_tools(&case.model(), &owner, "server-handoff").await?;
            assert_eq!(names.len(), 1);
            let running = owner.start_tool(&names[0], json!({"value":7})).await?;
            let calls = format!("{view}/calls");
            let call = tokio::time::timeout(Duration::from_secs(12), async {
                let mut next = calls.clone();
                loop {
                    let pending: Value = http.execute(request(Method::GET, &next)?)
                        .await?.require_success()?.json()?;
                    if let Some(id) = pending["calls"][0]["id"].as_str() {
                        return Ok::<_, anyhow::Error>(id.to_owned());
                    }
                    next = format!("{calls}?after={}", pending["revision"]);
                }
            }).await??;
            let relay = |view: &str, input: &Value| {
                request(Method::POST, &format!("{view}/server")).map(|request| request.json(input))
            };
            let status_for = async |view: &str, task_id: &str, status: &str| {
                let get = json!({"method":"tasks/get","params":{"taskId":task_id}});
                tokio::time::timeout(Duration::from_secs(8), async {
                    loop {
                        let state: Value = http.execute(relay(view, &get)?).await?.require_success()?.json()?;
                        if state["status"] == status {
                            return Ok::<_, anyhow::Error>(state);
                        }
                        ensure!(state["status"] == "working" || state["status"] == "input_required", "{state}");
                        tokio::time::sleep(Duration::from_millis(30)).await;
                    }
                }).await.with_context(|| format!("wait for App task status {status}"))?
            };
            let update = |id: &str, answers: Value| {
                json!({"method":"tasks/update","params":{"taskId":id,"inputResponses":answers}})
            };
            let mut start = json!({"method":"tools/call","params":{"name":"app_task"}});
            if !extension_view { start["params"]["task"] = json!({"ttl":45_000}); }
            let handle: Value = http.execute(relay(&view, &start)?).await?.require_success().context("receive the server task handle before completion")?.json()?;
            let task = if extension_view { assert_eq!(handle["resultType"], "task"); &handle }
                else { assert_eq!(handle["task"]["ttl"], 300_000); &handle["task"] };
            let task_id = task["taskId"].as_str().context("proxy task ID")?;
            ensure!(task_id.starts_with("agena-app-task:") && !handle.to_string().contains("server-native-0"));
            let get = json!({"method":"tasks/get","params":{"taskId":task_id}});
            let working: Value = http.execute(relay(&view, &get)?).await?.require_success()?.json()?;
            assert_eq!(working["status"], "working");
            assert_eq!(http.execute(relay(&leftover, &get)?).await?.status, StatusCode::CONFLICT);
            let mut transferred_input = None;
            if extension_view {
                peer.release.store(true, Ordering::SeqCst);
                let pending = status_for(&view, task_id, "input_required").await?;
                let form = app_input_key(&pending, "elicitation/create", Some("form"))?;
                let wrong_view = update(task_id, json!({(form):{"action":"decline"}}));
                assert_eq!(http.execute(relay(&leftover, &wrong_view)?).await?.status, StatusCode::CONFLICT);
                if modern {
                    let site = app_input_key(&pending, "elicitation/create", Some("url"))?;
                    let answer = update(task_id, json!({(site):{"action":"decline"}}));
                    let ack: Value = http.execute(relay(&view, &answer)?).await?.require_success()?.json()?;
                    assert_eq!(ack, json!({"resultType":"complete"}));
                }
                transferred_input = Some(form.to_owned());
            }
            http.execute(request(Method::POST, &format!("{calls}/{call}"))?.json(&json!({
                "status":"result", "result":handle,
            }))).await?.require_success()?;
            if let Some(id) = transferred_input {
                let status: Value = http.execute(relay(&view, &get)?).await?.require_success()?.json()?;
                ensure!(status.get("inputRequests").is_none(), "{status}");
                let answer = update(task_id, json!({(id):{"action":"decline"}}));
                assert_eq!(http.execute(relay(&view, &answer)?).await?.status, StatusCode::BAD_REQUEST);
                let cancel = json!({"method":"tasks/cancel","params":{"taskId":task_id}});
                assert_eq!(http.execute(relay(&view, &cancel)?).await?.status, StatusCode::CONFLICT);
            }
            http.execute(request(Method::DELETE, &view)?).await?.require_success()?;
            assert_eq!(http.execute(request(Method::GET, &calls)?).await?.status, StatusCode::CONFLICT);
            peer.release.store(true, Ordering::SeqCst);
            let interaction = running.wait_user_input().await?;
            assert_eq!(interaction.session_id(), owner.id());
            interaction.answer(BTreeMap::from([("0".into(), vec!["Double".into()])])).await?;
            let result = tokio::time::timeout(Duration::from_secs(20), running.wait_settled())
                .await.context("finish the handed-off MCP task after replying on the original connection")??;
            result.assert_succeeded()?;
            result.assert_text_contains("The detached server task finished")?;
            assert_eq!(result.data()?["structured_content"], json!({"doubled":14}));
            assert_eq!(peer.answers.lock().unwrap().get("server-native-0").map_or(0, BTreeMap::len),
                if modern && extension_view { 2 } else { 1 });

            let ordinary: Value = http.execute(relay(&leftover, &json!({
                "method":"tools/call",
                "params":{"name":"app_action","arguments":{"value":4},"task":if extension_view {Value::Null} else {json!({})}},
            }))?).await?.require_success()?.json()?;
            assert_eq!(ordinary["structuredContent"], json!({"received":4}));
            let independent: Value = http.execute(relay(&leftover, &start)?).await?.require_success()?.json()?;
            let id = if extension_view { &independent["taskId"] } else { &independent["task"]["taskId"] }
                .as_str().context("independent task ID")?;
            let complete: Value = if modern && extension_view {
                let state = status_for(&leftover, id, "input_required").await?;
                assert_eq!(state["inputRequests"].as_object().unwrap().len(), 3);
                let state_wire = state.to_string();
                for hidden in ["connection-metadata", "roots/list", "user:pass", "server-native-"] {
                    ensure!(!state_wire.contains(hidden), "{state_wire}");
                }
                let form = app_input_key(&state, "elicitation/create", Some("form"))?;
                let site = app_input_key(&state, "elicitation/create", Some("url"))?;
                let sample = app_input_key(&state, "sampling/createMessage", None)?;
                let wrong = update(id, json!({
                    (form):{"action":"accept","content":{"method":"invalid"}},
                    (site):{"action":"accept"},
                }));
                assert_eq!(http.execute(relay(&leftover, &wrong)?).await?.status, StatusCode::BAD_REQUEST);
                let partial = update(id, json!({
                    (form):{"action":"accept","content":{"method":"Double"},"_meta":{"ignored":1}},
                    "confirm":{"action":"decline"},
                }));
                let ack: Value = http.execute(relay(&leftover, &partial)?).await?.require_success()?.json()?;
                assert_eq!(ack, json!({"resultType":"complete"}));
                tokio::time::timeout(Duration::from_secs(5), async {
                    while !peer.answers.lock().unwrap()["server-native-1"].contains_key("confirm") {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                }).await.context("forward the first partial update before the App answers again")?;
                let after = status_for(&leftover, id, "input_required").await?;
                assert_eq!(after["inputRequests"].as_object().unwrap().len(), 2);
                assert_eq!(app_input_key(&after, "elicitation/create", Some("url"))?, site);
                assert_eq!(app_input_key(&after, "sampling/createMessage", None)?, sample);
                let remaining = update(id, json!({
                    (form):{"action":"decline"},
                    (site):{"action":"accept"},
                    (sample):{"model":"app-model","role":"assistant","content":{"type":"text","text":"App answer"}},
                }));
                http.execute(relay(&leftover, &remaining)?).await?.require_success()?;
                status_for(&leftover, id, "completed").await?["result"].clone()
            } else if extension_view {
                let state = status_for(&leftover, id, "input_required").await?;
                let form = app_input_key(&state, "elicitation/create", Some("form"))?;
                ensure!(!state.to_string().contains("server-native-1"), "{state}");
                let answer = update(id, json!({(form):{"action":"accept","content":{"method":"Double"}}}));
                http.execute(relay(&leftover, &answer)?).await?.require_success()?;
                status_for(&leftover, id, "completed").await?["result"].clone()
            } else {
                let state = status_for(&leftover, id, "input_required").await?;
                ensure!(state.get("inputRequests").is_none(), "{state}");
                let read = json!({"method":"tasks/result","params":{"taskId":id}});
                let read_result = async {
                    http.execute(relay(&leftover, &read)?).await?.require_success()?.json().map_err(Into::into)
                };
                let respond = async {
                    let calls = format!("{leftover}/calls");
                    let mut next = calls.clone();
                    loop {
                        let state: Value = http.execute(request(Method::GET, &next)?).await?.require_success()?.json()?;
                        let callback = &state["calls"][0];
                        if let Some(call_id) = callback["id"].as_str() {
                            assert_eq!(callback["method"], "elicitation/create");
                            assert_eq!(callback["params"]["requestedSchema"]["properties"]["method"]["enum"], json!(["Double","Triple"]));
                            assert_eq!(callback["params"]["_meta"]["io.modelcontextprotocol/related-task"]["taskId"], id);
                            http.execute(request(Method::POST, &format!("{calls}/{call_id}"))?.json(&json!({
                                "status":"result", "result":{"action":"accept","content":{"method":"Double"}},
                            }))).await?.require_success()?;
                            break Ok::<_, anyhow::Error>(());
                        }
                        next = format!("{calls}?after={}", state["revision"]);
                    }
                };
                let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
                    tokio::try_join!(read_result, respond)
                }).await??;
                result
            };
            assert_eq!(complete["structuredContent"], json!({"doubled":14}));
            assert_eq!(complete["_meta"]["io.modelcontextprotocol/related-task"]["taskId"], id);
            let cancel = json!({"method":"tasks/cancel","params":{"taskId":id}});
            let terminal: Value = http.execute(relay(&leftover, &cancel)?).await?.require_success()?.json()?;
            if extension_view {
                assert_eq!(terminal, json!({"resultType":"complete"}));
            } else {
                assert_eq!(terminal["status"], "completed");
                let read = json!({"method":"tasks/result","params":{"taskId":id}});
                let replayed: Value = http.execute(relay(&leftover, &read)?).await?.require_success()?.json()?;
                assert_eq!(replayed, complete);
            }

            peer.release.store(extension_view, Ordering::SeqCst);
            let closing: Value = http.execute(relay(&leftover, &start)?).await?.require_success()?.json()?;
            if extension_view {
                status_for(&leftover, closing["taskId"].as_str().context("closing task ID")?, "input_required").await?;
            }
            http.execute(request(Method::DELETE, &leftover)?).await?.require_success()?;
            tokio::time::timeout(Duration::from_secs(8), peer.cancelled.notified()).await?;
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_app_tools_are_live_session_tools_and_disappear_when_the_view_closes() -> TestResult {
    view_fixture()
        .run(|case| async move {
            let owner = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            allow_permissions(&owner).await?;
            allow_permissions(&other).await?;
            let rendered = owner.call_tool("mcp.tools.call", json!({"server":SERVER,"name":"render"})).await?;
            rendered.assert_succeeded()?;
            let model = case.model();
            let http = case.http().await;
            let part = rendered.part.part_id;
            let session = owner.id();
            let path = format!("/api/v1/sessions/{session}/parts/{part}/mcp-app");
            let document: Value = http.get(&path).await?.require_success()?.json()?;
            let revision = document["revision"].as_str().context("App binding revision")?;
            let request = |method, path: &str| {
                http.request(method, path).map(|request| request.header("if-match", format!("\"{revision}\"")))
            };
            let opened: Value = http.execute(request(Method::POST, &format!("{path}/views"))?
                .json(&publication("double"))).await?.require_success()?.json()?;
            let view = opened["view"].as_str().context("App view identity")?;
            let view_path = format!("{path}/views/{view}");
            let calls = format!("{view_path}/calls");
            let foreign = format!("/api/v1/sessions/{}/parts/{part}/mcp-app/views/{view}/calls", other.id());
            assert_eq!(http.execute(request(Method::GET, &foreign)?).await?.status, StatusCode::CONFLICT);
            ensure!(app_tools(&model, &owner, "no-browser").await?.is_empty());

            let idle: Value = http.execute(request(Method::GET, &calls)?).await?.require_success()?.json()?;
            assert_eq!(idle["calls"], json!([]));
            let names = app_tools(&model, &owner, "open-view").await?;
            assert_eq!(names.len(), 1);
            let tool = &names[0];
            ensure!(tool.contains("__double__"));
            ensure!(app_tools(&model, &other, "other-session").await?.is_empty());

            let mut permission = serde_json::to_value(owner.snapshot().await?.execution.selected_permission)?;
            permission["tools"]["names"]["agena.mcp.apps.call"] = json!("ask");
            owner.client().set_session_permission(session, serde_json::from_value(permission)?).await?;
            let running = owner.start_tool(tool, json!({"value":7})).await?;
            let approval = running.wait_permission().await?;
            approval.allow_once().await?;
            let pending: Value = http.execute(request(Method::GET, &format!("{calls}?after={}", idle["revision"]))?)
                .await?.require_success()?.json()?;
            let call = &pending["calls"][0];
            assert_eq!(call["name"], "double");
            assert_eq!(call["arguments"], json!({"value":7}));
            let reply = format!("{calls}/{}", call["id"].as_str().context("App call identity")?);
            let response = json!({"status":"result","result":{
                "content":[{"type":"text","text":"Browser calculated fourteen"}],
                "structuredContent":{"doubled":14},
            }});
            for _ in 0..2 {
                http.execute(request(Method::POST, &reply)?.json(&response)).await?.require_success()?;
            }
            let result = running.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_text_contains("Browser calculated fourteen")?;
            assert_eq!(result.data()?["structured_content"], json!({"doubled":14}));

            allow_permissions(&owner).await?;
            owner.call_tool(tool, json!({"value":"bad input"})).await?.assert_failed()?;
            let idle: Value = http.execute(request(Method::GET, &calls)?).await?.require_success()?.json()?;
            let running = owner.start_tool(tool, json!({"value":1})).await?;
            let invalid: Value = http.execute(request(Method::GET, &format!("{calls}?after={}", idle["revision"]))?)
                .await?.require_success()?.json()?;
            let call = invalid["calls"][0]["id"].as_str().context("Invalid-result tool identity")?;
            http.execute(request(Method::POST, &format!("{calls}/{call}"))?.json(&json!({
                "status":"result", "result":{"content":[], "structuredContent":{"doubled":"incorrect"}},
            }))).await?.require_success()?;
            running.wait_settled().await?.assert_failed()?;

            http.execute(request(Method::PUT, &view_path)?.json(&publication("replacement")))
                .await?.require_success()?;
            let names = app_tools(&model, &owner, "updated-view").await?;
            assert_eq!(names.len(), 1);
            ensure!(names[0].contains("__replacement__"));
            owner.call_tool(tool, json!({"value":7})).await?.assert_failed()?;
            let idle: Value = http.execute(request(Method::GET, &calls)?).await?.require_success()?.json()?;
            let running = owner.start_tool(&names[0], json!({"value":9})).await?;
            let pending: Value = http.execute(request(Method::GET, &format!("{calls}?after={}", idle["revision"]))?)
                .await?.require_success()?.json()?;
            assert_eq!(pending["calls"][0]["name"], "replacement");
            http.execute(request(Method::DELETE, &view_path)?).await?.require_success()?;
            running.wait_settled().await?.assert_failed()?;
            ensure!(app_tools(&model, &owner, "closed-view").await?.is_empty());
            assert_eq!(http.execute(request(Method::GET, &calls)?).await?.status, StatusCode::CONFLICT);
            let opened: Value = http.execute(request(Method::POST, &format!("{path}/views"))?
                .json(&publication("double"))).await?.require_success()?.json()?;
            let view = opened["view"].as_str().context("Reopened App view identity")?;
            let calls = format!("{path}/views/{view}/calls");
            http.execute(request(Method::GET, &calls)?).await?.require_success()?;
            owner.client().clear_session(session, true).await?;
            assert_eq!(http.execute(request(Method::GET, &calls)?).await?.status, StatusCode::CONFLICT);
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn app_view_tasks_use_the_owning_view_and_preserve_the_final_tool_result() -> TestResult {
    view_fixture()
        .run(|case| async move {
            let owner = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            allow_permissions(&owner).await?;
            allow_permissions(&other).await?;
            let rendered = owner
                .call_tool("mcp.tools.call", json!({"server":SERVER,"name":"render"}))
                .await?;
            rendered.assert_succeeded()?;
            let model = case.model();
            let http = case.http().await;
            let part = rendered.part.part_id;
            let session = owner.id();
            let path = format!("/api/v1/sessions/{session}/parts/{part}/mcp-app");
            let document: Value = http.get(&path).await?.require_success()?.json()?;
            let revision = document["revision"]
                .as_str()
                .context("App binding revision")?;
            let request = |method, path: &str| {
                http.request(method, path)
                    .map(|request| request.header("if-match", format!("\"{revision}\"")))
            };
            let from_server: Value = http
                .execute(request(Method::POST, &path)?.json(&json!({
                    "method": "tools/call",
                    "params": {"name":"app_task"},
                })))
                .await?
                .require_success()?
                .json()?;
            assert_eq!(
                from_server["content"][0]["text"],
                "Originating MCP task completed"
            );
            let next_request = async |calls: &str| {
                tokio::time::timeout(Duration::from_secs(12), async {
                    let mut poll = calls.to_owned();
                    loop {
                        let snapshot: Value = http
                            .execute(request(Method::GET, &poll)?)
                            .await?
                            .require_success()?
                            .json()?;
                        if let Some(call) =
                            snapshot["calls"].as_array().and_then(|calls| calls.first())
                        {
                            return Ok::<_, anyhow::Error>(call.clone());
                        }
                        poll = format!("{calls}?after={}", snapshot["revision"]);
                    }
                })
                .await?
            };
            let answer = async |calls: &str, call: &Value, result: Value| -> anyhow::Result<()> {
                let id = call["id"].as_str().context("View request ID")?;
                http.execute(
                    request(Method::POST, &format!("{calls}/{id}"))?
                        .json(&json!({"status":"result","result":result})),
                )
                .await?
                .require_success()?;
                Ok(())
            };
            let open_view = async |publication: Value| {
                let opened: Value = http
                    .execute(request(Method::POST, &format!("{path}/views"))?.json(&publication))
                    .await?
                    .require_success()?
                    .json()?;
                let view = opened["view"].as_str().context("View ID")?;
                let view_path = format!("{path}/views/{view}");
                http.execute(request(Method::GET, &format!("{view_path}/calls"))?)
                    .await?
                    .require_success()?;
                Ok::<_, anyhow::Error>(view_path)
            };

            let direct = open_view(publication("continue_double")).await?;
            let calls = format!("{direct}/calls");
            let names = app_tools(&model, &owner, "direct-continuation").await?;
            ensure!(names.len() == 1, "{names:?}");
            let tool = &names[0];
            let arguments = json!({"value":7});
            let run = owner.start_tool(tool, arguments.clone()).await?;
            let start = next_request(&calls).await?;
            assert_eq!(start["name"], "continue_double");
            assert_eq!(start["arguments"], arguments);
            ensure!(start.get("task").is_none() && start.get("requestState").is_none(), "{start}");
            ensure!(start.get("inputResponses").is_none(), "{start}");
            let opaque_state = " round 2 / α\n ";
            answer(&calls, &start, json!({
                "resultType":"input_required", "requestState":opaque_state,
            })).await?;
            let retry = next_request(&calls).await?;
            assert_ne!(retry["id"], start["id"]);
            assert_eq!(retry["name"], start["name"]);
            assert_eq!(retry["arguments"], arguments);
            assert_eq!(retry["requestState"], opaque_state);
            ensure!(retry.get("inputResponses").is_none(), "{retry}");

            let input = json!({
                "resultType":"input_required",
                "inputRequests":task_input("unused")["inputRequests"],
            });
            answer(&calls, &retry, input.clone()).await?;
            let pending = run.wait_user_input().await?;
            assert_eq!(pending.session_id(), session);
            let dialog = pending.request.request.as_user_input().context("App continuation dialog")?;
            ensure!(dialog.questions[0].question.contains("Choose how the App should calculate."));
            pending.answer(BTreeMap::from([("0".into(), vec!["Double".into()])])).await?;
            let finish = next_request(&calls).await?;
            assert_ne!(finish["id"], start["id"]);
            assert_ne!(finish["id"], retry["id"]);
            assert_eq!(finish["name"], start["name"]);
            assert_eq!(finish["arguments"], arguments);
            ensure!(finish.get("requestState").is_none(), "{finish}");
            assert_eq!(finish["inputResponses"], json!({
                "confirm":{"action":"accept","content":{"method":"Double"}},
            }));
            answer(&calls, &finish, json!({
                "resultType":"complete",
                "content":[{"type":"text","text":"App continuation finished"}],
                "structuredContent":{"doubled":14},
            })).await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_text_contains("App continuation finished")?;
            assert_eq!(result.data()?["structured_content"], json!({"doubled":14}));

            let run = owner.start_tool(tool, arguments.clone()).await?;
            let start = next_request(&calls).await?;
            answer(&calls, &start, json!({"resultType":"input_required","requestState":"pending"})).await?;
            let retry = next_request(&calls).await?;
            answer(&calls, &retry, json!({"resultType":"complete","structuredContent":{"doubled":"invalid"}})).await?;
            run.wait_settled().await?.assert_failed()?;

            let run = owner.start_tool(tool, arguments).await?;
            let start = next_request(&calls).await?;
            answer(&calls, &start, input).await?;
            assert_eq!(run.wait_user_input().await?.session_id(), session);
            http.execute(request(Method::DELETE, &direct)?)
                .await?
                .require_success()?;
            run.wait_settled().await?.assert_failed()?;
            assert!(owner.snapshot().await?.session.state.pending_interactive_requests().is_empty());

            let mut legacy_tools = publication("legacy_double");
            legacy_tools["tools"][0]["execution"] = json!({"taskSupport":"required"});
            let legacy = open_view(legacy_tools).await?;
            let calls = format!("{legacy}/calls");
            let names = app_tools(&model, &owner, "legacy-task").await?;
            ensure!(names.len() == 1, "{names:?}");
            let run = owner.start_tool(&names[0], json!({"value":7})).await?;
            let start = next_request(&calls).await?;
            assert_eq!(start["name"], "legacy_double");
            assert_eq!(start["task"], json!({}));
            answer(
                &calls,
                &start,
                json!({"task":task_info("legacy-task","working")}),
            )
            .await?;

            let poll = next_request(&calls).await?;
            assert_ne!(poll["id"], start["id"]);
            assert_eq!(poll["method"], "tasks/get");
            assert_eq!(poll["taskId"], "legacy-task");
            answer(&calls, &poll, task_info("legacy-task", "input_required")).await?;
            let finish = next_request(&calls).await?;
            assert_eq!(finish["method"], "tasks/result");
            assert_eq!(finish["taskId"], "legacy-task");
            let foreign = format!(
                "/api/v1/sessions/{}/parts/{part}/mcp-app{}",
                other.id(),
                calls.strip_prefix(&path).context("View path")?
            );
            assert_eq!(
                http.execute(request(Method::GET, &foreign)?).await?.status,
                StatusCode::CONFLICT
            );
            answer(
                &calls,
                &finish,
                json!({
                    "content":[{"type":"text","text":"Legacy App task finished"}],
                    "structuredContent":{"doubled":14},
                    "_meta":{"io.modelcontextprotocol/related-task":{"taskId":"legacy-task"}},
                }),
            )
            .await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_text_contains("Legacy App task finished")?;
            assert_eq!(result.data()?["structured_content"], json!({"doubled":14}));
            http.execute(request(Method::DELETE, &legacy)?)
                .await?
                .require_success()?;

            let mut extension_tools = publication("extension_double");
            extension_tools["task_extension"] = json!(true);
            let extension = open_view(extension_tools).await?;
            let calls = format!("{extension}/calls");
            let names = app_tools(&model, &owner, "extension-task").await?;
            ensure!(names.len() == 1, "{names:?}");
            let tool = &names[0];
            let run = owner.start_tool(tool, json!({"value":9})).await?;
            let start = next_request(&calls).await?;
            assert_eq!(start["name"], "extension_double");
            ensure!(start.get("task").is_none(), "{start}");
            answer(&calls, &start, json!({"resultType":"input_required","requestState":"preparing-task"})).await?;
            let retry = next_request(&calls).await?;
            assert_eq!(retry["requestState"], "preparing-task");
            let mut created = task_info("extension-task", "working");
            created["resultType"] = json!("task");
            answer(&calls, &retry, created).await?;
            let poll = next_request(&calls).await?;
            assert_eq!(poll["method"], "tasks/get");
            assert_eq!(poll["taskId"], "extension-task");
            let mut inputs = task_input("extension-task");
            inputs["inputRequests"]["sample"] = json!({
                "method":"sampling/createMessage",
                "params":{
                    "messages":[{"role":"user", "content":{"type":"text", "text":"App sampling prompt"}}],
                    "maxTokens":40,
                }
            });
            model
                .expect(RequestMatch::user_text("App sampling prompt"))
                .check(|request| {
                    assert_eq!(request.body["max_output_tokens"], 40);
                    ensure!(!request.body.to_string().contains("Model-visible MCP result"));
                    Ok(())
                })
                .reply(Reply::text("Approved App sampling output"));
            answer(&calls, &poll, inputs.clone()).await?;
            let pending = run.wait_user_input().await?;
            let dialog = pending.request.request.as_user_input().context("App input dialog")?;
            assert_eq!(pending.session_id(), session);
            ensure!(dialog.title.contains(&format!("App {SERVER}/render")));
            ensure!(dialog.questions[0].question.contains("Choose how the App should calculate."));
            pending.answer(BTreeMap::from([("0".into(), vec!["Double".into()])])).await?;
            for preview in ["App sampling prompt", "Approved App sampling output"] {
                let pending = run.wait_user_input().await?;
                let dialog = pending.request.request.as_user_input().context("App sampling dialog")?;
                ensure!(dialog.questions[0].question.contains(preview));
                pending.answer(BTreeMap::from([("0".into(), vec!["Allow".into()])])).await?;
            }
            let update = next_request(&calls).await?;
            assert_eq!(update["method"], "tasks/update");
            assert_eq!(update["taskId"], "extension-task");
            assert_eq!(update["inputResponses"]["confirm"], json!({
                "action":"accept", "content":{"method":"Double"},
            }));
            assert_eq!(update["inputResponses"]["sample"]["role"], "assistant");
            assert_eq!(update["inputResponses"]["sample"]["content"]["text"], "Approved App sampling output");
            answer(&calls, &update, json!({"resultType":"complete"})).await?;

            let repeated = next_request(&calls).await?;
            assert_eq!(repeated["method"], "tasks/get");
            answer(&calls, &repeated, inputs).await?;
            let poll = next_request(&calls).await?;
            assert_eq!(poll["method"], "tasks/get");
            assert_eq!(poll["taskId"], "extension-task");
            let mut completed = task_info("extension-task", "completed");
            completed["resultType"] = json!("complete");
            completed["result"] = json!({
                "content":[{"type":"text","text":"Extension App task finished"}],
                "structuredContent":{"doubled":18},
            });
            answer(&calls, &poll, completed).await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            assert_eq!(result.data()?["structured_content"], json!({"doubled":18}));

            let run = owner.start_tool(tool, json!({"value":11})).await?;
            let start = next_request(&calls).await?;
            let mut created = task_info("own-task", "working");
            created["resultType"] = json!("task");
            answer(&calls, &start, created).await?;
            let poll = next_request(&calls).await?;
            let mut wrong = task_info("other-task", "working");
            wrong["resultType"] = json!("complete");
            answer(&calls, &poll, wrong).await?;
            let cancel = next_request(&calls).await?;
            assert_eq!(cancel["method"], "tasks/cancel");
            assert_eq!(cancel["taskId"], "own-task");
            answer(&calls, &cancel, json!({"resultType":"complete"})).await?;
            run.wait_settled().await?.assert_failed()?;

            let mut untyped = publication("untyped");
            untyped["tools"][0].as_object_mut().unwrap().remove("outputSchema");
            http.execute(request(Method::PUT, &extension)?.json(&untyped))
                .await?
                .require_success()?;
            let names = app_tools(&model, &owner, "schema-free-task").await?;
            ensure!(names.len() == 1, "{names:?}");
            let run = owner.start_tool(&names[0], json!({"value":11})).await?;
            let start = next_request(&calls).await?;
            answer(&calls, &start, json!({"resultType":"input_required","inputRequests":{}})).await?;
            run.wait_settled().await?.assert_failed()?;

            let run = owner.start_tool(&names[0], json!({"value":11})).await?;
            let start = next_request(&calls).await?;
            let mut created = task_info("closed-during-input", "input_required");
            created["resultType"] = json!("task");
            answer(&calls, &start, created).await?;
            let poll = next_request(&calls).await?;
            assert_eq!(poll["method"], "tasks/get");
            answer(&calls, &poll, task_input("closed-during-input")).await?;
            assert_eq!(run.wait_user_input().await?.session_id(), session);
            http.execute(request(Method::DELETE, &extension)?)
                .await?
                .require_success()?;
            run.wait_settled().await?.assert_failed()?;
            assert!(owner.snapshot().await?.session.state.pending_interactive_requests().is_empty());
            Ok(())
        })
        .await
}
