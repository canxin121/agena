use std::{convert::Infallible, time::Duration};

use axum::response::{Sse, sse::Event};
use futures_util::{StreamExt, stream};
use tokio::sync::mpsc;

use super::*;

const MODERN_SESSION: &str = "stateless-app";
const FILTER: &str = "resourcesListChanged";
const SUBSCRIPTION_META: &str = "io.modelcontextprotocol/subscriptionId";
const HEADER: &str = "x-app-fixture";

struct StreamablePeer {
    app: AppPeer,
    modern: bool,
    opened: mpsc::Sender<Listener>,
}

struct Listener {
    send: mpsc::Sender<Value>,
    subscription: Option<Value>,
}

impl Listener {
    async fn notify(&self) -> TestResult {
        let mut notification =
            json!({"jsonrpc": "2.0", "method": "notifications/resources/list_changed"});
        if let Some(subscription) = &self.subscription {
            notification["params"] = json!({"_meta": {(SUBSCRIPTION_META): subscription}});
        }
        self.send.send(notification).await?;
        Ok(())
    }
}

fn receive_stream(peer: &StreamablePeer, subscription: Option<Value>) -> Response {
    let ack = subscription.as_ref().map(|id| {
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/subscriptions/acknowledged",
            "params": {
                "notifications": {(FILTER): true},
                "_meta": {(SUBSCRIPTION_META): id},
            },
        })
    });
    let (send, receive) = mpsc::channel::<Value>(4);
    if peer
        .opened
        .try_send(Listener { send, subscription })
        .is_err()
    {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let first = match ack {
        Some(value) => Event::default().data(value.to_string()),
        None => Event::default().comment("ready"),
    };
    let messages = stream::unfold(receive, |mut receive| async {
        receive
            .recv()
            .await
            .map(|message| (Ok(Event::default().data(message.to_string())), receive))
    });
    Sse::new(stream::once(async move { Ok::<_, Infallible>(first) }).chain(messages))
        .into_response()
}

async fn receive_legacy(State(peer): State<Arc<StreamablePeer>>, headers: HeaderMap) -> Response {
    let owner = peer.app.app_session.lock().unwrap();
    let session = headers
        .get("mcp-session-id")
        .and_then(|header| header.to_str().ok());
    if peer.modern || session.is_none() || session != owner.as_deref() {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    if headers.get(HEADER).is_none_or(|value| value != "catalog")
        || headers
            .get("mcp-protocol-version")
            .is_none_or(|value| value != "2025-11-25")
        || headers
            .get("accept")
            .is_none_or(|value| value != "text/event-stream")
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    receive_stream(&peer, None)
}

async fn handle(
    State(peer): State<Arc<StreamablePeer>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    if headers.get(HEADER).is_none_or(|value| value != "catalog") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if !peer.modern {
        // The legacy fixture negotiates the original MCP session and handles
        // resource and tool RPC just like the other Streamable HTTP App test.
        return legacy_reply(&peer.app, headers, request);
    }
    let id = &request["id"];
    if request["method"] == "server/discover" {
        return Json(json!({"jsonrpc":"2.0","id":id,"result":{
            "resultType": "complete",
            "supportedVersions": ["2026-07-28"],
            "capabilities": {"tools": {"listChanged": true}, "resources": {"listChanged": true}},
            "ttlMs": 0, "cacheScope": "private",
            "_meta": {"io.modelcontextprotocol/serverInfo": {"name":"streamable-fixture","version":"1"}}
        }}))
        .into_response();
    }
    if request["method"] == "subscriptions/listen"
        && request["params"]["notifications"] == json!({(FILTER): true})
    {
        if headers.contains_key("mcp-session-id") {
            return StatusCode::BAD_REQUEST.into_response();
        }
        return receive_stream(&peer, Some(id.clone()));
    }
    match app_reply(&peer.app, &request, Some(MODERN_SESSION), "2026-07-28") {
        Some(mut reply) => {
            if let Some(result) = reply["result"].as_object_mut() {
                result.insert("resultType".into(), json!("complete"));
            }
            Json(reply).into_response()
        }
        None => StatusCode::ACCEPTED.into_response(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streamable_mcp_app_receives_legacy_get_resource_catalog_notifications() -> TestResult {
    resource_notifications(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streamable_mcp_app_receives_modern_subscription_resource_catalog_notifications()
-> TestResult {
    resource_notifications(true).await
}

async fn resource_notifications(modern: bool) -> TestResult {
    let (opened, mut listeners) = mpsc::channel(4);
    let peer = Arc::new(StreamablePeer {
        app: AppPeer {
            sends_catalog_notifications: true,
            ..Default::default()
        },
        modern,
        opened,
    });
    let setup = peer.clone();
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .case_timeout(Duration::from_secs(45))
        .expect_plugin("agena.mcp")
        .before_start(move |case| async move {
            let endpoint = case.serve_http_on(
                native_fixture_address()?,
                Router::new().route("/mcp", post(handle).get(receive_legacy)).with_state(setup),
            ).await?.url("/mcp");
            let mut config: Value = serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["agena.mcp"] = json!({
                "enabled": true, "package": {"kind": "static"},
                "settings": {
                    "runtime": {"reconnect": {"enabled": false}},
                    "servers": {(SERVER): {
                        "transport": "http", "endpoint": {"url": endpoint, "headers": {(HEADER): "catalog"}},
                    }},
                },
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .run(move |case| async move {
            super::super::super::cordis::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let rendered = user.call_tool("mcp.tools.call", json!({"server": SERVER, "name": "render"})).await?;
            rendered.assert_succeeded()?;
            let user = if modern {
                case.crash().await?;
                case.restart().await?;
                super::super::super::cordis::wait_for_startup_catalog(&case).await?;
                case.user().attach_session(user.id()).await?
            } else {
                user
            };
            let path = format!("/api/v1/sessions/{}/parts/{}/mcp-app", user.id(), rendered.part.part_id);
            let http = case.http().await;
            let document: Value = http.get(&path).await?
                .require_success().context("open the rendered MCP App")?.json()?;
            let revision = document["revision"].as_str().context("App revision")?;
            let catalog = document["resource_catalog_revision"].as_str().context("resource revision")?;
            ensure!(listeners.try_recv().is_err(), "the server opened a stream before a watch");
            let list = || http.request(Method::POST, &path).map(|request| {
                request.header("if-match", format!("\"{revision}\""))
                    .json(&json!({"method":"resources/list","params":{}}))
            });
            let changes = |after: &str| http.request(Method::GET, &format!("{path}/resources/changes?after={after}"))
                .map(|request| request.header("if-match", format!("\"{revision}\"")));
            let observe = async { Ok::<_, anyhow::Error>(http.execute(changes(catalog)?).await?) };
            let notify = async {
                let listener = listeners.recv().await.context("resource watch did not open a stream")?;
                // Notification-only invalidation proves the result came from
                // the MCP event, even when the resource descriptor hash matches.
                let concurrent: Value = http.execute(list()?).await?
                    .require_success().context("list while the notification stream is open")?.json()?;
                ensure!(concurrent["resources"].as_array().is_some_and(|items| items.len() == 1));
                listener.notify().await?;
                listener.send.closed().await;
                Ok(())
            };
            let (updated, ()) = tokio::time::timeout(Duration::from_secs(10), async {
                tokio::try_join!(observe, notify)
            }).await.context("App notification or stream cleanup timed out")??;
            let updated: Value = updated.require_success().context("catalog invalidation response")?.json()?;
            assert_ne!(updated["revision"], catalog);
            assert_eq!(updated["retry_after_ms"], 250);
            let catalog = updated["revision"].as_str().context("updated resource revision")?;

            let observe = async { Ok::<_, anyhow::Error>(http.execute(changes(catalog)?).await?) };
            let revoke = async {
                let listener = listeners.recv().await.context("resource watch did not reopen")?;
                let mut policy = serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
                policy["tools"]["names"]["agena.mcp.resources.list"] = json!("deny");
                policy["tools"]["names"]["mcp.resources.list"] = json!("deny");
                user.client().set_session_permission(user.id(), serde_json::from_value(policy)?).await?;
                peer.app.resource_catalog_version.store(1, Ordering::Relaxed);
                listener.notify().await?;
                listener.send.closed().await;
                Ok(())
            };
            let (denied, ()) = tokio::time::timeout(Duration::from_secs(10), async {
                tokio::try_join!(observe, revoke)
            }).await.context("resource watch did not recheck revoked permission")??;
            assert_eq!(denied.status, StatusCode::CONFLICT);
            assert_eq!(http.execute(list()?).await?.status, StatusCode::CONFLICT);
            Ok(())
        }).await
}
