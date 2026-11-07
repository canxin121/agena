//! MCP connection management through real model calls, permission prompts and reloads.
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{allow_permissions, fixture};
use agena_testkit::{
    ConfigSnapshot, Failure, ModelFeedbackKind, Reply, RequestMatch, TestAgenaBuilder, TestCase,
    TestResult, ToolResult, UserSession,
};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

const TOKEN: &str = "stdio-create-private-token";

fn stdio(root: &Path) -> Value {
    json!({
        "transport": "stdio",
        "process": {
            "command": "python3",
            "args": ["-u", root.join("mcp-stdio-peer.py"), root.join("mcp-stdio-record.jsonl"),
                "two words", "$(literal)"],
            "cwd": "services",
            "env": {"TOKEN": TOKEN}
        },
        "timeouts": {"connect_ms": 10000, "request_ms": 10000}
    })
}

fn workspace_document(servers: Value) -> Value {
    json!({"plugins": {"list": {"agena.mcp": {
        "enabled": true, "package": {"kind": "static"}, "settings": {"servers": servers}
    }}}})
}

fn connection_fixture() -> TestAgenaBuilder {
    fixture()
        .builtin_plugin(
            "agena.mcp",
            json!({"runtime": {"reconnect": {"enabled": false}}}),
        )
        .workspace_config(json!({}))
        .expect_plugin("agena.mcp")
        .before_start(|case| async move {
            tokio::fs::create_dir_all(case.workspace().join("services")).await?;
            tokio::fs::write(
                case.paths().root.join("mcp-stdio-peer.py"),
                include_str!("stdio_peer.py"),
            )
            .await?;
            Ok(())
        })
}

async fn user(case: &TestCase) -> TestResult<UserSession> {
    let user = case.user().new_session().await?;
    allow_permissions(&user).await?;
    Ok(user)
}

async fn configure_policy(
    user: &UserSession,
    paths: &[(&Path, &str, &str)],
    spawn: &str,
) -> TestResult {
    let mut policy = serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
    let mut rules = serde_json::Map::new();
    for (path, read, write) in paths {
        rules.insert(
            path.to_string_lossy().into_owned(),
            json!({"read": read, "write": write}),
        );
    }
    policy["path"]["rules"] = Value::Object(rules);
    // The helper's explicit per-name allows must be replaced as well as defaults.
    policy["tools"]["names"]["shell.spawn"] = json!(spawn);
    policy["tools"]["names"]["agena.shell.spawn"] = json!(spawn);
    user.client()
        .set_session_permission(user.id(), serde_json::from_value(policy)?)
        .await?;
    Ok(())
}

async fn bytes(case: &TestCase) -> TestResult<(Vec<u8>, Vec<u8>)> {
    Ok((
        tokio::fs::read(&case.paths().global_config).await?,
        tokio::fs::read(&case.paths().workspace_config).await?,
    ))
}

async fn workspace(case: &TestCase) -> TestResult<Value> {
    Ok(serde_json::from_slice(
        &tokio::fs::read(&case.paths().workspace_config).await?,
    )?)
}

fn redacted(result: &ToolResult, secrets: &[&str], input_contains_secrets: bool) -> TestResult {
    let mut envelope = result.output.clone();
    if input_contains_secrets {
        // The protocol legitimately echoes the model's own mutation arguments.
        // Exclude only that input field; inspect every actual output/failure
        // field, model projection and durable raw output for secret disclosure.
        if let Some(invocation) = envelope
            .pointer_mut("/result/invocation")
            .and_then(Value::as_object_mut)
        {
            invocation.remove("input");
        }
    }
    let channels = [
        serde_json::to_string(&envelope)?,
        serde_json::to_string(&result.raw_output)?,
        result.text(),
    ];
    for secret in secrets {
        ensure!(
            channels.iter().all(|channel| !channel.contains(secret)),
            "MCP output exposed a configured secret in one of its channels"
        );
    }
    Ok(())
}

async fn wait_reload(case: &TestCase, result: &ToolResult, before: &ConfigSnapshot) -> TestResult {
    let payload = result.data()?;
    ensure!(
        payload["saved"] == true,
        "mutation was not persisted: {payload}"
    );
    let task_id = payload["reload_task_id"]
        .as_str()
        .context("saved MCP mutation omitted its reload receipt")?;
    case.settings()
        .with_timeout(Duration::from_secs(30))
        .track_reload(task_id, before)
        .wait()
        .await?;
    Ok(())
}

#[cfg(unix)]
async fn assert_peers_exited(record_path: &Path) -> TestResult {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let transcript = tokio::fs::read_to_string(record_path).await?;
        let records: Vec<Value> = transcript
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        let mut live = Vec::new();
        for started in records.iter().filter(|item| item["event"] == "started") {
            let pid = i32::try_from(
                started["pid"]
                    .as_u64()
                    .context("invalid owned MCP peer PID")?,
            )?;
            // SAFETY: signal 0 observes process existence only; it sends no
            // signal. This PID is recorded by the fixture's own child process.
            let exists = unsafe { libc::kill(pid as libc::pid_t, 0) };
            if exists == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                continue;
            }
            #[cfg(target_os = "linux")]
            {
                let current = match tokio::fs::read_to_string(format!("/proc/{pid}/stat")).await {
                    Ok(current) => current,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                };
                let current_start = current
                    .rsplit_once(')')
                    .context("invalid process stat")?
                    .1
                    .split_whitespace()
                    .nth(19)
                    .context("missing process starttime")?;
                if current_start
                    != started["starttime"]
                        .as_str()
                        .context("owned peer did not record process identity")?
                {
                    continue; // The owned process exited and the numeric PID was reused.
                }
            }
            live.push(pid);
        }
        if live.is_empty() {
            ensure!(
                !records
                    .iter()
                    .any(|item| item["event"] == "request-budget-exceeded"),
                "the product exhausted the stdio fixture's 256-request budget"
            );
            return Ok(());
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "deleting the MCP configuration left retired stdio peers alive: {live:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reads_require_both_config_paths_and_return_redacted_values_in_every_output_channel()
-> TestResult {
    let remote = json!({
        "transport": "http", "enabled": false,
        "endpoint": {
            "url": "https://example.invalid/mcp?key=private-url",
            "headers": {"X-Key": "private-header"}
        },
        "auth": {"kind": "bearer", "token": "private-bearer"}
    });
    connection_fixture()
        .configure_global(move |config| {
            config["plugins"]["list"]["agena.mcp"]["settings"]["servers"] =
                json!({"remote": remote});
        })
        .run(|case| async move {
            let user = user(&case).await?;
            let list = user.call_tool("mcp.connections.list", json!({})).await?;
            list.assert_succeeded()?;
            redacted(
                &list,
                &["private-url", "private-header", "private-bearer"],
                false,
            )?;
            let catalog = list.data()?;
            ensure!(
                catalog["workspace_root"] == case.workspace().to_string_lossy().as_ref(),
                "{catalog}"
            );
            ensure!(
                catalog["global_path"] == case.paths().global_config.to_string_lossy().as_ref()
            );
            ensure!(
                catalog["workspace_path"]
                    == case.paths().workspace_config.to_string_lossy().as_ref()
            );
            let summary = catalog["connections"]
                .as_array()
                .context("connection catalog")?
                .iter()
                .find(|item| item["name"] == "remote")
                .context("disabled HTTP record")?;
            let id = summary["id"]
                .as_str()
                .context("opaque connection id")?
                .to_owned();
            let get = user
                .call_tool("mcp.connections.get", json!({"id": id}))
                .await?;
            get.assert_succeeded()?;
            redacted(
                &get,
                &["private-url", "private-header", "private-bearer"],
                false,
            )?;
            ensure!(get.data()?["config"]["auth"]["token"] == "[REDACTED]");
            ensure!(
                get.data()?["secret_paths"]
                    .as_array()
                    .context("redacted paths")?
                    .len()
                    == 3
            );

            let before = bytes(&case).await?;
            for denied in [&case.paths().global_config, &case.paths().workspace_config] {
                configure_policy(&user, &[(denied, "deny", "allow")], "allow").await?;
                for (tool, input) in [
                    ("mcp.connections.list", json!({})),
                    ("mcp.connections.get", json!({"id": id})),
                ] {
                    user.call_tool(tool, input)
                        .await?
                        .assert_failure_kind(ModelFeedbackKind::PermissionDenied)?;
                }
                ensure!(bytes(&case).await? == before);
            }
            ensure!(!case.paths().root.join("mcp-stdio-record.jsonl").exists());
            Ok(())
        })
        .await
}

// Restore the owned source even if a model/permission assertion fails.
struct RestoreFile {
    path: PathBuf,
    original: Option<Vec<u8>>,
}
impl RestoreFile {
    fn restore(mut self) -> TestResult {
        std::fs::write(
            &self.path,
            self.original.as_ref().context("already restored")?,
        )?;
        self.original = None;
        Ok(())
    }
}
impl Drop for RestoreFile {
    fn drop(&mut self) {
        if let Some(bytes) = self.original.take() {
            let _ = std::fs::write(&self.path, bytes);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn denied_config_reads_fail_before_parsing_files_and_preserve_policy_error() -> TestResult {
    connection_fixture()
        .run(|case| async move {
            let user = user(&case).await?;
            for (index, path) in [&case.paths().global_config, &case.paths().workspace_config]
                .into_iter()
                .enumerate()
            {
                configure_policy(&user, &[(path, "deny", "allow")], "allow").await?;
                for (tool_index, (tool, input)) in [
                    ("mcp.connections.list", json!({})),
                    (
                        "mcp.connections.get",
                        json!({"id": "not-needed-before-permission"}),
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let marker = format!("mcp-read-before-parse-{index}-{tool_index}");
                    let call_id = format!("call_read_before_parse_{index}_{tool_index}");
                    let step = case
                        .model()
                        .expect(RequestMatch::user_text(marker.clone()))
                        .reply(Reply::tool_call(call_id.clone(), tool, input).held());
                    let check_id = call_id.clone();
                    case.model()
                        .expect(RequestMatch::tool_output(call_id))
                        .after(&step)
                        .check(move |request| {
                            let output: Value = serde_json::from_str(
                                &request
                                    .tool_output_text(&check_id)
                                    .context("denied tool result")?,
                            )?;
                            ensure!(output["phase"] == "failed", "{output}");
                            let failure: Failure =
                                serde_json::from_value(output["result"]["failure"].clone())?;
                            ensure!(
                                failure.model.as_ref().context("model feedback")?.kind
                                    == ModelFeedbackKind::PermissionDenied,
                                "the denied source was parsed before permission: {failure:?}"
                            );
                            Ok(())
                        })
                        .reply(Reply::text("read-denied-before-parsing"));
                    let run = user.send(marker).await?;
                    step.observed().await?;
                    let original = tokio::fs::read(path).await?;
                    let restore = RestoreFile {
                        path: path.to_path_buf(),
                        original: Some(original),
                    };
                    tokio::fs::write(path, b"invalid JSON must not be parsed by the denied tool")
                        .await?;
                    step.release();
                    run.wait_settled()
                        .await?
                        .assert_assistant_text("read-denied-before-parsing")?;
                    ensure!(
                        tokio::fs::read(path).await?
                            == b"invalid JSON must not be parsed by the denied tool"
                    );
                    restore.restore()?;
                }
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn denied_writes_and_declined_process_approval_leave_connections_unchanged() -> TestResult {
    connection_fixture().run(|case| async move {
        let user = user(&case).await?;
        let input = json!({"name": "blocked", "scope": "workspace", "config": stdio(&case.paths().root)});
        let before = bytes(&case).await?;
        configure_policy(&user, &[(&case.paths().workspace_config, "allow", "deny")], "ask").await?;
        user.call_tool("mcp.connections.create", input.clone()).await?
            .assert_failure_kind(ModelFeedbackKind::PermissionDenied)?;
        ensure!(bytes(&case).await? == before, "write denial changed configuration");
        ensure!(!case.paths().root.join("mcp-stdio-record.jsonl").exists(), "write denial started a process");

        configure_policy(&user, &[], "ask").await?;
        let create = user.start_tool("mcp.connections.create", input).await?;
        let approval = create.run().wait_permission().await?;
        ensure!(bytes(&case).await? == before, "process approval was requested after persistence");
        ensure!(!case.paths().root.join("mcp-stdio-record.jsonl").exists(), "process ran before approval");
        approval.deny_once().await?;
        create.wait_settled().await?.assert_failure_kind(ModelFeedbackKind::UserDeclined)?;
        ensure!(bytes(&case).await? == before, "declined process authorization changed configuration");
        ensure!(!case.paths().root.join("mcp-stdio-record.jsonl").exists(), "declined process authorization started the peer");
        let listed = user.call_tool("mcp.connections.list", json!({})).await?;
        listed.assert_succeeded()?;
        ensure!(listed.data()?["connections"].as_array().context("connections")?.is_empty());
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_mutation_authorizes_then_persists_reloads_and_runs_the_real_stdio_peer()
-> TestResult {
    connection_fixture()
        .run(|case| async move {
            let user = user(&case).await?;
            configure_policy(&user, &[], "ask").await?;
            let before = bytes(&case).await?;
            let before_reload = case.config_snapshot().await?;
            let created = user
                .start_tool(
                    "mcp.connections.create",
                    json!({
                        "name": "created", "scope": "workspace", "config": stdio(&case.paths().root)
                    }),
                )
                .await?;
            let approval = created.run().wait_permission().await?;
            ensure!(
                bytes(&case).await? == before,
                "configuration was saved before process approval"
            );
            ensure!(
                !case.paths().root.join("mcp-stdio-record.jsonl").exists(),
                "peer started before approval"
            );
            approval.allow_once().await?;
            let created = created.wait_settled().await?;
            created.assert_succeeded()?;
            redacted(&created, &[TOKEN], true)?;
            let payload = created.data()?;
            let id = payload["connection"]["id"]
                .as_str()
                .context("created id")?
                .to_owned();
            ensure!(
                !payload["connection"]["revision"]
                    .as_str()
                    .context("created revision")?
                    .is_empty()
            );
            ensure!(
                payload["warning"]
                    .as_str()
                    .context("activation warning")?
                    .contains("queued")
            );
            ensure!(
                payload["connection"]["state"] == "pending_reload",
                "{payload}"
            );
            let stored = workspace(&case).await?;
            ensure!(
                stored["plugins"]["list"]["agena.mcp"]["settings"]["servers"]["created"]["process"]
                    ["env"]["TOKEN"]
                    == TOKEN
            );
            ensure!(
                tokio::fs::read(&case.paths().global_config).await? == before.0,
                "workspace create rewrote global config"
            );
            wait_reload(&case, &created, &before_reload).await?;

            configure_policy(&user, &[], "allow").await?;
            let search = user
                .call_tool(
                    "mcp.tools.search",
                    json!({"query": "stdio-owned-echo", "limit": 10}),
                )
                .await?;
            search.assert_succeeded()?;
            ensure!(
                search.data()?["results"]
                    .as_array()
                    .context("MCP search results")?
                    .iter()
                    .any(|item| item["server"] == "created" && item["name"] == "echo"),
                "reloaded stdio catalog: {}",
                search.data()?
            );
            let echo = user.call_tool("mcp.tools.call", json!({
            "server": "created", "name": "echo", "arguments": {"text": "stdio-activation-proof"}
        })).await?;
            echo.assert_succeeded()?;
            echo.assert_text_contains("stdio-peer:stdio-activation-proof")?;
            ensure!(echo.data()?["structured_content"]["origin"] == "local-stdio");
            let records =
                tokio::fs::read_to_string(case.paths().root.join("mcp-stdio-record.jsonl")).await?;
            let records: Vec<Value> = records
                .lines()
                .map(serde_json::from_str)
                .collect::<Result<_, _>>()?;
            let starts: Vec<_> = records
                .iter()
                .filter(|item| item["event"] == "started")
                .collect();
            ensure!(!starts.is_empty(), "the configured process never started");
            for started in starts {
                ensure!(
                    started["argv"] == json!(["two words", "$(literal)"]),
                    "stdio argv used shell expansion: {started}"
                );
                ensure!(
                    started["cwd"] == case.workspace().join("services").to_string_lossy().as_ref(),
                    "wrong process cwd: {started}"
                );
                ensure!(
                    started["token"] == TOKEN,
                    "configured environment did not reach the child"
                );
            }
            ensure!(
                records
                    .iter()
                    .any(|item| item["request"]["method"] == "initialize")
            );
            ensure!(
                records
                    .iter()
                    .any(|item| item["request"]["method"] == "tools/call"
                        && item["request"]["params"]["arguments"]["text"]
                            == "stdio-activation-proof")
            );

            let detail = user
                .call_tool("mcp.connections.get", json!({"id": id}))
                .await?;
            detail.assert_succeeded()?;
            let before_reload = case.config_snapshot().await?;
            let deleted = user
                .call_tool(
                    "mcp.connections.delete",
                    json!({
                        "id": id, "expected_revision": detail.data()?["revision"]
                    }),
                )
                .await?;
            deleted.assert_succeeded()?;
            wait_reload(&case, &deleted, &before_reload).await?;
            ensure!(
                workspace(&case).await?["plugins"]["list"]["agena.mcp"]["settings"]["servers"]
                    .get("created")
                    .is_none()
            );
            let catalog = user.call_tool("mcp.connections.list", json!({})).await?;
            catalog.assert_succeeded()?;
            ensure!(
                catalog.data()?["connections"]
                    .as_array()
                    .context("connections")?
                    .is_empty()
            );
            #[cfg(unix)]
            assert_peers_exited(&case.paths().root.join("mcp-stdio-record.jsonl")).await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flat_update_keeps_only_requested_secrets_and_omitted_preservation_clears_removed_fields()
-> TestResult {
    connection_fixture()
        .before_start(|case| async move {
            let mut config = stdio(&case.paths().root);
            config["enabled"] = json!(false);
            config["process"]["env"] = json!({"KEEP": "keep-private-token", "REMOVE": "remove-private-token"});
            case.write_config(agena_testkit::SettingsLayer::Workspace, workspace_document(json!({"editable": config}))).await
        })
        .run(|case| async move {
            let user = user(&case).await?;
            configure_policy(&user, &[], "ask").await?;
            let list = user.call_tool("mcp.connections.list", json!({})).await?;
            list.assert_succeeded()?;
            let id = list.data()?["connections"].as_array().context("connections")?.iter()
                .find(|item| item["name"] == "editable").context("editable record")?["id"]
                .as_str().context("editable id")?.to_owned();
            let detail = user.call_tool("mcp.connections.get", json!({"id": id})).await?;
            detail.assert_succeeded()?;
            let mut config = detail.data()?["config"].clone();
            config["process"]["env"].as_object_mut().context("environment map")?.remove("REMOVE");
            let before_reload = case.config_snapshot().await?;
            let updated = user.call_tool("mcp.connections.update", json!({
                "id": id, "config": config, "expected_revision": detail.data()?["revision"],
                "preserve_secrets": ["/process/env/KEEP"]
            })).await?;
            updated.assert_succeeded()?;
            redacted(&updated, &["keep-private-token", "remove-private-token"], true)?;
            ensure!(workspace(&case).await?["plugins"]["list"]["agena.mcp"]["settings"]["servers"]["editable"]["process"]["env"]
                == json!({"KEEP": "keep-private-token"}));
            wait_reload(&case, &updated, &before_reload).await?;

            let detail = user.call_tool("mcp.connections.get", json!({"id": id})).await?;
            detail.assert_succeeded()?;
            let mut config = detail.data()?["config"].clone();
            config["process"]["env"] = json!({});
            let before_reload = case.config_snapshot().await?;
            let cleared = user.call_tool("mcp.connections.update", json!({
                "id": id, "config": config, "expected_revision": detail.data()?["revision"]
            })).await?;
            cleared.assert_succeeded()?;
            wait_reload(&case, &cleared, &before_reload).await?;
            ensure!(workspace(&case).await?["plugins"]["list"]["agena.mcp"]["settings"]["servers"]["editable"]["process"]["env"] == json!({}));
            ensure!(!case.paths().root.join("mcp-stdio-record.jsonl").exists(), "saving a disabled connection started a process");
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_a_workspace_override_requires_approval_before_reactivating_global_process()
-> TestResult {
    connection_fixture()
        .configure_global_with(|config, paths, _| {
            config["plugins"]["list"]["agena.mcp"]["settings"]["servers"] =
                json!({"shared": stdio(&paths.root)});
        })
        .workspace_config(workspace_document(json!({"shared": {
            "transport": "stdio", "enabled": false, "process": {"command": "python3"}
        }})))
        .run(|case| async move {
            let user = user(&case).await?;
            configure_policy(&user, &[], "ask").await?;
            let list = user.call_tool("mcp.connections.list", json!({})).await?;
            list.assert_succeeded()?;
            let connections = list.data()?["connections"]
                .as_array()
                .context("connections")?;
            ensure!(connections.len() == 2, "{connections:?}");
            ensure!(
                connections
                    .iter()
                    .any(|item| item["scope"] == "global" && item["effective"] == false)
            );
            let override_record = connections
                .iter()
                .find(|item| item["scope"] == "workspace")
                .context("disabled workspace override")?;
            ensure!(override_record["enabled"] == false);
            let id = override_record["id"]
                .as_str()
                .context("workspace override id")?
                .to_owned();
            let detail = user
                .call_tool("mcp.connections.get", json!({"id": id}))
                .await?;
            detail.assert_succeeded()?;
            let before = bytes(&case).await?;
            ensure!(
                !case.paths().root.join("mcp-stdio-record.jsonl").exists(),
                "disabled override did not shadow global process"
            );
            let delete = user
                .start_tool(
                    "mcp.connections.delete",
                    json!({
                        "id": id, "expected_revision": detail.data()?["revision"]
                    }),
                )
                .await?;
            let approval = delete.run().wait_permission().await?;
            ensure!(
                bytes(&case).await? == before,
                "override was removed before fallback process approval"
            );
            ensure!(
                !case.paths().root.join("mcp-stdio-record.jsonl").exists(),
                "fallback started before approval"
            );
            approval.deny_once().await?;
            delete
                .wait_settled()
                .await?
                .assert_failure_kind(ModelFeedbackKind::UserDeclined)?;
            ensure!(
                bytes(&case).await? == before,
                "declined fallback activation removed the override"
            );
            ensure!(
                !case.paths().root.join("mcp-stdio-record.jsonl").exists(),
                "declined fallback activation started the process"
            );
            let after = user.call_tool("mcp.connections.list", json!({})).await?;
            after.assert_succeeded()?;
            ensure!(
                after.data()?["connections"]
                    .as_array()
                    .context("connections")?
                    .iter()
                    .any(|item| item["id"] == id && item["enabled"] == false)
            );
            Ok(())
        })
        .await
}
