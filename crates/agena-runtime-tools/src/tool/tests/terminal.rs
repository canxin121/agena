//! Exercise the actual executor-backed tool route, not only the PTY registry.
use super::*;
use crate::part::{MonitorToolInput, ShellToolInput};
use crate::tool::{ToolPayloadExecution, ToolRuntimeContext, monitor_tool, process_tool};
use agena_domain::{PermissionDecision, ProcessStatus};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

async fn fixture(policy: ToolPermissionPolicy) -> (tempfile::TempDir, ToolExecutor) {
    let directory = tempfile::tempdir().unwrap();
    let workspace = std::fs::canonicalize(directory.path()).unwrap();
    let mut config = PluginsConfig::default();
    config
        .list
        .insert("agena.shell".into(), ConfiguredPlugin::static_default());
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![StaticPluginRegistration::new(
            "agena.shell".parse().unwrap(),
            ExecutorBackedShellAdapter,
        )],
        config,
        workspace_root: workspace.clone(),
        agena_version: "terminal-test".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .unwrap();
    let executor = ToolExecutor::new(
        workspace,
        ExecutionPrincipal::new(PermissionPolicy::allow_all(), policy),
        plugins,
        None,
        None,
        None,
    );
    (directory, executor)
}

fn invoke(name: &str, input: Value) -> ToolInvocation {
    ToolInvocation::new(name, StructuredObject::try_from(input).unwrap())
}

fn python(script: &str) -> Value {
    json!({
        "command": format!("exec /usr/bin/python3 -u -c '{}'", script.replace('\'', "'\"'\"'")),
        "tty": true, "yield_time_ms": 1000, "timeout_ms": 10000,
        "reads": [], "writes": [], "network": []
    })
}

async fn process(
    executor: &ToolExecutor,
    input: Value,
    session: i64,
    call: i64,
) -> Result<ToolPayloadExecution, ToolError> {
    process_tool::execute_async(
        executor,
        &serde_json::from_value::<ShellToolInput>(input).unwrap(),
        ToolRuntimeContext {
            session_id: Some(session),
            call_id: Some(call),
            ..Default::default()
        },
    )
    .await
}

async fn settle(executor: &ToolExecutor, id: &str, session: i64) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let result = process(
                executor,
                json!({"action":"logs", "process_id":id, "wait_ms":100}),
                session,
                90,
            )
            .await
            .unwrap();
            let value = serde_json::to_value(result.output).unwrap();
            if value["status"] != "running" {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

async fn observe(executor: &ToolExecutor, id: &str, session: i64, expected: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let result = process(
                executor,
                json!({"action":"logs", "process_id":id, "since_seq":0, "wait_ms":100}),
                session,
                91,
            )
            .await
            .unwrap();
            let value = serde_json::to_value(result.output).unwrap();
            if value["output"]
                .as_str()
                .unwrap_or_default()
                .contains(expected)
            {
                return value;
            }
            assert_eq!(
                value["status"], "running",
                "terminal exited before {expected:?}: {value}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_tool_route_preserves_session_input_screen_and_legacy_behavior() {
    let (_dir, executor) = fixture(ToolPermissionPolicy::allow_all()).await;
    let command = python(
        "import sys\nprint('TTY='+str(sys.stdin.isatty()),flush=True)\nwhile True:\n try: value=input('PROMPT> ')\n except EOFError: break\n print('VALUE='+repr(value),flush=True)",
    );
    let started = executor
        .execute_invocation_detailed(&invoke("shell.run", command), 41, 1)
        .await
        .unwrap();
    let result = Value::from(started.output.payload.clone());
    let id = result["process_id"].as_str().unwrap();
    assert_eq!(result["terminal"]["rows"], 24);
    assert_eq!(result["background"], true);
    assert!(
        !started
            .view
            .output_text
            .contains("plugin adapter must not execute")
    );
    for (call, name, chars) in [(2, "shell.write", " 你好 "), (3, "agena.shell.write", "\r")] {
        let input = invoke(
            name,
            json!({"process_id":id,"chars":chars,"wait_ms":1000,
            "reads":[],"writes":[],"network":[]}),
        );
        let prepared = executor.prepare_invocation(&input, 41, call).await.unwrap();
        let written = executor
            .execute_invocation_detailed(&prepared.invocation, 41, call)
            .await
            .unwrap();
        assert!(
            !written
                .view
                .output_text
                .contains("plugin adapter must not execute")
        );
        let output = Value::from(written.output.payload);
        assert_eq!(output["action"], "write");
        assert_eq!(output["process_id"], id);
        if call == 2 {
            assert!(
                !output["output"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("VALUE=")
            );
        }
    }
    let output = observe(&executor, id, 41, "VALUE=' 你好 '").await;
    assert!(
        output["terminal"]["text"]
            .as_str()
            .unwrap()
            .contains("TTY=True")
    );
    let resized = process(
        &executor,
        json!({"action":"resize","process_id":id,"rows":35,"cols":100}),
        41,
        5,
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(resized.output).unwrap()["terminal"]["cols"],
        100
    );
    executor
        .execute_invocation_detailed(
            &invoke(
                "shell.write",
                json!({
                    "process_id":id,"chars":"\u{4}","reads":[],"writes":[],"network":[]
                }),
            ),
            41,
            6,
        )
        .await
        .unwrap();
    assert_eq!(settle(&executor, id, 41).await["exit_code"], 0);

    let plain = executor
        .execute_invocation_detailed(
            &invoke(
                "shell.run",
                json!({
                    "command":"printf legacy","reads":[],"writes":[],"network":[]
                }),
            ),
            41,
            7,
        )
        .await
        .unwrap();
    let plain = Value::from(plain.output.payload);
    assert_eq!(plain["background"], false);
    assert!(plain["process_id"].is_null());
    assert!(plain["terminal"].is_null());
    assert_eq!(plain["output"], "legacy");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_sessions_cannot_use_monitor_stop_alias_or_discover_terminals() {
    let (_dir, executor) = fixture(ToolPermissionPolicy::allow_all()).await;
    let started = executor
        .execute_invocation_detailed(&invoke("shell.run", python("input('OWNED> ')")), 41, 1)
        .await
        .unwrap();
    let value = Value::from(started.output.payload);
    let id = value["process_id"].as_str().unwrap();
    let listed = process(&executor, json!({"action":"list"}), 42, 2)
        .await
        .unwrap();
    assert!(matches!(listed.output,
        crate::tool::ToolPayloadOutput::Shell { processes, .. } if processes.is_empty()
    ));
    assert!(
        process(&executor, json!({"action":"logs","process_id":id}), 42, 3)
            .await
            .is_err()
    );
    assert!(
        monitor_tool::execute_async(
            &executor,
            &MonitorToolInput::Stop {
                monitor_id: id.into()
            },
            ToolRuntimeContext {
                session_id: Some(42),
                ..Default::default()
            },
        )
        .await
        .is_err()
    );
    let registry = executor.monitor_registry().unwrap();
    assert_eq!(registry.list()[0].status, ProcessStatus::Running);
    process(&executor, json!({"action":"stop","process_id":id}), 41, 4)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prefix_allow_rule_cannot_override_terminal_write_denial() {
    let mut policy = ToolPermissionPolicy::new(PermissionMode::Deny);
    policy.add_bash_overlay_rule("echo *", PermissionMode::Allow);
    let (_dir, executor) = fixture(policy).await;
    let run = invoke(
        "shell.run",
        json!({"command":"echo safe","reads":[],"writes":[],"network":[]}),
    );
    assert!(matches!(
        executor.authorize_invocation(&run).unwrap().1,
        PermissionDecision::Allow
    ));
    let write = invoke(
        "shell.write",
        json!({"process_id":"pty","chars":"echo safe\r",
        "reads":[],"writes":[],"network":[]}),
    );
    assert!(matches!(
        executor.authorize_invocation(&write).unwrap().1,
        PermissionDecision::Deny { .. }
    ));
    assert_eq!(
        super::super::output_helpers::shell_command_from_invocation(&write).as_deref(),
        Some("echo safe\r")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn successful_launch_survives_cancellation_of_the_original_turn() {
    let (_dir, executor) = fixture(ToolPermissionPolicy::allow_all()).await;
    let cancellation = CancellationToken::new();
    let launch_executor = executor
        .clone()
        .with_cancellation_token(Some(cancellation.clone()));
    let started = launch_executor
        .execute_invocation_detailed(
            &invoke(
                "shell.run",
                python("value=input('READY> ');print('VALUE='+value,flush=True)"),
            ),
            41,
            1,
        )
        .await
        .unwrap();
    let value = Value::from(started.output.payload);
    let id = value["process_id"].as_str().unwrap();
    cancellation.cancel();
    let written = executor.execute_invocation_detailed(&invoke("shell.write",json!({
        "process_id":id,"chars":"next-turn\r","wait_ms":1000,"reads":[],"writes":[],"network":[]
    })),41,2).await.unwrap();
    assert!(written.view.output_text.contains("next-turn"));
    assert_eq!(settle(&executor, id, 41).await["exit_code"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_inflight_launch_cleans_child_and_returns_cancelled() {
    let (_dir, executor) = fixture(ToolPermissionPolicy::allow_all()).await;
    let cancellation = CancellationToken::new();
    let launching = executor
        .clone()
        .with_cancellation_token(Some(cancellation.clone()));
    let mut input = python("import time;time.sleep(30)");
    input["yield_time_ms"] = json!(30000);
    let task = tokio::spawn(async move {
        launching
            .execute_invocation_detailed(&invoke("shell.run", input), 41, 1)
            .await
    });
    let registry = executor.monitor_registry().unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while registry.list().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    cancellation.cancel();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap(),
        Err(ToolError::Cancelled)
    ));
    let id = registry.list()[0].process_id.clone();
    assert_eq!(settle(&executor, &id, 41).await["status"], "stopped");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_inflight_launch_does_not_leave_child_running() {
    let (_dir, executor) = fixture(ToolPermissionPolicy::allow_all()).await;
    let launching = executor.clone();
    let mut input = python("import time;time.sleep(30)");
    input["yield_time_ms"] = json!(30000);
    let task = tokio::spawn(async move {
        launching
            .execute_invocation_detailed(&invoke("shell.run", input), 41, 1)
            .await
    });
    let registry = executor.monitor_registry().unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while registry.list().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let id = registry.list()[0].process_id.clone();
    assert_eq!(settle(&executor, &id, 41).await["status"], "stopped");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incompatible_monitor_is_rejected_without_starting_a_pty() {
    let (_dir, executor) = fixture(ToolPermissionPolicy::allow_all()).await;
    let mut input = python("input('must not start')");
    input["monitor"] = json!({"persistent":true});
    input["action"] = json!("run");
    assert!(process(&executor, input, 41, 1).await.is_err());
    assert!(executor.monitor_registry().unwrap().list().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_input_keeps_explicit_shell_denials() {
    let mut policy = ToolPermissionPolicy::allow_all();
    policy.add_bash_overlay_rule("rm *", PermissionMode::Deny);
    let (_dir, executor) = fixture(policy).await;
    let input = invoke(
        "shell.write",
        json!({"process_id":"pty", "chars":"rm protected.txt\r",
        "reads":[], "writes":["protected.txt"], "network":[]}),
    );
    assert!(matches!(
        executor.authorize_invocation(&input).unwrap().1,
        PermissionDecision::Deny { .. }
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_processes_and_monitor_aliases_are_owner_scoped() {
    let (_dir, executor) = fixture(ToolPermissionPolicy::allow_all()).await;
    let started=process(&executor,json!({"action":"run","command":"printf 'OWNED\\n'; exec sleep 30","run_in_background":true,"reads":[],"writes":[],"network":[]}),41,100).await.unwrap();
    let value = serde_json::to_value(started.output).unwrap();
    let id = value["process_id"].as_str().unwrap();
    let foreign = process(&executor, json!({"action":"list"}), 42, 101)
        .await
        .unwrap();
    let list = serde_json::to_value(foreign.output).unwrap();
    assert!(list["processes"].as_array().is_none_or(|v| v.is_empty()));
    assert!(
        process(&executor, json!({"action":"logs","process_id":id}), 42, 102)
            .await
            .is_err()
    );
    assert!(
        process(&executor, json!({"action":"stop","process_id":id}), 42, 103)
            .await
            .is_err()
    );
    assert!(
        monitor_tool::execute_async(
            &executor,
            &MonitorToolInput::Stop {
                monitor_id: id.into()
            },
            ToolRuntimeContext {
                session_id: Some(42),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert!(
        process(&executor, json!({"action":"logs","process_id":id}), 41, 104)
            .await
            .is_ok()
    );
    process(&executor, json!({"action":"stop","process_id":id}), 41, 105)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_cannot_change_an_existing_process_launch() {
    let (_dir, executor) = fixture(ToolPermissionPolicy::allow_all()).await;
    let input = json!({"action":"run","command":"exec sleep 30","run_in_background":true,"reads":[],"writes":[],"network":[]});
    let started = process(&executor, input.clone(), 41, 106).await.unwrap();
    let value = serde_json::to_value(started.output).unwrap();
    let id = value["process_id"].as_str().unwrap();
    process(&executor, input, 41, 106).await.unwrap();
    assert!(process(&executor,json!({"action":"run","command":"printf 'do-not-run'","run_in_background":true,"reads":[],"writes":[],"network":[]}),41,106).await.is_err());
    process(&executor, json!({"action":"stop","process_id":id}), 41, 107)
        .await
        .unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn required_sandbox_wraps_foreground_background_and_pty_tool_paths() {
    let marker = "AGENA_SANDBOX_TOOL_TEST_CHILD";
    if std::env::var_os(marker).is_none() {
        let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","tool::tests::terminal::required_sandbox_wraps_foreground_background_and_pty_tool_paths","--nocapture"])
            .env(marker,"1").env("AGENA_SHELL_SANDBOX","offline").output().unwrap();
        assert!(
            output.status.success(),
            "{}\\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let (_dir, executor) = fixture(ToolPermissionPolicy::allow_all()).await;
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("forbidden.txt");
    let command = format!("printf forbidden > '{}'", target.display());
    let mut input = serde_json::json!({"action":"run","command":command,"reads":["."],"writes":[],"network":[]});
    let foreground = process(&executor, input.clone(), 41, 111).await.unwrap();
    assert_ne!(
        serde_json::to_value(foreground.output).unwrap()["exit_code"],
        0
    );
    assert!(!target.exists());
    input["run_in_background"] = serde_json::json!(true);
    let started = process(&executor, input.clone(), 41, 112).await.unwrap();
    let id = serde_json::to_value(started.output).unwrap()["process_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let result = settle(&executor, &id, 41).await;
    assert_ne!(result["exit_code"], 0);
    assert!(!target.exists());
    input["tty"] = serde_json::json!(true);
    input["yield_time_ms"] = serde_json::json!(1000);
    let started = process(&executor, input, 41, 113).await.unwrap();
    let id = serde_json::to_value(started.output).unwrap()["process_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let result = settle(&executor, &id, 41).await;
    assert_ne!(result["exit_code"], 0);
    assert!(!target.exists());
}
