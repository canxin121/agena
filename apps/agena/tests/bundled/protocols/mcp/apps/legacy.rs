use std::{collections::BTreeSet, convert::Infallible, time::Duration};

use agena_testkit::{FixtureWebSocketUpgrade, TestCase};
use axum::{
    extract::{Path, ws::Message},
    response::{Sse, sse::Event},
    routing::get,
};
use futures_util::{Stream, StreamExt, stream};
use tokio::sync::{Notify, mpsc::{self, Sender}};

use super::*;

#[derive(Default)]
struct LivePeers {
    connected: BTreeSet<String>,
    feeds: BTreeMap<String, Sender<Value>>,
    awaiting_roots: BTreeMap<String, Value>,
}

#[derive(Default)]
struct LegacyFixture {
    app: AppPeer,
    live: Mutex<LivePeers>,
    received: AtomicU64,
    tool_listed: Notify,
}

struct LegacyConnection {
    fixture: Arc<LegacyFixture>,
    id: String,
}

impl LegacyFixture {
    fn connect(self: &Arc<Self>) -> LegacyConnection {
        let n = self.app.next_session.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("peer-{n}");
        self.live.lock().unwrap().connected.insert(id.clone());
        LegacyConnection {
            fixture: self.clone(),
            id,
        }
    }

    fn handle(&self, session: &str, request: &Value) -> Option<Value> {
        self.received.fetch_add(1, Ordering::Relaxed);
        let owns_app = self.app.app_session.lock().unwrap().as_deref() == Some(session);
        if owns_app
            && request["method"] == "tools/call"
            && request["params"]["name"] == "app_action"
        {
            self.live
                .lock()
                .unwrap()
                .awaiting_roots
                .insert(session.to_owned(), request.clone());
            return Some(json!({"jsonrpc":"2.0","id":"fixture-roots","method":"roots/list"}));
        }
        if request["id"] == "fixture-roots" {
            let original = self.live.lock().unwrap().awaiting_roots.remove(session)?;
            let mut result = app_reply(&self.app, &original, Some(session), "2024-11-05")?;
            result["result"]["structuredContent"]["roots_callback"] =
                json!(request["result"]["roots"].is_array());
            result["result"]["structuredContent"]["roots"] = request["result"]["roots"].clone();
            return Some(result);
        }
        let response = app_reply(&self.app, request, Some(session), "2024-11-05");
        if owns_app && request["method"] == "tools/list" {
            self.tool_listed.notify_waiters();
        }
        response
    }

    async fn notify(&self, session: &str, method: &str) -> TestResult {
        let send = self.live.lock().unwrap().feeds.get(session).cloned()
            .context("the App peer had no notification stream")?;
        send.send(json!({"jsonrpc":"2.0","method":method})).await?;
        Ok(())
    }

    fn current_app(&self) -> anyhow::Result<String> {
        self.app
            .app_session
            .lock()
            .unwrap()
            .clone()
            .context("the tool did not render on a peer")
    }

    async fn wait_closed(&self, id: &str) -> TestResult {
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.live.lock().unwrap().connected.contains(id) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .with_context(|| format!("App peer {id} remained open"))?;
        Ok(())
    }
}

impl Drop for LegacyConnection {
    fn drop(&mut self) {
        let mut live = self.fixture.live.lock().unwrap();
        live.connected.remove(&self.id);
        live.feeds.remove(&self.id);
        live.awaiting_roots.remove(&self.id);
    }
}

async fn open_sse(
    State(peer): State<Arc<LegacyFixture>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let connection = peer.connect();
    let (send, receive) = mpsc::channel::<Value>(16);
    peer.live
        .lock()
        .unwrap()
        .feeds
        .insert(connection.id.clone(), send);
    let endpoint = Event::default()
        .event("endpoint")
        .data(format!("/sse/{}", connection.id));
    let messages = stream::unfold(
        (receive, connection),
        |(mut receive, connection)| async move {
            receive.recv().await.map(|message| {
                let event = Event::default().event("message").data(message.to_string());
                (Ok(event), (receive, connection))
            })
        },
    );
    Sse::new(stream::once(async move { Ok(endpoint) }).chain(messages))
}

async fn sse_post(
    Path(id): Path<String>,
    State(peer): State<Arc<LegacyFixture>>,
    Json(request): Json<Value>,
) -> StatusCode {
    let send = peer.live.lock().unwrap().feeds.get(&id).cloned();
    let Some(send) = send else {
        return StatusCode::NOT_FOUND;
    };
    if let Some(response) = peer.handle(&id, &request)
        && send.send(response).await.is_err()
    {
        return StatusCode::GONE;
    }
    StatusCode::ACCEPTED
}

async fn open_socket(
    State(peer): State<Arc<LegacyFixture>>,
    websocket: FixtureWebSocketUpgrade,
) -> Response {
    websocket
        .configure(|upgrade| upgrade.protocols(["mcp"]))
        .on_upgrade(move |mut socket, cancel| async move {
            let connection = peer.connect();
            let (send, mut receive) = mpsc::channel::<Value>(16);
            peer.live.lock().unwrap().feeds.insert(connection.id.clone(), send);
            loop {
                let frame = tokio::select! {
                    () = cancel.cancelled() => break,
                    value = receive.recv() => {
                        let Some(value) = value else { break };
                        if socket.send(Message::Text(value.to_string().into())).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    frame = socket.recv() => frame,
                };
                let Some(Ok(frame)) = frame else { break };
                let Message::Text(text) = frame else { continue };
                let request: Value = serde_json::from_str(&text)?;
                if let Some(response) = peer.handle(&connection.id, &request)
                    && socket
                        .send(Message::Text(response.to_string().into()))
                        .await
                        .is_err()
                {
                    break;
                }
            }
            Ok(())
        })
}

async fn install_claude_peer(case: &TestCase, transport: &str, endpoint: &str) -> TestResult {
    let package = case.paths().root.join("claude-mcp-app");
    tokio::fs::create_dir_all(package.join(".claude-plugin")).await?;
    tokio::fs::write(
        package.join(".claude-plugin/plugin.json"),
        br#"{"name":"mcp-app-peer"}"#,
    )
    .await?;
    tokio::fs::write(
        package.join(".mcp.json"),
        serde_json::to_vec(&json!({
            "mcpServers": {"app-peer": {"type": transport, "url": endpoint}}
        }))?,
    )
    .await?;
    case.run_cli([
        "plugin",
        "install",
        package.to_str().context("fixture path")?,
    ])
    .await?
    .require_success()?;
    Ok(())
}

async fn stateful_app_on_legacy_transport(transport: &'static str) -> TestResult {
    let peer = Arc::new(LegacyFixture {
        app: AppPeer { sends_catalog_notifications: true, ..Default::default() },
        ..Default::default()
    });
    let setup = peer.clone();
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .builtin_plugin("agena.mcp", json!({"runtime":{"reconnect":{"enabled":false}}}))
        .expect_plugin("claude.mcp-app-peer")
        .before_start(move |case| async move {
            let server = case.serve_http_on(native_fixture_address()?, Router::new()
                .route("/sse", get(open_sse))
                .route("/sse/{id}", post(sse_post))
                .route("/socket", get(open_socket))
                .with_state(setup)).await?;
            let endpoint = match transport {
                "sse" => server.url("/sse"),
                "ws" => server.url("/socket").replacen("http://", "ws://", 1),
                _ => unreachable!(),
            };
            install_claude_peer(&case, transport, &endpoint).await
        })
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let search = user.call_tool("mcp.tools.search", json!({"query":"render"})).await?;
            search.assert_succeeded()?;
            let data = search.data()?;
            let server = data["results"].as_array().context("MCP catalog")?.iter()
                .find(|tool| tool["name"] == "render").and_then(|tool| tool["server"].as_str())
                .context("render tool in imported catalog")?;
            let render = || user.call_tool("mcp.tools.call", json!({"server":server,"name":"render"}));
            let rendered = render().await?;
            rendered.assert_succeeded()?;
            let origin = peer.current_app()?;
            let session = user.id();
            let part = rendered.part.part_id;
            let path = format!("/api/v1/sessions/{session}/parts/{part}/mcp-app");
            let http = case.http().await;
            let document: Value = http.get(&path).await?.require_success()?.json()?;
            assert_eq!(document["html"], "<main>Fixture MCP App</main>");
            let revision = document["revision"].as_str().context("MCP App revision")?;
            let catalog_revision = document["tool_catalog_revision"].as_str().context("MCP App tool catalog revision")?;
            let resource_revision = document["resource_catalog_revision"].as_str()
                .context("MCP App resource catalog revision")?;
            let changes = |catalog: &str, after: &str| {
                http.request(Method::GET, &format!("{path}/{catalog}/changes?after={after}"))
                    .map(|request| request.header("if-match", format!("\"{revision}\"")))
            };
            let extra = case.paths().root.join("App working directory");
            tokio::fs::create_dir_all(&extra).await?;
            let registered = user.client().register_working_directory(session, extra).await?;
            assert!(registered.added);
            let root_uri = url::Url::from_directory_path(&registered.directory)
                .ok().context("registered MCP root URI")?;
            let invoke = |body: Value| http.request(Method::POST, &path).map(|request| {
                request.header("if-match", format!("\"{revision}\"")).json(&body)
            });
            let response: Value = http.execute(invoke(json!({"method":"resources/read","params":{"uri":"fixture://data"}}))?)
                .await?.require_success()?.json()?;
            assert_eq!(response["contents"][0]["text"], "Fixture data");
            let action = json!({"method":"tools/call","params":{"name":"app_action","arguments":{"value":7}}});
            let result: Value = http.execute(invoke(action.clone())?).await?.require_success()?.json()?;
            let data = &result["structuredContent"];
            assert_eq!(data["received"], 7);
            assert_eq!(data["roots_callback"], true);
            assert!(data["roots"].as_array().context("MCP root list")?.iter()
                .any(|root| root["uri"] == root_uri.as_str()));

            let initial_list = peer.tool_listed.notified();
            tokio::pin!(initial_list);
            initial_list.as_mut().enable();
            let update = async {
                initial_list.await;
                // A passive watch must release the original peer to other
                // requests while it waits for a server notification.
                let resource = json!({"method":"resources/read","params":{"uri":"fixture://data"}});
                http.execute(invoke(resource)?).await?.require_success()?;
                peer.app.catalog_version.store(2, Ordering::Relaxed);
                peer.notify(&origin, "notifications/tools/list_changed").await
            };
            let read = async {
                Ok::<_, anyhow::Error>(http.execute(changes("tools", catalog_revision)?).await?.require_success()?.json::<Value>()?)
            };
            let (updated, ()) = tokio::time::timeout(Duration::from_secs(8), async {
                tokio::try_join!(read, update)
            }).await.context("tool change did not reach the App's original peer watcher")??;
            assert_ne!(updated["revision"], catalog_revision);
            assert_eq!(updated["retry_after_ms"], 250);

            let update_revision = updated["revision"].as_str().context("updated tool catalog revision")?;
            let listed = peer.tool_listed.notified();
            tokio::pin!(listed);
            listed.as_mut().enable();
            let revoke = async {
                listed.await;
                let mut permission = serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
                permission["tools"]["names"]["mcp.tools.search"] = json!("deny");
                permission["tools"]["names"]["agena.mcp.tools.search"] = json!("deny");
                user.client().set_session_permission(session, serde_json::from_value(permission)?).await?;
                peer.notify(&origin, "notifications/tools/list_changed").await
            };
            let observe = async {
                Ok::<_, anyhow::Error>(http.execute(changes("tools", update_revision)?).await?)
            };
            let (denied, ()) = tokio::time::timeout(Duration::from_secs(8), async {
                tokio::try_join!(observe, revoke)
            }).await.context("catalog authorization was not rechecked after a notification")??;
            assert_eq!(denied.status, StatusCode::CONFLICT);
            allow_permissions(&user).await?;

            let listing = json!({"method":"resources/list","params":{}});
            let listed = peer.tool_listed.notified();
            tokio::pin!(listed);
            listed.as_mut().enable();
            let update = async {
                listed.await;
                let before: Value = http.execute(invoke(listing.clone())?).await?
                    .require_success()?.json()?;
                assert_eq!(before["resources"].as_array().context("App resources")?.len(), 1);
                peer.app.resource_catalog_version.store(1, Ordering::Relaxed);
                peer.notify(&origin, "notifications/resources/list_changed").await
            };
            let read = async {
                let reply = http.execute(changes("resources", resource_revision)?).await?;
                Ok::<_, anyhow::Error>(reply.require_success()?.json::<Value>()?)
            };
            let (updated, ()) = tokio::time::timeout(Duration::from_secs(8), async {
                tokio::try_join!(read, update)
            }).await.context("resource change did not wake the App watcher")??;
            assert_ne!(updated["revision"], resource_revision);
            let resource_revision = updated["revision"].as_str()
                .context("updated MCP App resource catalog revision")?;
            let refreshed: Value = http.execute(invoke(listing.clone())?).await?
                .require_success()?.json()?;
            let available = refreshed["resources"].as_array().context("App resources")?;
            assert_eq!(available.len(), 2);
            assert!(available.iter().any(|resource| resource["uri"] == "fixture://report"));

            let listed = peer.tool_listed.notified();
            tokio::pin!(listed);
            listed.as_mut().enable();
            let revoke = async {
                listed.await;
                http.execute(invoke(listing)?).await?.require_success()?;
                let mut permission = serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
                permission["tools"]["names"]["mcp.resources.list"] = json!("deny");
                permission["tools"]["names"]["agena.mcp.resources.list"] = json!("deny");
                user.client().set_session_permission(session, serde_json::from_value(permission)?).await?;
                peer.notify(&origin, "notifications/resources/list_changed").await
            };
            let observe = async {
                Ok::<_, anyhow::Error>(http.execute(changes("resources", resource_revision)?).await?)
            };
            let (denied, ()) = tokio::time::timeout(Duration::from_secs(8), async {
                tokio::try_join!(observe, revoke)
            }).await.context("resource catalog authorization was not rechecked")??;
            assert_eq!(denied.status, StatusCode::CONFLICT);
            allow_permissions(&user).await?;

            let before_denial = peer.received.load(Ordering::Relaxed);
            let mut policy = serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
            policy["network"] = json!({"default":"deny"});
            user.client().set_session_permission(session, serde_json::from_value(policy)?).await?;
            assert_eq!(http.execute(invoke(action)?).await?.status, StatusCode::CONFLICT);
            assert_eq!(peer.received.load(Ordering::Relaxed), before_denial);
            peer.wait_closed(&origin).await?;
            allow_permissions(&user).await?;
            let before_reopen = peer.app.next_session.load(Ordering::Relaxed);
            assert_eq!(http.get(&path).await?.status, StatusCode::CONFLICT);
            assert_eq!(peer.app.next_session.load(Ordering::Relaxed), before_reopen);

            let rendered = render().await?;
            rendered.assert_succeeded()?;
            let before_reload = peer.current_app()?;
            assert_ne!(before_reload, origin);
            let path = format!("/api/v1/sessions/{session}/parts/{}/mcp-app", rendered.part.part_id);
            http.get(&path).await?.require_success()?;

            case.settings().reload_and_wait().await?;
            peer.wait_closed(&before_reload).await?;
            let before_reopen = peer.app.next_session.load(Ordering::Relaxed);
            assert_eq!(http.get(&path).await?.status, StatusCode::CONFLICT);
            assert_eq!(peer.app.next_session.load(Ordering::Relaxed), before_reopen);

            let rendered = render().await?;
            rendered.assert_succeeded()?;
            let next = peer.current_app()?;
            assert_ne!(next, before_reload);
            let new_path = format!("/api/v1/sessions/{session}/parts/{}/mcp-app", rendered.part.part_id);
            http.get(&new_path).await?.require_success()?;
            user.client().clear_session(session, true).await?;
            peer.wait_closed(&next).await?;
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_sse_app_retains_its_peer_and_authorizes_each_operation() -> TestResult {
    stateful_app_on_legacy_transport("sse").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_app_retains_its_peer_and_authorizes_each_operation() -> TestResult {
    stateful_app_on_legacy_transport("ws").await
}
