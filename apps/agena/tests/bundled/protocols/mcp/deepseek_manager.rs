use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::{sync::Arc, time::Duration};

use agena_plugin_sdk::CommandStatus;
use agena_testkit::{TestCase, TestResult, UserSession};
use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::super::cordis::wait_for_startup_catalog;
use super::{Peer, RpcRequest, allow_permissions, fixture_without_memory, native_fixture_address};

const PLUGIN: &str = "deepseek.fixture-mcp-manager";
const OTHER: &str = "deepseek.fixture-mcp-other";
const CONSOLE: &str = "deepseek.fixture-mcp-console";
const LOCAL_SHARED: &str = "plugin:deepseek.fixture-mcp-manager:enabled";
const ENABLED_SERVER: &str = "agent-plugin-efcbe95f13272b20d389e6a4-enabled";
const FOREIGN_SHARED: &str = "plugin:deepseek.fixture-mcp-other:enabled";
const OWNER_ENV: &str = "AGENA_TEST_DEEPSEEK_MCP_OWNER_TAG";
const REMOTE_SESSION: &str = "deepseek-cleanup-fixture";

#[derive(Default)]
struct ManagerPeer {
    rpc: Peer,
    stateful: bool,
    deletes: AtomicUsize,
    initialized: Notify,
    enabled_failures: AtomicUsize,
    hold_final_page: AtomicBool,
    final_page: Notify,
    release_final_page: Notify,
}

async fn rpc(
    State(peer): State<Arc<ManagerPeer>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let source = headers
        .get("x-fixture")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let stateful = peer.stateful && source == "enabled";
    let initialize = request["method"] == "initialize";
    if stateful
        && !initialize
        && request["method"] != "server/discover"
        && headers
            .get("mcp-session-id")
            .is_none_or(|value| value != REMOTE_SESSION)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    peer.rpc.requests.lock().unwrap().push(RpcRequest {
        headers,
        body: request.clone(),
    });
    let result = match request["method"].as_str() {
        Some("server/discover") => {
            return Json(json!({
                "jsonrpc":"2.0", "id":request["id"],
                "error":{"code":-32601,"message":"Method not found"},
            }))
            .into_response();
        }
        Some("initialize") => {
            if source == "enabled"
                && peer
                    .enabled_failures
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                        left.checked_sub(1)
                    })
                    .is_ok()
            {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            if source == "registered" {
                peer.initialized.notify_waiters();
            }
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": format!("fixture-{source}"), "version": "1.0"},
            })
        }
        Some("notifications/initialized" | "notifications/cancelled") => {
            return StatusCode::ACCEPTED.into_response();
        }
        Some("tools/list") => {
            if source == "inactive" {
                tokio::time::sleep(Duration::from_millis(1350)).await;
            }
            if source == "enabled"
                && request["params"]["cursor"] == "second-page"
                && peer.hold_final_page.swap(false, Ordering::Relaxed)
            {
                peer.final_page.notify_one();
                peer.release_final_page.notified().await;
            }
            let tool = |name| json!({ "name": name, "inputSchema": { "type": "object" }});
            if request["params"]["cursor"].is_null() {
                json!({ "tools": [tool("first")], "nextCursor":"second-page" })
            } else {
                json!({ "tools": [tool("second")] })
            }
        }
        Some("tools/call") if request["params"]["name"] == "first" => json!({
            "content": [{"type":"text", "text":request["params"]["arguments"]["text"]}],
        }),
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    let mut response =
        Json(json!({ "jsonrpc": "2.0", "id":request["id"], "result":result })).into_response();
    if stateful && initialize {
        response.headers_mut().insert(
            "mcp-session-id",
            axum::http::HeaderValue::from_static(REMOTE_SESSION),
        );
    }
    response
}

async fn delete_session(State(peer): State<Arc<ManagerPeer>>, headers: HeaderMap) -> StatusCode {
    if headers
        .get("mcp-session-id")
        .is_some_and(|value| value == REMOTE_SESSION)
    {
        peer.deletes.fetch_add(1, Ordering::Relaxed);
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::BAD_REQUEST
    }
}

async fn install(case: &TestCase, url: &str) -> TestResult {
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    let declaration = |source, enabled| {
        json!({
            "transport":"streamable-http", "url":url,
            "headers":{"x-fixture":source}, "enabled":enabled,
        })
    };
    let mut direct = declaration("direct", true);
    direct["serverName"] = json!("direct");
    let entries = json!([
        {"id":"mcp-manager","config":{"servers":{
            "enabled":declaration("enabled", true),
            "inactive":declaration("inactive", false)
        }}},
        {"insert":[
            {"name":"@deepseek-ai/dsh-mcp-client","config":direct},
            {"name":"./commands.mjs","config":{"url":url,"register":true}},
        ]},
    ]);
    let foreign = json!([{"id":"mcp-manager","config":{"servers":{
        "enabled":declaration("foreign-shared-name", true),
        "outside":declaration("foreign", true),
        "outside-disabled":declaration(&format!("${{credential:{OWNER_ENV}}}"), false),
    }}}]);
    let console = json!([{"insert":[{"name":"./commands.mjs"}]}]);
    for (name, bundle) in [
        ("mcp-manager", entries),
        ("mcp-other", foreign),
        ("mcp-console", console),
    ] {
        let root = case.paths().root.join(name);
        tokio::fs::create_dir_all(&root).await?;
        tokio::fs::write(
            root.join("package.json"),
            serde_json::to_vec(&json!({
                "name":format!("@fixture/{name}"), "dsh":{"bundle":{"patch":"bundle.yml"}},
            }))?,
        )
        .await?;
        tokio::fs::write(root.join("bundle.yml"), serde_json::to_vec(&bundle)?).await?;
        if name != "mcp-other" {
            tokio::fs::write(
                root.join("commands.mjs"),
                r#"
export default class ManagerFixture {
  static inject = { commands: null, mcpManager: null };
  constructor({ commands, mcpManager: manager, on, root }, config = {}) {
    const events = [];
    const first = [];
    on('mcp-manager/status', (name, status) => events.push([name, status.state]));
    root.events.once('mcp-manager/status', (name, status) => first.push([name, status.state]));
    if (config?.register) {
        manager.register('registered', {
            transport: 'streamable-http', url: config.url, headers: { 'x-fixture': 'registered' },
        });
    }
    commands.register({
        name: 'manage', description: 'Exercise the imported MCP manager',
        async handler({rawInput}) {
            const ok = value => ({ kind: 'success', text: JSON.stringify(value) });
            if (rawInput === 'events') return ok(events);
            if (rawInput === 'first-event') return ok(first);
            if (rawInput === 'start') {
                await manager.restart('enabled');
                return ok(manager.list());
            }
            if (rawInput === 'probe') {
                const probe = await manager.probe('inactive', { maxPages: 1, maxToolNames: 1 });
                return ok({ probe, after: manager.list() });
            }
            if (rawInput === 'timeout') {
                return ok(await manager.probe('inactive', { timeoutMs: 850 }));
            }
            if (rawInput === 'abort') {
                const signal = AbortSignal.any([new AbortController().signal, AbortSignal.abort('fixture')]);
                if (signal.reason !== 'fixture') throw new Error('combined abort reason was lost');
                return ok(await manager.probe('inactive', { signal }));
            }
            if (rawInput === 'cancel') {
                const signal = AbortSignal.any([AbortSignal.timeout(3000), AbortSignal.timeout(850)]);
                return ok(await manager.probe('inactive', { signal }));
            }
            if (rawInput.startsWith('watch ')) {
                const target = rawInput.slice(6);
                const controller = new AbortController();
                const stream = manager.watch(AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]));
                const initial = await stream.next();
                let waiting = stream.next();
                await manager.restart(target);
                let changed = await waiting;
                while (!changed.done && changed.value.find(row => row.name === target)?.status.state !== 'connected') {
                    changed = await stream.next();
                }
                waiting = stream.next();
                controller.abort();
                return ok({ initial, changed, events, finished: (await waiting).done });
            }
            if (rawInput === 'watch-permission') {
                const stream = manager.watch(AbortSignal.timeout(10000));
                const initial = await stream.next();
                const change = stream.next();
                const marker = await manager.probe('registered', {maxPages: 1});
                return ok({ initial, change: await change, marker });
            }
            if (rawInput === 'watch-reload') {
                const signal = AbortSignal.timeout(20000);
                const stream = manager.watch(signal);
                const initial = await stream.next();
                const change = stream.next();
                const marker = await manager.probe('registered', {maxPages: 1});
                return ok({ initial, change: await change, marker, aborted: signal.aborted });
            }
            if (rawInput.startsWith('watch-clear ')) {
                await manager.restart(rawInput.slice(12));
                const stream = manager.watch(AbortSignal.timeout(10000));
                const initial = await stream.next();
                const change = stream.next();
                const marker = await manager.probe('registered', {maxPages: 1});
                return ok({ initial, change: await change, marker });
            }
            if (rawInput === 'restart') {
                const before = manager.list();
                await manager.restart('inactive');
                await manager.restart('enabled');
                const first = manager.list();
                await manager.restart('enabled');
                return ok({ before, first, after: manager.list() });
            }
            if (rawInput.startsWith('probe ')) {
                try {
                    return ok(await manager.probe(rawInput.slice(6)));
                } catch (error) {
                    return ok(String(error));
                }
            }
            if (rawInput === 'denied') {
                try {
                    await manager.probe('enabled');
                    return { kind: 'error', text: 'Unexpected probe success' };
                } catch (error) {
                    return ok(String(error));
                }
            }
            return ok(manager.list());
        },
    });
  }
}
"#,
            )
            .await?;
        }
        let mut args = vec!["plugin", "install", root.to_str().context("package path")?];
        if name == "mcp-other" {
            args.extend(["--env", OWNER_ENV]);
        }
        case.run_cli(args).await?.require_success()?;
    }
    Ok(())
}

async fn invoke(user: &UserSession, plugin: &str, args: &str) -> anyhow::Result<Value> {
    let result = user
        .client()
        .invoke_plugin_command(plugin, "manage", json!({}), Some(user.id()), None, args)
        .await?
        .result;
    ensure!(result.status == CommandStatus::Succeeded, "{result:?}");
    Ok(serde_json::from_str(&result.summary)?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_deepseek_manager_reconnects_between_commands_and_retires_listeners() -> TestResult
{
    let peer = Arc::new(ManagerPeer {
        enabled_failures: AtomicUsize::new(2),
        ..Default::default()
    });
    let setup = peer.clone();
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .env(OWNER_ENV, "foreign-disabled")
        .expect_plugin(PLUGIN)
        .before_start(move |case| async move {
            let server = case
                .serve_http_on(
                    native_fixture_address()?,
                    Router::new().route("/mcp", post(rpc)).with_state(setup),
                )
                .await?;
            install(&case, &server.url("/mcp")).await
        })
        .run(move |case| async move {
            wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            allow_permissions(&user).await?;
            allow_permissions(&other).await?;
            ensure!(invoke(&user, PLUGIN, "events").await? == json!([]));
            ensure!(invoke(&other, PLUGIN, "events").await? == json!([]));
            invoke(&user, PLUGIN, "start").await?;
            let attempts = || {
                peer.rpc
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|request| {
                        request.body["method"] == "initialize"
                            && request
                                .headers
                                .get("x-fixture")
                                .is_some_and(|value| value == "enabled")
                    })
                    .count()
            };
            tokio::time::timeout(Duration::from_secs(8), async {
                while attempts() < 3 {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await?;
            let events = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let events = invoke(&user, PLUGIN, "events").await?;
                    if events
                        .as_array()
                        .is_some_and(|events| events.contains(&json!([LOCAL_SHARED, "connected"])))
                    {
                        return Ok::<_, anyhow::Error>(events);
                    }
                }
            })
            .await??;
            ensure!(
                events
                    .as_array()
                    .context("status history")?
                    .iter()
                    .filter(|event| **event == json!([LOCAL_SHARED, "reconnecting"]))
                    .count()
                    >= 2,
                "listener missed the retry published between commands: {events:#}"
            );
            ensure!(invoke(&other, PLUGIN, "events").await? == json!([]));
            let first = invoke(&user, PLUGIN, "first-event").await?;
            ensure!(first == json!([events[0]]), "{first:#}");
            let initial = invoke(&other, PLUGIN, "list").await?;

            // Revoke while the final discovery response is already in flight.
            // No later request is needed for the transport to report readiness.
            peer.hold_final_page.store(true, Ordering::Relaxed);
            let revoke = async {
                tokio::time::timeout(Duration::from_secs(5), peer.final_page.notified()).await?;
                let result = user
                    .client()
                    .set_session_permission(
                        user.id(),
                        serde_json::from_value(json!({
                            "tools": {"default": "allow"},
                            "network": {"default": "deny"},
                        }))?,
                    )
                    .await;
                peer.release_final_page.notify_one();
                result?;
                Ok::<_, anyhow::Error>(())
            };
            let (_, ()) = tokio::try_join!(invoke(&user, PLUGIN, "start"), revoke)?;
            let failure = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let catalog = invoke(&user, PLUGIN, "list").await?;
                    if let Some(row) =
                        catalog
                            .as_array()
                            .context("status catalog")?
                            .iter()
                            .find(|row| {
                                row["name"] == LOCAL_SHARED && row["status"]["state"] == "failed"
                            })
                    {
                        return Ok::<_, anyhow::Error>(row["status"].clone());
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await??;
            ensure!(failure["reason"] == "reconnect-disabled", "{failure:#}");

            user.client().clear_session(user.id(), false).await?;
            ensure!(invoke(&user, PLUGIN, "list").await? == initial);
            ensure!(invoke(&user, PLUGIN, "events").await? == json!([]));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_deepseek_manager_controls_the_session_fleet() -> TestResult {
    let peer = Arc::new(ManagerPeer::default());
    let setup = Arc::clone(&peer);
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env(OWNER_ENV, "foreign-disabled")
        .env_relative("PATH", "empty-bin")
        .builtin_plugin(
            "agena.mcp",
            json!({"runtime":{"reconnect":{"enabled":false}}}),
        )
        .expect_plugin(PLUGIN)
        .expect_plugin(OTHER)
        .expect_plugin(CONSOLE)
        .before_start(move |case| async move {
            let server = case
                .serve_http_on(
                    native_fixture_address()?,
                    Router::new().route("/mcp", post(rpc)).with_state(setup),
                )
                .await?;
            install(&case, &server.url("/mcp")).await
        })
        .run(move |case| async move {
            wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let listed = invoke(&user, PLUGIN, "list").await?;
            ensure!(invoke(&user, CONSOLE, "list").await? == listed);
            let listed_rows = listed
                .as_array()
                .context("expected an MCP manager catalog")?;
            let names: Vec<_> = listed_rows
                .iter()
                .filter_map(|row| row["name"].as_str())
                .collect();
            ensure!(
                names.len() == 6 && names.last() == Some(&"registered"),
                "{listed:#}"
            );
            for (name, state) in [
                (LOCAL_SHARED, "closed"),
                (FOREIGN_SHARED, "closed"),
                ("registered", "closed"),
                ("outside", "closed"),
                ("outside-disabled", "disabled"),
                ("inactive", "disabled"),
            ] {
                let row = listed_rows
                    .iter()
                    .find(|row| row["name"] == name)
                    .context("missing service")?;
                ensure!(
                    row["configured"] == true && row["status"]["state"] == state,
                    "{row:#}"
                );
            }
            let direct = invoke(&user, CONSOLE, "probe direct").await?;
            ensure!(
                direct.as_str().is_some_and(|s| s.contains("unavailable")),
                "{direct}"
            );
            let ambiguous = invoke(&user, CONSOLE, "probe enabled").await?;
            ensure!(
                ambiguous
                    .as_str()
                    .is_some_and(|s| s.contains("qualified name")),
                "{ambiguous}"
            );
            let aborted = invoke(&user, PLUGIN, "abort").await?;
            ensure!(
                aborted["ok"] == false
                    && aborted["message"] == "probe aborted"
                    && aborted["closeConfirmed"] == true,
                "{aborted:#}"
            );
            ensure!(
                peer.rpc.requests.lock().unwrap().is_empty(),
                "observation or rejected operation opened a connection"
            );

            for (server, identity) in [
                ("outside", "fixture-foreign"),
                ("outside-disabled", "fixture-[redacted]"),
            ] {
                let result = invoke(&user, CONSOLE, &format!("probe {server}")).await?;
                ensure!(
                    result["ok"] == true && result["serverInfo"]["name"] == identity,
                    "{result:#}"
                );
            }

            let probed = invoke(&user, PLUGIN, "probe").await?;
            ensure!(probed["after"] == listed, "{probed:#}");
            let probe = &probed["probe"];
            ensure!(
                probe["ok"] == true && probe["closeConfirmed"] == true,
                "{probe:#}"
            );
            ensure!(probe["serverInfo"] == json!({"name":"fixture-inactive","version":"1.0"}));
            ensure!(probe["toolNames"] == json!(["first"]) && probe["truncated"] == true);
            ensure!(probe["durationMs"].as_i64().unwrap_or_default() >= 1300);

            let timed_out = invoke(&user, PLUGIN, "timeout").await?;
            ensure!(
                timed_out["ok"] == false
                    && timed_out["stage"] == "list"
                    && timed_out["closeConfirmed"] == true,
                "{timed_out:#}"
            );
            let canceled = invoke(&user, PLUGIN, "cancel").await?;
            ensure!(
                canceled["ok"] == false
                    && canceled["stage"] == "list"
                    && canceled["message"] == "probe aborted"
                    && canceled["closeConfirmed"] == true,
                "{canceled:#}"
            );

            let restarted = invoke(&user, PLUGIN, "restart").await?;
            ensure!(restarted["before"] == listed, "{restarted:#}");
            for phase in ["first", "after"] {
                let rows = restarted[phase]
                    .as_array()
                    .context("missing restart catalog")?;
                let find = |name: &str| rows.iter().find(|row| row["name"] == name).unwrap();
                let status = &find(LOCAL_SHARED)["status"];
                ensure!(
                    status["state"] == "connected" && status["toolCount"] == 2,
                    "{status:#}"
                );
                ensure!(status["connectedAt"].is_i64());
                ensure!(find("inactive")["status"]["state"] == "disabled");
                ensure!(find(FOREIGN_SHARED)["status"]["state"] == "closed");
            }
            let watched = invoke(&user, CONSOLE, &format!("watch {FOREIGN_SHARED}")).await?;
            ensure!(
                watched["initial"]["value"] == listed && watched["finished"] == true,
                "{watched:#}"
            );
            let changed = watched["changed"]["value"]
                .as_array()
                .context("missing watch catalog")?;
            ensure!(
                changed
                    .iter()
                    .any(|row| row["name"] == FOREIGN_SHARED
                        && row["status"]["state"] == "connected"),
                "{watched:#}"
            );
            ensure!(
                watched["events"]
                    .as_array()
                    .is_some_and(|events| events.contains(&json!([FOREIGN_SHARED, "connected"]))),
                "{watched:#}"
            );
            ensure!(invoke(&user, PLUGIN, "list").await? == listed);
            let initializations = {
                let requests = peer.rpc.requests.lock().unwrap();
                let mut initialized = Vec::new();
                for entry in requests
                    .iter()
                    .filter(|entry| entry.body["method"] == "initialize")
                {
                    initialized.push(entry.headers["x-fixture"].to_str()?.to_owned());
                }
                initialized
            };
            ensure!(
                initializations
                    == [
                        "foreign",
                        "foreign-disabled",
                        "inactive",
                        "inactive",
                        "inactive",
                        "enabled",
                        "enabled",
                        "foreign-shared-name"
                    ],
                "{initializations:?}"
            );

            let barrier = peer.initialized.notified();
            tokio::pin!(barrier);
            barrier.as_mut().enable();
            let clear = async {
                tokio::time::timeout(Duration::from_secs(5), barrier).await?;
                user.client().clear_session(user.id(), false).await?;
                Ok::<(), anyhow::Error>(())
            };
            let watch_clear = format!("watch-clear {LOCAL_SHARED}");
            let (watched, ()) = tokio::try_join!(invoke(&user, CONSOLE, &watch_clear), clear)?;
            ensure!(watched["marker"]["ok"] == true, "{watched:#}");
            ensure!(
                watched["initial"]["value"]
                    .as_array()
                    .is_some_and(|rows| rows.iter().any(|row| {
                        row["name"] == LOCAL_SHARED && row["status"]["state"] == "connected"
                    })),
                "{watched:#}"
            );
            ensure!(watched["change"]["value"] == listed, "{watched:#}");

            let barrier = peer.initialized.notified();
            tokio::pin!(barrier);
            barrier.as_mut().enable();
            let update = async {
                tokio::time::timeout(Duration::from_secs(5), barrier).await?;
                user.client()
                    .set_session_permission(
                        user.id(),
                        serde_json::from_value(json!({
                            "plugins":{(OTHER):false}, "tools":{"default":"allow"},
                            "network":super::super::super::native_fixture_network()?,
                        }))?,
                    )
                    .await?;
                Ok::<(), anyhow::Error>(())
            };
            let (watched, ()) =
                tokio::try_join!(invoke(&user, CONSOLE, "watch-permission"), update)?;
            ensure!(
                watched["initial"]["value"] == listed && watched["marker"]["ok"] == true,
                "{watched:#}"
            );
            let permitted = watched["change"]["value"]
                .as_array()
                .context("missing permission update")?;
            ensure!(
                permitted.iter().map(|row| row["name"].as_str()).eq([
                    Some(LOCAL_SHARED),
                    Some("inactive"),
                    Some("registered"),
                ]),
                "{watched:#}"
            );
            let before = peer.rpc.requests.lock().unwrap().len();
            for name in [
                "outside",
                FOREIGN_SHARED,
                "plugin:deepseek.fixture-mcp-other:outside-disabled",
            ] {
                let rejected = invoke(&user, CONSOLE, &format!("probe {name}")).await?;
                ensure!(
                    rejected.as_str().is_some_and(|s| s.contains("unavailable")),
                    "{rejected}"
                );
            }
            ensure!(peer.rpc.requests.lock().unwrap().len() == before);

            let barrier = peer.initialized.notified();
            tokio::pin!(barrier);
            barrier.as_mut().enable();
            let reload = async {
                tokio::time::timeout(Duration::from_secs(5), barrier).await?;
                case.settings()
                    .with_timeout(Duration::from_secs(30))
                    .reload_and_wait()
                    .await?;
                Ok::<(), anyhow::Error>(())
            };
            let (retired, ()) = tokio::try_join!(invoke(&user, CONSOLE, "watch-reload"), reload)?;
            ensure!(
                retired["initial"]["value"] == watched["change"]["value"]
                    && retired["marker"]["ok"] == true
                    && retired["change"]["done"] == true
                    && retired["aborted"] == false,
                "{retired:#}"
            );
            ensure!(invoke(&user, CONSOLE, "list").await? == watched["change"]["value"]);

            let denied_user = case.user().new_session().await?;
            denied_user
                .client()
                .set_session_permission(
                    denied_user.id(),
                    serde_json::from_value(json!({
                        "network":{"default":"deny"}, "tools":{"default":"allow"},
                    }))?,
                )
                .await?;
            let before = peer.rpc.requests.lock().unwrap().len();
            let denied = invoke(&denied_user, PLUGIN, "denied").await?;
            ensure!(denied.is_string());
            ensure!(peer.rpc.requests.lock().unwrap().len() == before);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_deepseek_manager_reuses_http_for_ordinary_calls() -> TestResult {
    let peer = Arc::new(ManagerPeer::default());
    let setup = Arc::clone(&peer);
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env(OWNER_ENV, "foreign-disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(move |case| async move {
            let server = case
                .serve_http_on(
                    native_fixture_address()?,
                    Router::new().route("/mcp", post(rpc)).with_state(setup),
                )
                .await?;
            install(&case, &server.url("/mcp")).await
        })
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            wait_for_startup_catalog(&case).await?;
            let search = user
                .call_tool("mcp.tools.search", json!({"query": ENABLED_SERVER}))
                .await?;
            search.assert_succeeded()?;
            let catalog = search.data()?;
            ensure!(
                catalog["results"].as_array().context("MCP tool catalog")?.iter().any(|entry| {
                    entry["server"] == ENABLED_SERVER && entry["name"] == "first"
                }),
                "managed fixture is missing from the MCP tool catalog: {catalog:#}"
            );
            let arguments = |text: &str| {
                json!({"server":ENABLED_SERVER, "name":"first", "arguments":{"text":text}})
            };
            for marker in ["ordinary-call-one", "ordinary-call-two"] {
                let response = user.call_tool("mcp.tools.call", arguments(marker)).await?;
                response.assert_succeeded()?;
                response.assert_text_contains(marker)?;
            }
            let views = invoke(&user, PLUGIN, "list").await?;
            ensure!(
                views
                    .as_array()
                    .context("status catalog")?
                    .iter()
                    .any(|view| {
                        view["name"] == LOCAL_SHARED && view["status"]["state"] == "connected"
                    }),
                "{views:#}"
            );
            user.client()
                .set_session_permission(
                    user.id(),
                    serde_json::from_value(json!({
                        "tools":{"default":"allow"}, "network":{"default":"deny"},
                    }))?,
                )
                .await?;
            user.call_tool("mcp.tools.call", arguments("denied"))
                .await?
                .assert_failed()?;
            let records = peer.rpc.requests.lock().unwrap();
            let methods = records
                .iter()
                .filter_map(|record| {
                    (record.headers.get("x-fixture")?.to_str().ok()? == "enabled")
                        .then(|| record.body["method"].as_str())
                        .flatten()
                })
                .collect::<Vec<_>>();
            ensure!(
                methods
                    .iter()
                    .filter(|method| **method == "initialize")
                    .count()
                    == 1,
                "{methods:?}"
            );
            ensure!(
                methods
                    .iter()
                    .filter(|method| **method == "tools/call")
                    .count()
                    == 2,
                "{methods:?}"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_deepseek_manager_keeps_failed_cleanup_across_reload() -> TestResult {
    let peer = Arc::new(ManagerPeer {
        stateful: true,
        ..Default::default()
    });
    let setup = Arc::clone(&peer);
    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env(OWNER_ENV, "foreign-disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(move |case| async move {
            let server = case
                .serve_http_on(
                    native_fixture_address()?,
                    Router::new()
                        .route(
                            "/mcp",
                            post(rpc)
                                .get(|| async { StatusCode::METHOD_NOT_ALLOWED })
                                .delete(delete_session),
                        )
                        .with_state(setup),
                )
                .await?;
            install(&case, &server.url("/mcp")).await
        })
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            wait_for_startup_catalog(&case).await?;
            let initial = invoke(&user, PLUGIN, "start").await?;
            ensure!(
                initial
                    .as_array()
                    .context("status catalog")?
                    .iter()
                    .any(|view| {
                        view["name"] == LOCAL_SHARED && view["status"]["state"] == "connected"
                    }),
                "{initial:#}"
            );

            let path = case.paths().root.join("mcp-manager/bundle.yml");
            let original = tokio::fs::read(&path).await?;
            let mut removed: Value = serde_json::from_slice(&original)?;
            removed[0]["config"]["servers"]
                .as_object_mut()
                .context("server declarations")?
                .remove("enabled")
                .context("managed declaration")?;
            tokio::fs::write(&path, serde_json::to_vec(&removed)?).await?;
            case.settings()
                .with_timeout(Duration::from_secs(30))
                .reload_and_wait()
                .await?;
            let cleanup = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let views = invoke(&user, CONSOLE, "list").await?;
                    if let Some(view) =
                        views
                            .as_array()
                            .context("status catalog")?
                            .iter()
                            .find(|view| {
                                view["name"] == LOCAL_SHARED && view["status"]["state"] == "failed"
                            })
                    {
                        return Ok::<_, anyhow::Error>(view.clone());
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await??;
            ensure!(
                cleanup["configured"] == false
                    && cleanup["status"]["reason"] == "cleanup"
                    && cleanup["status"]["result"]["closeConfirmed"] == false,
                "{cleanup:#}"
            );
            ensure!(peer.deletes.load(Ordering::Relaxed) == 1);

            tokio::fs::write(&path, original).await?;
            case.settings()
                .with_timeout(Duration::from_secs(30))
                .reload_and_wait()
                .await?;
            let attempted = invoke(&user, CONSOLE, &format!("probe {LOCAL_SHARED}")).await?;
            ensure!(
                attempted.is_string(),
                "the replacement opened a transport: {attempted}"
            );
            user.call_tool(
                "mcp.tools.call",
                json!({"server":ENABLED_SERVER,"name":"first","arguments":{"text":"blocked"}}),
            )
            .await?
            .assert_failed()?;
            let records = peer.rpc.requests.lock().unwrap();
            ensure!(
                records
                    .iter()
                    .filter(|record| record.body["method"] == "initialize"
                        && record
                            .headers
                            .get("x-fixture")
                            .is_some_and(|value| value == "enabled"))
                    .count()
                    == 1
            );
            Ok(())
        })
        .await
}
