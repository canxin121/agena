use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use agena_testkit::TestResult;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::super::{allow_permissions, fixture_without_memory};

async fn records(path: &Path) -> Result<Vec<Value>> {
    tokio::fs::read_to_string(path)
        .await
        .context("the LSP peer did not write its protocol transcript")?
        .lines()
        .map(|line| serde_json::from_str(line).context("invalid LSP peer transcript record"))
        .collect()
}

fn configured_server<'a>(inventory: &'a Value, name: &str) -> Result<&'a Value> {
    inventory["servers"]
        .as_array()
        .context("LSP server inventory has no servers")?
        .iter()
        .find(|server| server["name"] == name)
        .with_context(|| format!("LSP route {name} was not registered"))
}

#[cfg(unix)]
async fn assert_peer_exited(pid: u32) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        // SAFETY: signal 0 only checks existence/permission; it sends no signal
        // and cannot terminate a process. The PID came from this case's peer.
        let exists = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if exists == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "Agena cleanup left its LSP peer running (pid {pid})"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub(super) async fn language_server_navigation_and_document_versions() -> TestResult {
    let pid = Arc::new(AtomicU32::new(0));
    let observed_pid = pid.clone();
    let result = fixture_without_memory()
        // The fixture records its protocol in a test-owned directory outside the indexed project.
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin("agena.lsp")
        .seed_file(
            "fixtures/lsp_peer.py",
            include_bytes!("lsp_peer.py").to_vec(),
        )
        .seed_file("workspace/.lsp-fixture-root", b"local LSP project".to_vec())
        .seed_file(
            "workspace/sample.rs",
            b"let initial_symbol = 1;\ninitial_symbol;\n".to_vec(),
        )
        .seed_file("workspace/unrouted.unknown", b"no LSP route".to_vec())
        .seed_file("workspace/crash.rs", b"let force_peer_exit = 0;".to_vec())
        .configure_global_with(|config, paths, _| {
            config["plugins"]["list"]["agena.lsp"]["package"] = json!({"kind": "static"});
            config["plugins"]["list"]["agena.lsp"]["settings"] = json!({
                "defaults": {
                    "env": {"AGENA_LSP_SHARED": "shared-default"},
                    "root_markers": [".lsp-fixture-root"]
                },
                "servers": {
                    "local-fixture": {
                        "process": {
                            "command": "python3",
                            "args": [
                                "-u",
                                paths.root.join("fixtures/lsp_peer.py"),
                                paths.root.join("fixtures/lsp-rpc.jsonl")
                            ],
                            "env": {"AGENA_LSP_SERVER": "server-specific"}
                        },
                        "routing": {"file_extensions": ["rs"]},
                        "session": {
                            "initialization_options": {"fixture": "initialization-marker"}
                        }
                    }
                }
            });
        })
        .run(move |case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let log = case.paths().root.join("fixtures/lsp-rpc.jsonl");
            ensure!(!log.try_exists()?, "startup launched the LSP peer");
            let servers = user.call_tool("lsp.servers", json!({})).await?;
            servers.assert_succeeded()?;
            for name in ["clangd", "gopls", "local-fixture", "pyright", "rust-analyzer", "typescript"] {
                let route = configured_server(servers.data()?, name)?;
                ensure!(route["running_roots"] == json!([]), "{route}");
            }
            let configured = configured_server(servers.data()?, "local-fixture")?;
            ensure!(
                configured["command_available"] == true,
                "Python 3 is required for repository protocol fixtures: {configured}"
            );

            let cold_write = user.call_tool(
                "fs.write",
                json!({"path": "cold.rs", "content": "fn cold_write() {}\n"}),
            ).await?;
            cold_write.assert_succeeded()?;
            ensure!(cold_write.data()?.get("post_edit_validation").is_none());
            ensure!(
                !log.try_exists()?,
                "inventory or a file write launched the LSP peer"
            );

            let start = user.call_tool("lsp.start", json!({"file_path": "sample.rs"})).await?;
            start.assert_succeeded()?;
            ensure!(start.data()?["server"] == "local-fixture");
            let first_records = records(&log).await?;
            let started = first_records
                .iter()
                .find(|entry| entry["event"] == "started")
                .context("LSP peer did not record its actual process identity")?;
            let first_pid = u32::try_from(started["pid"].as_u64().context("invalid LSP peer PID")?)?;
            observed_pid.store(first_pid, Ordering::SeqCst);
            ensure!(started["shared_env"] == "shared-default");
            ensure!(started["server_env"] == "server-specific");
            let servers = user.call_tool("lsp.servers", json!({})).await?;
            servers.assert_succeeded()?;
            let running = &configured_server(servers.data()?, "local-fixture")?["running_roots"];
            ensure!(running == &json!([case.workspace().canonicalize()?]), "{running}");

            let position = json!({"file_path": "sample.rs", "line": 0, "character": 4});
            let hover = user.call_tool("lsp.hover", position.clone()).await?;
            hover.assert_succeeded()?;
            hover.assert_text_contains("lsp-hover-initial")?;

            let definition = user.call_tool("lsp.definition", position.clone()).await?;
            definition.assert_succeeded()?;
            let locations = definition.data()?["locations"]
                .as_array()
                .context("LSP definition response has no locations")?;
            ensure!(locations.len() == 1);
            ensure!(
                locations[0]
                    .as_str()
                    .is_some_and(|location| location.ends_with("sample.rs:1:5")),
                "LSP source coordinates were not projected as one-based locations: {locations:?}"
            );

            let data = definition.data()?;
            ensure!(data["targets"][0]["range"]["start"]["character"] == 0);
            ensure!(data["targets"][0]["selection_range"]["start"]["character"] == 4);
            ensure!(data["position_encoding"] == "utf-16");
            let revision = data["source_revision"].clone();

            let implementation = user
                .call_tool("lsp.implementation", position.clone())
                .await?;
            implementation.assert_succeeded()?;
            ensure!(implementation.data()?["total_locations"] == 1);

            let symbols = user
                .call_tool("lsp.symbols", json!({"file_path": "sample.rs"}))
                .await?;
            symbols.assert_succeeded()?;
            ensure!(symbols.data()?["symbols"][0]["name"] == "initial_symbol");
            let symbols = user.call_tool("lsp.symbols", json!({
                "file_path": "sample.rs", "query": "initial_symbol", "server": "local-fixture",
            })).await?;
            symbols.assert_succeeded()?;
            ensure!(symbols.data()?["symbols"][0]["name"] == "initial_symbol");

            let mut reference_input = position.clone();
            reference_input["include_declaration"] = json!(false);
            let references = user.call_tool("lsp.references", reference_input).await?;
            references.assert_succeeded()?;
            ensure!(
                references.data()?["locations"]
                    .as_array()
                    .is_some_and(|locations| locations.len() == 2)
            );

            let diagnostics = user
                .call_tool("lsp.diagnostics", json!({"file_path": "sample.rs"}))
                .await?;
            diagnostics.assert_succeeded()?;
            let diagnostic_data = diagnostics.data()?;
            ensure!(
                diagnostic_data["state"] == "current",
                "diagnostics are not synchronized: {diagnostic_data}"
            );
            let initial_version = diagnostic_data["document_version"]
                .as_i64()
                .context("LSP diagnostics omitted the synchronized version")?;
            ensure!(diagnostic_data["reported_version"] == initial_version);
            diagnostics
                .assert_text_contains(&format!("lsp-diagnostic-version-{initial_version}"))?;

            let batch = user
                .call_tool(
                    "lsp.diagnostics_many",
                    json!({
                        "file_paths": ["sample.rs", "unrouted.unknown"],
                    }),
                )
                .await?;
            batch.assert_succeeded()?;
            let batch_data = batch.data()?;
            ensure!(batch_data["files"][0]["state"] == "current", "{batch_data}");
            ensure!(batch_data["files"][1]["state"] == "error", "{batch_data}");

            let edit = user
                .call_tool(
                    "fs.replace",
                    json!({
                        "path": "sample.rs",
                        "old": "let initial_symbol = 1;\ninitial_symbol;\n",
                        "new": "let updated_symbol = 2;\nupdated_symbol;\n",
                    }),
                )
                .await?;
            edit.assert_succeeded()?;
            let edit_data = edit.data()?;
            let validation = &edit_data["post_edit_validation"]["files"][0];
            ensure!(validation["state"] == "current", "{edit_data}");
            ensure!(validation["total_diagnostics"] == 1, "{validation}");
            let mut stale_query = position.clone();
            stale_query["expected_revision"] = revision;
            user.call_tool("lsp.definition", stale_query)
                .await?
                .assert_failed()?;

            let updated = user.call_tool("lsp.hover", position).await?;
            updated.assert_succeeded()?;
            updated.assert_text_contains("lsp-hover-updated")?;
            let updated_diagnostics = user
                .call_tool("lsp.diagnostics", json!({"file_path": "sample.rs"}))
                .await?;
            updated_diagnostics.assert_succeeded()?;
            let updated_data = updated_diagnostics.data()?;
            let next_version = updated_data["document_version"]
                .as_i64()
                .context("updated LSP diagnostics omitted the synchronized version")?;
            ensure!(next_version > initial_version);
            ensure!(updated_data["reported_version"] == next_version);
            ensure!(updated_data["state"] == "current");
            updated_diagnostics
                .assert_text_contains(&format!("lsp-diagnostic-version-{next_version}"))?;

            user.call_tool(
                "lsp.hover",
                json!({"file_path": "unrouted.unknown", "line": 0, "character": 0}),
            )
            .await?
            .assert_failed()?;

            let transcript = records(&log).await?;
            let requests = |method: &str| {
                transcript
                    .iter()
                    .filter(move |entry| entry["method"] == method)
                    .collect::<Vec<_>>()
            };
            let initialized = requests("initialize");
            ensure!(
                initialized.len() == 1,
                "the same project spawned duplicate LSP peers"
            );
            let init = &initialized[0]["params"];
            let root = url::Url::from_directory_path(case.workspace())
                .map_err(|_| anyhow::anyhow!("fixture workspace is not an absolute directory"))?;
            ensure!(
                init["rootUri"] == root.as_str(),
                "wrong LSP project root: {init}"
            );
            ensure!(init["initializationOptions"]["fixture"] == "initialization-marker");
            ensure!(
                init["clientInfo"]["name"]
                    .as_str()
                    .is_some_and(|name| !name.is_empty())
            );
            ensure!(requests("initialized").len() == 1);
            let opened = requests("textDocument/didOpen");
            let changed = requests("textDocument/didChange");
            ensure!(
                opened.len() == 1,
                "repeated navigation reopened the same document"
            );
            ensure!(
                changed.len() == 1,
                "unchanged files caused redundant document updates"
            );
            ensure!(
                opened[0]["params"]["textDocument"]["text"]
                    == "let initial_symbol = 1;\ninitial_symbol;\n"
            );
            ensure!(changed[0]["params"]["textDocument"]["version"] == next_version);
            let saved = requests("textDocument/didSave");
            ensure!(saved.len() == 1);
            ensure!(saved[0]["params"]["text"] == "let updated_symbol = 2;\nupdated_symbol;\n");
            ensure!(
                changed[0]["params"]["contentChanges"][0]["range"]["end"]
                    == json!({"line":2,"character":0})
            );
            ensure!(
                changed[0]["params"]["contentChanges"][0]["text"]
                    == "let updated_symbol = 2;\nupdated_symbol;\n"
            );
            let reference_requests = requests("textDocument/references");
            ensure!(reference_requests.len() == 1);
            ensure!(reference_requests[0]["params"]["context"]["includeDeclaration"] == false);
            let hovers = requests("textDocument/hover");
            ensure!(
                hovers.len() == 2,
                "an unrouted file reached the wrong language server"
            );
            ensure!(
                hovers
                    .iter()
                    .all(|entry| entry["params"]["position"] == json!({"line": 0, "character": 4}))
            );

            let stop = user.call_tool("lsp.stop", json!({"server":"local-fixture"})).await?;
            stop.assert_succeeded()?;
            ensure!(stop.data()?["stopped_instances"] == 1);
            #[cfg(unix)]
            assert_peer_exited(first_pid).await?;
            let servers = user.call_tool("lsp.servers", json!({})).await?;
            servers.assert_succeeded()?;
            ensure!(configured_server(servers.data()?, "local-fixture")?["running_roots"] == json!([]));

            let edit = user.call_tool(
                "fs.replace",
                json!({"path":"cold.rs", "old":"cold_write", "new":"after_stop"}),
            ).await?;
            edit.assert_succeeded()?;
            ensure!(edit.data()?.get("post_edit_validation").is_none());
            let after_stop = records(&log).await?;
            ensure!(after_stop.iter().any(|entry| entry["method"] == "shutdown"));
            ensure!(after_stop.iter().filter(|entry| entry["event"] == "started").count() == 1);

            let position = json!({"file_path": "sample.rs", "line": 0, "character": 4});
            let reopened = user.call_tool("lsp.hover", position.clone()).await?;
            reopened.assert_succeeded()?;
            reopened.assert_text_contains("lsp-hover-updated")?;
            user.call_tool(
                "lsp.hover",
                json!({"file_path":"crash.rs", "line":0, "character":4}),
            )
            .await?
            .assert_failed()?;
            let servers = user.call_tool("lsp.servers", json!({})).await?;
            servers.assert_succeeded()?;
            ensure!(
                configured_server(servers.data()?, "local-fixture")?["running_roots"] == json!([])
            );

            let recovered = user.call_tool("lsp.hover", position).await?;
            recovered.assert_succeeded()?;
            recovered.assert_text_contains("lsp-hover-updated")?;
            let after = records(&log).await?;
            let pids = after
                .iter()
                .filter(|entry| entry["event"] == "started")
                .filter_map(|entry| entry["pid"].as_u64())
                .collect::<Vec<_>>();
            ensure!(
                pids.len() == 3 && pids.windows(2).all(|pair| pair[0] != pair[1]),
                "expected one fresh process after stop and one after crash: {pids:?}"
            );
            observed_pid.store(u32::try_from(pids[2])?, Ordering::SeqCst);
            Ok(())
        })
        .await;

    // run() has awaited the product's normal shutdown and case process cleanup.
    // Do not infer a graceful LSP handshake from that fact: independently check
    // the actual peer PID on Unix, where the repository's Python fixtures run.
    #[cfg(unix)]
    if pid.load(Ordering::SeqCst) != 0 {
        assert_peer_exited(pid.load(Ordering::SeqCst)).await?;
    }
    result
}
