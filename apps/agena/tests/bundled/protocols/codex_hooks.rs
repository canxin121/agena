use std::sync::{Arc, Mutex};

use agena_api::{
    commands::{Command, CommandResult, ForkSessionParams},
    resource::RunOptions,
};
use agena_testkit::{Reply, RequestMatch, SessionOptions, TestCase, TestResult, UserSession};
use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};

const PLUGIN: &str = "codex.fixture-codex-start";

#[derive(Default)]
struct SamplingPeer {
    calls: Mutex<Vec<Value>>,
    changed: tokio::sync::Notify,
}

async fn sampling_rpc(
    State(peer): State<Arc<SamplingPeer>>,
    Json(request): Json<Value>,
) -> Response {
    let mut result = match request["method"].as_str() {
        Some("server/discover") => json!({
            "supportedVersions":["2026-07-28"],"capabilities":{"tools":{}},"ttlMs":0,"cacheScope":"private"
        }),
        Some("tools/list") => json!({"tools":[
            {"name":"observe","inputSchema":{"type":"object"}},
            {"name":"bad_charge","inputSchema":{"type":"object"}}
        ]}),
        Some("tools/call") if request["params"]["name"] == "observe" => {
            peer.calls
                .lock()
                .unwrap()
                .push(request["params"]["arguments"].clone());
            peer.changed.notify_one();
            json!({
                "content":[{"type":"text","text":r#"{"continue":false,"decision":"block","systemMessage":"unexpected sampling hook context"}"#}],
                "structuredContent":{"extra_codex_rollout_budget_units":2.5}
            })
        }
        Some("tools/call") if request["params"]["name"] == "bad_charge" => json!({
            "content":[],
            "structuredContent":{"extra_codex_rollout_budget_units":-10}
        }),
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    result["resultType"] = json!("complete");
    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})).into_response()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_codex_model_hooks_observe_attempts_and_record_units() -> TestResult {
    use agena_testkit::Usage;
    use std::time::Duration;

    let peer = Arc::new(SamplingPeer::default());
    let server_peer = peer.clone();
    let network = super::super::native_fixture_network()?;
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin("codex.fixture-sampling")
        .builtin_plugin("agena.mcp", json!({"runtime":{"reconnect":{"enabled":false}}}))
        .configure_global(move |config| {
            config["permission"]["network"] = network.clone();
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["tools"]["names"]["agena.mcp.tools.call"] = json!("allow");
        })
        .before_start(move |case| async move {
            let server = case.serve_http_on(
                super::super::native_fixture_address()?,
                Router::new().route("/mcp", post(sampling_rpc)).with_state(server_peer),
            ).await?;
            let package = case.paths().root.join("codex-sampling-hooks");
            tokio::fs::create_dir(case.paths().root.join("empty-bin")).await?;
            tokio::fs::create_dir_all(package.join(".codex-plugin")).await?;
            let mut hooks = serde_json::Map::new();
            for event in ["PreModelRequest", "PostModelRequest"] {
                let mut input = serde_json::Map::new();
                for field in [
                    "hook_event_name", "request_id", "attempt_number", "kind", "model",
                    "session_id", "thread_id", "turn_id", "cwd",
                ] {
                    input.insert(field.into(), json!(format!("${{{field}}}")));
                }
                let mut handlers = Vec::new();
                if event == "PostModelRequest" {
                    for field in ["status", "error", "token_usage", "response_id"] {
                        input.insert(field.into(), json!(format!("${{{field}}}")));
                    }
                    handlers.push(json!({"type":"mcp_tool", "server":"sampling", "tool":"bad_charge"}));
                }
                handlers.push(json!({
                    "type":"mcp_tool", "server":"sampling", "tool":"observe", "input":input,
                    "timeout":5
                }));
                hooks.insert(event.into(), json!([{"matcher":"^ignored$", "hooks":handlers}]));
            }
            tokio::fs::write(
                package.join(".codex-plugin/plugin.json"),
                serde_json::to_vec(&json!({"name":"fixture-sampling","hooks":hooks}))?,
            ).await?;
            tokio::fs::write(
                package.join(".mcp.json"),
                serde_json::to_vec(&json!({"mcpServers":{"sampling":{"type":"http","url":server.url("/mcp")}}}))?,
            ).await?;
            case.run_cli(["plugin", "install", "codex:fixture-sampling", "--package-source", package.to_str().context("package path")?, "--enable-hooks"])
                .await?.require_success()?;
            Ok(())
        })
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            let root = case.workspace().canonicalize()?;
            let check_before = peer.clone();
            case.model().expect(RequestMatch::user_text("try sampling twice").and(|request| request.sequence == 1))
                .check(move |_| {
                    let calls = check_before.calls.lock().unwrap();
                    ensure!(calls.len() == 1 && calls[0]["hook_event_name"] == "PreModelRequest", "{calls:?}");
                    Ok(())
                })
                .reply(Reply::raw_json(503, json!({"error":{"message":"fixture retry","type":"server_error"}})));
            let check_retry = peer.clone();
            let success = case.model().expect(RequestMatch::user_text("try sampling twice").and(|request| request.sequence == 2))
                .check(move |request| {
                    let calls = check_retry.calls.lock().unwrap();
                    ensure!(calls.len() == 3 && calls[2]["attempt_number"] == "2", "{calls:?}");
                    ensure!(!request.body.to_string().contains("unexpected sampling hook context"));
                    Ok(())
                })
                .reply(Reply::text("completed sampling").usage(Usage::tokens(101, 31).cached(21).reasoning(11).codex_rollout_units(1.25)));
            user.send("try sampling twice").await?.wait_settled().await?
                .assert_assistant_text("completed sampling")?;

            {
                let calls = peer.calls.lock().unwrap();
                ensure!(calls.len() == 4, "{calls:?}");
                let request = calls[0]["request_id"].as_str().context("generation ID")?;
                let turn = calls[0]["turn_id"].as_str().context("turn ID")?;
                ensure!(!request.is_empty() && !turn.is_empty());
                for (index, call) in calls.iter().enumerate() {
                    ensure!(call["request_id"] == request && call["turn_id"] == turn, "{call}");
                    ensure!(call["session_id"] == user.id().to_string() && call["thread_id"] == user.id().to_string(), "{call}");
                    ensure!(call["cwd"] == json!(root) && call["model"] == "fake-model" && call["kind"] == "sampling", "{call}");
                    ensure!(call["attempt_number"] == (index / 2 + 1).to_string(), "{call}");
                    let event = if index % 2 == 0 { "PreModelRequest" } else { "PostModelRequest" };
                    ensure!(call["hook_event_name"] == event, "{call}");
                }
                ensure!(calls[1]["status"] == "failed" && calls[1]["token_usage"] == "null"
                    && calls[1]["response_id"] == "null", "{calls:?}");
                ensure!(calls[1]["error"].as_str().unwrap_or_default().contains("503"), "{calls:?}");
                let post = &calls[3];
                ensure!(post["status"] == "completed" && post["error"] == "null", "{post}");
                ensure!(post["response_id"] == format!("resp_testkit_{}", success.id()), "{post}");
                let usage: Value = serde_json::from_str(post["token_usage"].as_str().context("serialized token usage")?)?;
                ensure!(usage == json!({
                    "input_tokens":101, "cached_input_tokens":21, "cache_write_input_tokens":0,
                    "output_tokens":31, "reasoning_output_tokens":11, "total_tokens":132
                }), "{usage}");
            }

            let incomplete = case.model().expect(RequestMatch::user_text("sample incomplete"))
                .reply(Reply::text("incomplete but usable").incomplete("max_output_tokens").usage(Usage::tokens(78, 9).codex_rollout_units(2.5)));
            case.model().expect(RequestMatch::user_text("sample incomplete"))
                .after(&incomplete).reply(Reply::text("completed continuation").without_usage());
            let result = user.send("sample incomplete").await?.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_text_contains("incomplete but usable")?;
            result.assert_text_contains("completed continuation")?;
            {
                let calls = peer.calls.lock().unwrap();
                ensure!(calls.len() == 8 && calls[4]["attempt_number"] == "1", "{calls:?}");
                ensure!(calls[4]["request_id"] != calls[0]["request_id"], "{calls:?}");
                ensure!(calls[5]["status"] == "failed" && calls[5]["response_id"] == format!("resp_testkit_{}", incomplete.id()), "{calls:?}");
                ensure!(calls[5]["error"].as_str().unwrap_or_default().contains("max_output_tokens"), "{calls:?}");
                ensure!(calls[5]["token_usage"] != "null", "{calls:?}");
                ensure!(calls[6]["request_id"] != calls[4]["request_id"] && calls[6]["turn_id"] == calls[4]["turn_id"], "{calls:?}");
                ensure!(calls[6]["attempt_number"] == "1" && calls[7]["status"] == "completed"
                    && calls[7]["token_usage"] == "null", "{calls:?}");
            }
            let cost = user.client().session_cost_summary(user.id()).await?;
            let units = cost.billable_units.iter().find(|item| item.kind == "extra_codex_rollout_budget_units")
                .context("Codex model hook units in committed usage")?;
            ensure!(units.quantity == 7.5 && units.estimated_cost_usd == 0.0, "{units:?}");
            let native = cost.billable_units.iter().find(|item| item.kind == "codex_rollout_budget_units")
                .context("provider rollout units in committed usage")?;
            ensure!(native.quantity == 3.75 && native.estimated_cost_usd == 0.0, "{native:?}");

            let compact_response = case.model().expect(RequestMatch::new("manual compaction", |request| {
                request.body.to_string().contains("Summarize the conversation for the next assistant continuation")
            })).reply(Reply::text("sampling hook compact checkpoint"));
            let compact = user.client().compact_session(user.id(), Default::default()).await?;
            user.attach_run(compact.receipt.context("compaction receipt")?).wait_settled().await?.assert_succeeded()?;
            {
                let calls = peer.calls.lock().unwrap();
                ensure!(calls.len() == 10 && calls[8]["attempt_number"] == "1", "{calls:?}");
                ensure!(calls[8]["kind"] == "foreground_compaction" && calls[9]["kind"] == "foreground_compaction", "{calls:?}");
                ensure!(calls[8]["request_id"] == calls[9]["request_id"] && calls[9]["status"] == "completed", "{calls:?}");
                ensure!(calls[9]["response_id"] == format!("resp_testkit_{}", compact_response.id()), "{calls:?}");
            }

            let held = case.model().expect(RequestMatch::user_text("cancel in provider"))
                .reply(Reply::text("must not become visible").held());
            let run = user.send("cancel in provider").await?;
            held.observed().await?;
            run.cancel().await?;
            run.wait_settled().await?.assert_cancelled()?;
            held.disconnected().await?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let changed = peer.changed.notified();
                    if peer.calls.lock().unwrap().len() >= 12 {
                        break;
                    }
                    changed.await;
                }
            }).await.context("cancelled sampling hook")?;
            let calls = peer.calls.lock().unwrap();
            ensure!(calls.len() == 12 && calls[10]["request_id"] == calls[11]["request_id"], "{calls:?}");
            ensure!(calls[11]["status"] == "cancelled" && calls[11]["token_usage"] == "null"
                && calls[11]["error"] == "null" && calls[11]["response_id"] == "null", "{calls:?}");
            Ok(())
        })
        .await
}

#[cfg(unix)]
const UPDATED_CWD_COMMAND: &str = "printf 'rewritten:%s' \"$PWD\"";
#[cfg(unix)]
const BLOCK_BEFORE_COMMAND: &str = "printf blocked > blocked.txt";
#[cfg(unix)]
const WITHHOLD_AFTER_COMMAND: &str = "printf 'withheld payload'; printf committed > committed.txt";
#[cfg(unix)]
const TERMINAL_COMMAND: &str = "printf 'terminal-ready\n'; IFS= read -r answer; printf 'terminal-complete:%s\n' \"$answer\"; exit 7";

#[cfg(unix)]
async fn cwd_rpc(
    State(calls): State<Arc<Mutex<Vec<Value>>>>,
    Json(request): Json<Value>,
) -> Response {
    let mut result = match request["method"].as_str() {
        Some("server/discover") => json!({
            "supportedVersions":["2026-07-28"],"capabilities":{"tools":{}},
            "ttlMs":0,"cacheScope":"private"
        }),
        Some("tools/list") => json!({"tools":[{"name":"record","inputSchema":{"type":"object"}}]}),
        Some("tools/call") if request["params"]["name"] == "record" => {
            let arguments = request["params"]["arguments"].clone();
            let input = arguments["tool_input"]
                .as_str()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .unwrap_or_default();
            let event = arguments["event"].as_str().unwrap_or_default();
            let response = match (event, input["command"].as_str()) {
                ("PreToolUse", Some("printf original")) => json!({"hookSpecificOutput":{
                    "hookEventName":"PreToolUse","permissionDecision":"allow",
                    "updatedInput":{"command":UPDATED_CWD_COMMAND,"workdir":"/"}
                }}),
                ("PreToolUse", Some("printf terminal-original")) => json!({"hookSpecificOutput":{
                    "hookEventName":"PreToolUse","updatedInput":{"command":TERMINAL_COMMAND}
                }}),
                _ => json!({}),
            };
            let failure = matches!(
                (event, input["command"].as_str()),
                ("PreToolUse", Some(BLOCK_BEFORE_COMMAND))
                    | ("PostToolUse", Some(WITHHOLD_AFTER_COMMAND))
                    | ("PostCompact", _)
            );
            calls.lock().unwrap().push(arguments);
            if failure {
                json!({"isError":true,"content":[{"type":"text","text":"required fixture hook failed"}]})
            } else {
                json!({"content":[{"type":"text","text":response.to_string()}]})
            }
        }
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    result["resultType"] = json!("complete");
    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})).into_response()
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_codex_bash_hooks_keep_the_local_command_directory_and_honor_required_checks()
-> TestResult {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let peer = calls.clone();
    let network = super::super::native_fixture_network()?;
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env("AGENA_SHELL_SNAPSHOT", "disabled")
        .expect_plugin("codex.fixture-codex-cwd")
        .builtin_plugin("agena.mcp", json!({"runtime":{"reconnect":{"enabled":false}}}))
        .configure_global(move |config| {
            config["permission"]["network"] = network.clone();
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["tools"]["names"]["agena.shell.exec"] = json!("ask");
            config["permission"]["tools"]["names"]["agena.shell.open"] = json!("ask");
            config["permission"]["tools"]["names"]["agena.shell.write"] = json!("allow");
            config["permission"]["tools"]["names"]["agena.shell.read"] = json!("allow");
            config["permission"]["tools"]["names"]["agena.shell.logs"] = json!("allow");
            config["permission"]["tools"]["names"]["agena.shell.spawn"] = json!("allow");
            config["permission"]["tools"]["names"]["agena.shell.watch"] = json!("allow");
            config["permission"]["tools"]["names"]["agena.mcp.tools.call"] = json!("allow");
        })
        .before_start(move |case| async move {
            let server = case.serve_http_on(
                super::super::native_fixture_address()?,
                Router::new().route("/mcp", post(cwd_rpc)).with_state(peer),
            ).await?;
            let package = case.paths().root.join("codex-shell-hooks");
            tokio::fs::create_dir_all(package.join(".codex-plugin")).await?;
            tokio::fs::create_dir_all(case.workspace().join("nested")).await?;
            let mut hooks = serde_json::Map::new();
            let shell_matcher = "^(Bash|read|write|logs)$";
            for (event, matcher) in [
                ("PreToolUse", shell_matcher), ("PermissionRequest", shell_matcher), ("PostToolUse", shell_matcher),
                ("PostCompact", "manual"), ("SessionStart", "compact"),
            ] {
                let mut input = json!({"event":"${hook_event_name}","cwd":"${cwd}"});
                if matcher == shell_matcher {
                    input["tool_input"] = json!("${tool_input}");
                }
                if matches!(event, "PreToolUse" | "PostToolUse") {
                    input["tool_use_id"] = json!("${tool_use_id}");
                }
                if event == "PostToolUse" {
                    input["tool_response"] = json!("${tool_response}");
                }
                hooks.insert(event.into(), json!([{"matcher":matcher,"hooks":[{
                    "type":"mcp_tool","server":"cwd","tool":"record","onFailure":"block","input":input
                }]}]));
            }
            tokio::fs::write(
                package.join(".codex-plugin/plugin.json"),
                serde_json::to_vec(&json!({"name":"fixture-codex-cwd","hooks":hooks}))?,
            )
            .await?;
            tokio::fs::write(
                package.join(".mcp.json"),
                serde_json::to_vec(&json!({"mcpServers":{"cwd":{"type":"http","url":server.url("/mcp")}}}))?,
            ).await?;
            case.run_cli([
                "plugin",
                "install",
                package.to_str().context("package path")?,
                "--enable-hooks",
            ])
            .await?
            .require_success()?;
            Ok(())
        })
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            let root = case.workspace().canonicalize()?;
            let nested = root.join("nested");
            let tool = user
                .start_tool("shell.exec", json!({
                    "command":"printf original", "workdir":"nested/../nested", "timeout_ms":5000,
                }))
                .await?;
            tool.wait_permission().await?.allow_once().await?;
            let result = tool.wait_settled().await?;
            result.assert_succeeded()?;
            ensure!(
                result
                    .text()
                    .contains(&format!("rewritten:{}", nested.display())), "{}", result.text()
            );

            {
                let calls = calls.lock().unwrap();
                let expected = [("PreToolUse", "printf original"), ("PermissionRequest", UPDATED_CWD_COMMAND), ("PostToolUse", UPDATED_CWD_COMMAND)];
                ensure!(calls.len() == expected.len(), "{calls:?}");
                for (input, (event, command)) in calls.iter().zip(expected) {
                    ensure!(input["event"] == event && input["cwd"] == json!(nested), "{input}");
                    let tool_input: Value = serde_json::from_str(input["tool_input"].as_str().context("tool input")?)?;
                    ensure!(tool_input == json!({"command":command}), "{input}");
                }
                ensure!(calls[2]["tool_response"] == format!("rewritten:{}", nested.display()), "{:?}", calls[2]);
            }

            user.call_tool(
                "shell.exec",
                json!({"command":"printf invalid","workdir":"\0"}),
            )
            .await?
            .assert_failed()?;
            ensure!(calls.lock().unwrap().len() == 3);
            let blocked = user.call_tool("shell.exec", json!({
                "command": BLOCK_BEFORE_COMMAND, "writes":["blocked.txt"],
            })).await?;
            blocked.assert_failed()?;
            blocked.assert_text_contains("PreToolUse hook failed")?;
            ensure!(!root.join("blocked.txt").exists());

            let withheld = user.start_tool("shell.exec", json!({
                "command": WITHHOLD_AFTER_COMMAND, "writes":["committed.txt"],
            })).await?;
            withheld.wait_permission().await?.allow_once().await?;
            let withheld = withheld.wait_settled().await?;
            withheld.assert_succeeded()?;
            withheld.assert_text_contains("PostToolUse hook failed")?;
            ensure!(!withheld.output.to_string().contains("withheld payload"), "{}", withheld.output);
            ensure!(withheld.output["sources"] == json!([]), "{}", withheld.output);
            ensure!(
                withheld.raw_output.as_ref()
                    .and_then(|raw| raw.payload.as_ref())
                    .and_then(|payload| payload["output"].as_str())
                    .is_some_and(|output| output.contains("withheld payload")),
                "{:?}", withheld.raw_output
            );
            ensure!(tokio::fs::read_to_string(root.join("committed.txt")).await? == "committed");

            let launched = user.start_tool("shell.open", json!({
                "command":"printf terminal-original", "workdir":"nested", "yield_time_ms":5000,
            })).await?;
            launched.wait_permission().await?.allow_once().await?;
            let launched = launched.wait_settled().await?;
            launched.assert_succeeded()?;
            launched.assert_text_contains("terminal-ready")?;
            let launch = launched.data()?;
            ensure!(launch["status"] == "running", "{launch}");
            let process_id = launch["process_id"].as_str().context("terminal process ID")?;
            let launch_hooks = calls.lock().unwrap().len();
            {
                let calls = calls.lock().unwrap();
                ensure!(launch_hooks >= 2, "{calls:?}");
                ensure!(calls[launch_hooks - 2]["event"] == "PreToolUse", "{calls:?}");
                ensure!(calls[launch_hooks - 1]["event"] == "PermissionRequest", "{calls:?}");
            }

            let poll = user.call_tool("shell.read", json!({"process_id":process_id,"wait_ms":0})).await?;
            poll.assert_succeeded()?;
            ensure!(poll.data()?["status"] == "running");
            ensure!(calls.lock().unwrap().len() == launch_hooks);

            let write = user.call_tool("shell.write", json!({
                "process_id":process_id,"chars":"fixture\r","wait_ms":1000,
            })).await?;
            write.assert_succeeded()?;
            let mut exited = write.data()?["status"] != "running";
            for _ in 0..3 {
                if exited {
                    break;
                }
                let read = user.call_tool("shell.read", json!({"process_id":process_id,"since_seq":0,"wait_ms":1000})).await?;
                read.assert_succeeded()?;
                exited = read.data()?["status"] != "running";
            }
            ensure!(exited, "terminal stayed open after receiving input");
            let replay = user.call_tool("shell.read", json!({"process_id":process_id,"since_seq":0,"wait_ms":0})).await?;
            replay.assert_succeeded()?;
            replay.assert_text_contains("terminal-complete:fixture")?;
            ensure!(replay.data()?["status"] == "failed" && replay.data()?["exit_code"] == 7);
            {
                let calls = calls.lock().unwrap();
                ensure!(calls.len() == launch_hooks + 1, "{calls:?}");
                let pre = &calls[launch_hooks - 2];
                let post = &calls[launch_hooks];
                ensure!(post["event"] == "PostToolUse" && post["cwd"] == json!(nested), "{post}");
                ensure!(post["tool_use_id"] == pre["tool_use_id"], "{pre} {post}");
                let args: Value = serde_json::from_str(post["tool_input"].as_str().context("terminal hook input")?)?;
                ensure!(args == json!({"command":TERMINAL_COMMAND}), "{post}");
                ensure!(post["tool_response"].as_str().is_some_and(|output| output.contains("terminal-complete:fixture")), "{post}");
            }

            for (launch_tool, collector, gate, marker, watched) in [
                ("shell.spawn", "shell.logs", "release-spawn", "pipe-spawn-done", false),
                ("shell.watch", "shell.read", "release-watch", "pipe-watch-done", true),
            ] {
                let command = format!(
                    "until [ -f {gate} ]; do sleep 0.05; done; printf '%s\\n' {marker}; printf 'fixture stderr\\n' >&2; exit 9"
                );
                let mut input = json!({
                    "command":command, "workdir":"nested", "reads":[gate], "timeout_ms":90000,
                });
                if watched {
                    input["policy"] = json!({});
                }
                let before_launch = calls.lock().unwrap().len();
                let launch = user.call_tool(launch_tool, input).await?;
                launch.assert_succeeded()?;
                let data = launch.data()?;
                ensure!(data["status"] == "running", "{data}");
                let id = data["process_id"].as_str().context("background process ID")?;
                let poll = user.call_tool(collector, json!({"process_id":id,"wait_ms":0})).await?;
                poll.assert_succeeded()?;
                ensure!(poll.data()?["status"] == "running");
                {
                    let calls = calls.lock().unwrap();
                    ensure!(calls.len() == before_launch + 1, "{calls:?}");
                    ensure!(calls[before_launch]["event"] == "PreToolUse", "{calls:?}");
                }

                let wake = case.model().expect(RequestMatch::new("background command completion", move |request| {
                    request.body["input"].as_array().is_some_and(|items| items.iter().any(|item| {
                        matches!(item["role"].as_str(), Some("system" | "developer"))
                            && item.to_string().contains(marker)
                            && item.to_string().contains("failed (exit 9)")
                    }))
                })).reply(Reply::text("background completion observed"));
                tokio::fs::write(nested.join(gate), b"run").await?;
                wake.observed().await?;
                let result = user.call_tool(collector, json!({"process_id":id,"since_seq":0,"wait_ms":0})).await?;
                result.assert_succeeded()?;
                ensure!(result.data()?["exit_code"] == 9);
                let replay = user.call_tool(collector, json!({"process_id":id,"since_seq":0,"wait_ms":0})).await?;
                replay.assert_succeeded()?;
                replay.assert_text_contains(marker)?;
                {
                    let calls = calls.lock().unwrap();
                    ensure!(calls.len() == before_launch + 2, "{calls:?}");
                    let pre = &calls[before_launch];
                    let post = &calls[before_launch + 1];
                    ensure!(post["event"] == "PostToolUse" && post["cwd"] == json!(nested), "{post}");
                    ensure!(post["tool_use_id"] == pre["tool_use_id"], "{pre} {post}");
                    let args: Value = serde_json::from_str(post["tool_input"].as_str().context("background hook input")?)?;
                    ensure!(args == json!({"command":command}), "{post}");
                    let output = post["tool_response"].as_str().context("background hook output")?;
                    ensure!(output.lines().any(|line| line == marker), "{post}");
                    ensure!(output.lines().any(|line| line == "fixture stderr"), "{post}");
                }
            }

            case.model().expect(RequestMatch::new("required PostCompact check", |request| {
                request.body.to_string().contains("Summarize the conversation for the next assistant continuation")
            })).reply(Reply::text("Codex required hook checkpoint"));
            let compact = user.client().compact_session(user.id(), Default::default()).await?;
            let compact = user.attach_run(compact.receipt.context("manual compaction receipt")?).wait_settled().await?;
            compact.assert_succeeded()?;
            ensure!(compact.parts.iter().any(|part| part.kind == "hook"
                && part.content["detail"].as_str().is_some_and(|reason| reason.contains("PostCompact hook failed"))));
            let calls = calls.lock().unwrap();
            ensure!(calls.last().is_some_and(|call| call["event"] == "PostCompact"), "{calls:?}");
            Ok(())
        })
        .await
}

#[derive(Default)]
struct HookPeer {
    calls: Mutex<Vec<(String, i64)>>,
}

async fn rpc(State(peer): State<Arc<HookPeer>>, Json(request): Json<Value>) -> Response {
    let mut result = match request["method"].as_str() {
        Some("server/discover") => json!({
            "supportedVersions":["2026-07-28"],
            "capabilities":{"tools":{}},
            "ttlMs":0,
            "cacheScope":"private"
        }),
        Some("tools/list") => json!({
            "tools":[{"name":"check","inputSchema":{"type":"object"},
                "annotations":{"readOnlyHint":true}}]
        }),
        Some("tools/call") if request["params"]["name"] == "check" => {
            let args = &request["params"]["arguments"];
            let Some(source) = args["source"].as_str() else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            let Some(session) = args["session"]
                .as_str()
                .and_then(|id| id.parse::<i64>().ok())
            else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            peer.calls
                .lock()
                .unwrap()
                .push((source.to_owned(), session));
            let reply = json!({
                "continue":false,
                "stopReason":format!("fixture stopped {source}"),
                "hookSpecificOutput":{
                    "hookEventName":"SessionStart",
                    "additionalContext":format!("fixture context from {source}")
                }
            });
            json!({"content":[{"type":"text","text":reply.to_string()}]})
        }
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    result["resultType"] = json!("complete");
    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result})).into_response()
}

async fn install(case: TestCase, peer: Arc<HookPeer>) -> TestResult {
    let server = case
        .serve_http_on(
            super::super::native_fixture_address()?,
            Router::new().route("/mcp", post(rpc)).with_state(peer),
        )
        .await?;
    let package = case.paths().root.join("codex-session-start");
    tokio::fs::create_dir(case.paths().root.join("empty-bin")).await?;
    tokio::fs::create_dir_all(package.join(".codex-plugin")).await?;
    tokio::fs::write(
        package.join(".codex-plugin/plugin.json"),
        serde_json::to_vec(&json!({
            "name":"fixture-codex-start",
            "hooks":{"SessionStart":[{"hooks":[{
                "type":"mcp_tool","server":"gate","tool":"check",
                "input":{"source":"${source}","session":"${session_id}"}
            }]}]}
        }))?,
    )
    .await?;
    tokio::fs::write(
        package.join(".mcp.json"),
        serde_json::to_vec(&json!({
            "mcpServers":{"gate":{"type":"http","url":server.url("/mcp")}}
        }))?,
    )
    .await?;
    case.run_cli([
        "plugin",
        "install",
        package.to_str().context("fixture package path")?,
        "--enable-hooks",
    ])
    .await?
    .require_success()?;
    Ok(())
}

async fn assert_stops_once(case: &TestCase, user: &UserSession, source: &str) -> TestResult {
    let blocked_prompt = format!("prompt stopped at {source}");
    let blocked = user.send(&blocked_prompt).await?;
    blocked.wait_settled().await?.assert_cancelled()?;
    let reason = format!("fixture stopped {source}");
    ensure!(
        user.parts().await?.iter().any(|part| {
            part.kind == "hook"
                && part.message_id == Some(blocked.receipt().message_id)
                && part.content["detail"] == reason
        }),
        "the stopped {source} turn has no SessionStart reason"
    );

    let followup = format!("prompt after {source}");
    let context = format!("fixture context from {source}");
    case.model()
        .expect(RequestMatch::user_text(&followup).streaming(true))
        .check(move |request| {
            let input = request.body.to_string();
            ensure!(input.contains(&context), "SessionStart context was lost");
            ensure!(
                !input.contains("prompt stopped at "),
                "a stopped turn entered the model prompt"
            );
            ensure!(
                !input.contains(&reason),
                "the display-only stop reason entered the prompt"
            );
            Ok(())
        })
        .reply(Reply::text("follow-up ran"));
    user.send(followup)
        .await?
        .wait_settled()
        .await?
        .assert_succeeded()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_codex_session_start_stop_applies_once_per_lifecycle() -> TestResult {
    let peer = Arc::new(HookPeer::default());
    let setup = Arc::clone(&peer);
    let network = super::super::native_fixture_network()?;
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .builtin_plugin(
            "agena.mcp",
            json!({"runtime":{"reconnect":{"enabled":false}}}),
        )
        .configure_global(move |config| {
            config["permission"]["network"] = network.clone();
            config["permission"]["tools"]["names"]["agena.mcp.tools.call"] = json!("allow");
            config["session"]["auto_title"]["enabled"] = json!(true);
        })
        .before_start(move |case| install(case, setup))
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            assert_stops_once(&case, &user, "startup").await?;

            let response = user
                .client()
                .command(Command::ForkSession(ForkSessionParams {
                    session_id: user.id(),
                    at_message_id: None,
                    title: Some("Codex fork fixture".into()),
                    conversation_mode: None,
                    expected_version: None,
                }))
                .await?;
            let CommandResult::Execution(fork) = response else {
                anyhow::bail!("unexpected fork response: {response:?}");
            };
            let fork = case.user().attach_session(fork.session.id).await?;
            assert_stops_once(&case, &fork, "fork").await?;

            user.client().clear_session(user.id(), false).await?;
            assert_stops_once(&case, &user, "clear").await?;

            case.model()
                .expect(RequestMatch::new("manual compaction", |request| {
                    request
                        .body
                        .to_string()
                        .contains("Summarize the conversation for the next assistant continuation")
                }))
                .check(|request| {
                    ensure!(
                        !request.body.to_string().contains("prompt stopped at clear"),
                        "the stopped prompt entered the compaction"
                    );
                    Ok(())
                })
                .reply(Reply::text("Codex compaction checkpoint"));
            let compact = user
                .client()
                .compact_session(user.id(), RunOptions::default())
                .await?;
            user.attach_run(compact.receipt.context("manual compaction receipt")?)
                .wait_settled()
                .await?
                .assert_succeeded()?;
            assert_stops_once(&case, &user, "compact").await?;

            case.restart().await?;
            let user = case.user().attach_session(user.id()).await?;
            assert_stops_once(&case, &user, "resume").await?;

            let titled = case
                .user()
                .new_session_with(SessionOptions {
                    title: String::new(),
                    auto_title: true,
                    ..Default::default()
                })
                .await?;
            case.model()
                .expect(RequestMatch::new("automatic title", |request| {
                    request
                        .body
                        .to_string()
                        .contains("You maintain a short, recognizable name")
                }))
                .check(|request| {
                    let input = request.body.to_string();
                    ensure!(
                        input.contains("prompt after startup"),
                        "the allowed prompt is missing"
                    );
                    ensure!(
                        !input.contains("prompt stopped at startup"),
                        "the stopped prompt entered the title"
                    );
                    Ok(())
                })
                .reply(Reply::text(
                    r#"{"action":"rename","title":"Allowed startup request"}"#,
                ));
            assert_stops_once(&case, &titled, "startup").await?;
            titled
                .wait_state("automatic title based on the allowed request", |state| {
                    state.session.title == "Allowed startup request"
                })
                .await?;

            assert_eq!(
                *peer.calls.lock().unwrap(),
                [
                    ("startup".into(), user.id()),
                    ("fork".into(), fork.id()),
                    ("clear".into(), user.id()),
                    ("compact".into(), user.id()),
                    ("resume".into(), user.id()),
                    ("startup".into(), titled.id()),
                ]
            );
            Ok(())
        })
        .await
}
