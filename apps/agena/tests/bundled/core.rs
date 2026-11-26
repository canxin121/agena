//! Real user/model/tool scenarios for the bundled filesystem and content plugins.
//!
//! Every operation below is requested by the local Responses model through
//! UserSession::call_tool. Disk assertions are independent observations of the
//! product's effects; no plugin Rust method is called from these tests.

use agena_testkit::{
    EditOptions, Reply, RequestMatch, SettingsLayer, TestCase, TestResult, ToolResult, UserSession,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::{allow_permissions, fixture};

#[path = "core_contracts.rs"]
mod core_contracts;
#[path = "core_memory.rs"]
mod core_memory;
#[path = "core_permissions.rs"]
mod core_permissions;
#[path = "core_tool_script.rs"]
mod core_tool_script;

pub(super) const CASES: &[(&str, fn() -> TestResult)] = &[
    ("agena.fs", fs_edits_files_with_revision_checks),
    ("agena.shell", shell_runs_commands_and_owns_background_jobs),
    (
        "agena.tool_script",
        core_tool_script::composes_session_tools_and_preserves_permissions,
    ),
    (
        "agena.commands",
        core_contracts::commands_and_skills_use_independent_catalogs,
    ),
    (
        "agena.skills",
        skills_manage_resources_revisions_and_availability,
    ),
    (
        "agena.memory",
        memory_persists_and_honors_retrieval_settings,
    ),
    (
        "agena.inline_ui",
        inline_ui_publishes_and_updates_session_content,
    ),
    (
        "agena.report",
        report_preserves_structured_findings_and_validates_locations,
    ),
];

async fn user(case: &TestCase) -> Result<UserSession> {
    let user = case.user().new_session().await?;
    allow_permissions(&user).await?;
    Ok(user)
}

async fn call(user: &UserSession, tool: &str, input: Value) -> Result<ToolResult> {
    let result = user
        .call_tool(tool, input)
        .await
        .with_context(|| format!("real model round trip for {tool}"))?;
    result.assert_succeeded()?;
    Ok(result)
}

fn string(value: &Value, pointer: &str) -> Result<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("missing string {pointer} in durable tool data: {value}"))
}

fn text_payload_json(result: &ToolResult) -> Result<Value> {
    // Text-only tools retain their complete output under the durable `text`
    // field. The model projection can append preview notices, so it is a
    // separate contract and must not be parsed as the source JSON document.
    serde_json::from_str(&string(result.data()?, "/text")?)
        .with_context(|| format!("parse {:?} durable text as JSON", result.tool))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fs_edits_files_with_revision_checks() -> TestResult {
    fixture()
        .expect_plugin("agena.fs")
        .run(|case| async move {
            let user = user(&case).await?;
            let path = case.workspace().join("notes/sample.txt");
            call(
                &user,
                "fs.write",
                json!({
                    "path": "notes/sample.txt",
                    "content": "alpha-marker\nalpha-marker\n",
                    "create_parents": true
                }),
            )
            .await?;
            assert_eq!(tokio::fs::read_to_string(&path).await?, "alpha-marker\nalpha-marker\n");

            let stat = call(&user, "fs.stat", json!({"path":"notes/sample.txt","hash":true})).await?;
            assert_eq!(stat.data()?["kind"], "file");
            let revision = string(stat.data()?, "/sha256")?;
            assert_eq!(revision.len(), 64);

            let read = call(&user, "fs.read", json!({"file_path":"notes/sample.txt","mode":"text"})).await?;
            read.assert_text_contains("alpha-marker")?;
            let replace = call(
                &user,
                "fs.replace",
                json!({
                    "path": "notes/sample.txt",
                    "old": "alpha-marker",
                    "new": "beta-marker",
                    "replace_all": true,
                    "expected_occurrences": 2,
                    "expected_sha256": revision
                }),
            )
            .await?;
            assert_eq!(replace.data()?["replacements"], 2);
            assert_eq!(tokio::fs::read_to_string(&path).await?, "beta-marker\nbeta-marker\n");

            // A real stale writer must not overwrite the product's newer file.
            let stale = user
                .call_tool(
                    "fs.write",
                    json!({
                        "path":"notes/sample.txt",
                        "content":"stale-writer-overwrite",
                        "expected_sha256":revision
                    }),
                )
                .await?;
            stale.assert_failed()?;
            assert_eq!(tokio::fs::read_to_string(&path).await?, "beta-marker\nbeta-marker\n");

            call(
                &user,
                "fs.apply_patch",
                json!({"patch":"*** Begin Patch\n*** Add File: patched.txt\n+patch-created-marker\n*** End Patch"}),
            )
            .await?;
            assert_eq!(
                tokio::fs::read_to_string(case.workspace().join("patched.txt")).await?,
                "patch-created-marker\n"
            );
            call(
                &user,
                "fs.glob",
                json!({"path":"notes","pattern":"*.txt","kind":"file"}),
            )
            .await?
            .assert_text_contains("sample.txt")?;
            call(
                &user,
                "fs.grep",
                json!({"path":"notes","pattern":"beta-marker","fixed_strings":true}),
            )
            .await?
            .assert_text_contains("beta-marker")?;
            let files = call(
                &user,
                "fs.read_many",
                json!({"paths":["notes/sample.txt","patched.txt"],"max_total_bytes":4096}),
            )
            .await?;
            files.assert_text_contains("beta-marker")?;
            files.assert_text_contains("patch-created-marker")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_runs_commands_and_owns_background_jobs() -> TestResult {
    super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin("agena.shell")
        .seed_file("workspace/shell-input.txt", b"shell-input-marker\n".to_vec())
        .run(|case| async move {
            let user = user(&case).await?;
            let shell = if cfg!(windows) { "powershell" } else { "bash" };
            let command = if cfg!(windows) {
                "Get-Content shell-input.txt; [System.IO.File]::WriteAllText('shell-result.txt', 'shell-output-marker')"
            } else {
                "cat shell-input.txt; printf 'shell-output-marker' > shell-result.txt"
            };
            let completed = call(
                &user,
                "shell.exec",
                json!({
                    "shell":shell,"command":command,
                    "reads":["shell-input.txt"],"writes":["shell-result.txt"],"network":[],
                    "timeout_ms":10000
                }),
            )
            .await?;
            completed.assert_text_contains("shell-input-marker")?;
            assert_eq!(completed.data()?["exit_code"], 0);
            assert_eq!(
                tokio::fs::read_to_string(case.workspace().join("shell-result.txt")).await?,
                "shell-output-marker"
            );

            // A command can execute correctly and return a nonzero child status;
            // its exit code is product data, not a fabricated successful exit.
            let nonzero = call(
                &user,
                "shell.exec",
                json!({
                    "shell":shell,
                    "command": if cfg!(windows) {
                        "[Console]::Error.WriteLine('expected-command-error'); exit 17"
                    } else {
                        "printf 'expected-command-error\\n' >&2; exit 17"
                    },
                    "reads":[],"writes":[],"network":[],"timeout_ms":10000
                }),
            )
            .await?;
            assert_eq!(nonzero.data()?["exit_code"], 17);
            nonzero.assert_text_contains("expected-command-error")?;

            let background = call(
                &user,
                "shell.spawn",
                json!({
                    "shell":shell,
                    "command": if cfg!(windows) {
                        "[Console]::WriteLine('background-ready-marker'); Start-Sleep -Seconds 120"
                    } else {
                        "printf 'background-ready-marker\\n'; sleep 120"
                    },
                    "reads":[],"writes":[],"network":[],"timeout_ms":120000
                }),
            )
            .await?;
            let process_id = string(background.data()?, "/process_id")?;
            assert_eq!(background.data()?["background"], true);
            call(
                &user,
                "shell.read",
                json!({"process_id":process_id,"since_seq":0,"wait_ms":1000,"max_output_bytes":4096}),
            )
            .await?
            .assert_text_contains("background-ready-marker")?;

            let watched = call(
                &user,
                "shell.watch",
                json!({"process_id":process_id,"policy":{"include_pattern":"^watch-event-marker$"}}),
            )
            .await?;
            assert_eq!(watched.data()?["action"], "watch_update");

            let http = case.http().await;
            let endpoint = format!("/api/v1/sessions/{}/shell/processes/{process_id}", user.id());
            let running: Value = http.get(&endpoint).await?.require_success()?.json()?;
            assert_eq!(running["status"], "running");
            assert_eq!(running["monitored"], true);
            let resource = running["output_resource"]["resource_id"].as_str().context("shell output resource")?;
            let output = format!("/api/v1/sessions/{}/content/{resource}/text", user.id());
            assert!(user.parts().await?.iter().any(|part| part.presentation.as_ref().is_some_and(|view| {
                view.blocks.iter().any(|block| matches!(block, agena_domain::ViewBlock::ShellProcess {
                    process_id: presented, ..
                } if presented == &process_id))
            })));
            assert!(http.get(&output).await?.require_success()?.text()?.contains("background-ready-marker"));

            // Another session cannot take over a process owned by this one.
            let stranger = self::user(&case).await?;
            let owned: Vec<Value> = http.get(&format!("/api/v1/sessions/{}/shell/processes", user.id())).await?.require_success()?.json()?;
            assert!(owned.iter().any(|row| row["process_id"] == process_id && row["output_resource"] == running["output_resource"]));
            let rows: Vec<Value> = http.get(&format!("/api/v1/sessions/{}/shell/processes", stranger.id())).await?.require_success()?.json()?;
            assert!(!rows.iter().any(|row| row["process_id"] == process_id));
            let retired = format!("/api/v1/sessions/{}/monitor/subscriptions", user.id());
            assert_eq!(http.get(&retired).await?.status, reqwest::StatusCode::NOT_FOUND);
            assert_eq!(http.get("/api/v1/plugins/agena.monitor").await?.status, reqwest::StatusCode::NOT_FOUND);
            let wrong_owner = format!("/api/v1/sessions/{}/shell/processes/{process_id}", stranger.id());
            assert_eq!(
                http.send_json(reqwest::Method::DELETE, &wrong_owner, &json!({})).await?.status,
                reqwest::StatusCode::NOT_FOUND
            );
            stranger
                .call_tool("shell.stop", json!({"process_id":process_id}))
                .await?
                .assert_failed()?;
            let completion = case.model()
                .expect(RequestMatch::new("operator stop wakes the owning conversation", |request| {
                    request.body["input"].as_array().is_some_and(|items| {
                        items.iter().any(|item| {
                            let text = item.to_string();
                            matches!(item["role"].as_str(), Some("system" | "developer"))
                                && text.contains("background-ready-marker")
                                && text.contains("was stopped.")
                        })
                    })
                }))
                .reply(Reply::text("shell-stop-observed"));
            let stopped_process: Value = http.send_json(reqwest::Method::DELETE, &endpoint, &json!({}))
                .await?.require_success()?.json()?;
            assert_eq!(stopped_process["status"], "stopped");
            assert!(http.get(&output).await?.require_success()?.text()?.contains("background-ready-marker"));
            completion.observed().await?;
            user.wait_text_contains("shell-stop-observed").await?;
            let stopped = call(&user, "shell.stop", json!({"process_id":process_id})).await?;
            assert_eq!(stopped.data()?["process_id"], process_id);
            ensure!(stopped.data()?["status"] != "running", "managed job remained running");
            let jobs = call(&user, "shell.list", json!({})).await?;
            let own_job = jobs.data()?["processes"]
                .as_array()
                .context("shell.list omitted process summaries")?
                .iter()
                .find(|job| job["process_id"] == process_id)
                .context("stopped job disappeared from its owner's process history")?;
            ensure!(own_job["status"] != "running");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skills_manage_resources_revisions_and_availability() -> TestResult {
    const FIRST: &str = "---\nname: testkit-managed-skill\ndescription: Testkit managed skill fixture.\n---\n# Instructions\nskill-first-marker\n";
    const SECOND: &str = "---\nname: testkit-managed-skill\ndescription: Updated testkit managed skill fixture.\n---\n# Instructions\nskill-updated-marker\n";
    fixture()
        .expect_plugin("agena.skills")
        .run(|case| async move {
            let user = user(&case).await?;
            let created = call(&user, "skills.create", json!({"scope":"workspace","document":FIRST})).await?;
            let id = string(created.data()?, "/id")?;
            let path = std::path::PathBuf::from(string(created.data()?, "/path")?);
            ensure!(path.starts_with(case.workspace()), "skill escaped its workspace");
            assert_eq!(tokio::fs::read_to_string(&path).await?, FIRST);
            assert_eq!(created.data()?["enabled"], true);
            let resource = call(
                &user,
                "skills.write_resource",
                json!({"id":id,"path":"references/note.txt","content":"skill-resource-marker\n"}),
            )
            .await?;
            let resource_revision = string(resource.data()?, "/revision")?;
            let read_resource = call(
                &user, "skills.read_resource", json!({"id":id,"path":"references/note.txt"}),
            ).await?;
            assert_eq!(read_resource.data()?["content"], "skill-resource-marker\n");
            assert_eq!(read_resource.data()?["revision"], resource_revision);
            // Replacing an existing resource requires its revision, even when
            // the same model authored the original version earlier in this run.
            user.call_tool(
                "skills.write_resource",
                json!({"id":id,"path":"references/note.txt","content":"unversioned-overwrite"}),
            ).await?.assert_failed()?;
            assert_eq!(
                tokio::fs::read_to_string(path.parent().context("skill directory")?.join("references/note.txt")).await?,
                "skill-resource-marker\n"
            );

            let opened = call(&user, "skills.get", json!({"id":id})).await?;
            opened.assert_text_contains("skill-first-marker")?;
            let revision = string(opened.data()?, "/revision")?;
            let updated = call(
                &user, "skills.update", json!({"id":id,"document":SECOND,"expected_revision":revision}),
            ).await?;
            assert_eq!(tokio::fs::read_to_string(&path).await?, SECOND);
            assert_eq!(
                tokio::fs::read_to_string(path.parent().context("skill directory")?.join("references/note.txt")).await?,
                "skill-resource-marker\n"
            );
            user.call_tool(
                "skills.update",
                json!({"id":id,"document":FIRST,"expected_revision":revision}),
            ).await?.assert_failed()?;
            assert_eq!(tokio::fs::read_to_string(&path).await?, SECOND);

            let disabled = call(
                &user, "skills.set_enabled",
                json!({"id":id,"enabled":false,"expected_revision":string(updated.data()?, "/revision")?}),
            ).await?;
            assert_eq!(disabled.data()?["enabled"], false);
            let available = call(
                &user, "skills.list", json!({"query":"testkit-managed-skill","scope":"workspace"}),
            ).await?;
            assert_eq!(available.data()?["total"], 0);
            let inactive = call(
                &user, "skills.list",
                json!({"query":"testkit-managed-skill","scope":"workspace","include_inactive":true}),
            ).await?;
            assert_eq!(inactive.data()?["total"], 1);
            assert_eq!(inactive.data()?["skills"][0]["enabled"], false);

            call(
                &user, "skills.set_enabled",
                json!({"id":id,"enabled":true,"expected_revision":string(disabled.data()?, "/revision")?}),
            ).await?;
            call(
                &user, "skills.remove_resource",
                json!({"id":id,"path":"references/note.txt","expected_revision":resource_revision}),
            ).await?;
            ensure!(!path.parent().context("skill directory")?.join("references/note.txt").exists());
            let final_detail = call(&user, "skills.get", json!({"id":id})).await?;
            let removed = call(
                &user, "skills.remove",
                json!({"id":id,"expected_revision":string(final_detail.data()?, "/revision")?}),
            ).await?;
            assert_eq!(removed.data()?["removed"], true);
            ensure!(!path.parent().context("skill directory")?.exists());
            ensure!(!case.paths().data_dir.join(".agena/skills/testkit-managed-skill").exists());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_persists_and_honors_retrieval_settings() -> TestResult {
    const NAME: &str = "testkitmemoryalpha";
    const FIRST: &str = "testkitmemoryalpha durable-memory-first-marker";
    const SECOND: &str = "testkitmemoryalpha durable-memory-updated-marker";
    fixture()
        .expect_plugin("agena.memory")
        .configure_global(|config| {
            config["plugins"]["list"]["agena.memory"] = json!({
                "package":{"kind":"static"},
                "settings":{"retrieval":{"enabled":false,"limit":3,"min_query_chars":8}}
            });
        })
        .run(|case| async move {
            let user = user(&case).await?;
            let created = call(
                &user,
                "memory.write",
                json!({
                    "name":NAME,"description":"Memory regression fixture.",
                    "kind":"fact","content":FIRST
                }),
            )
            .await?;
            let revision = string(created.data()?, "/sha256")?;
            assert_eq!(created.data()?["file_name"], "testkitmemoryalpha.md");
            let opened = call(&user, "memory.get", json!({"name":NAME})).await?;
            opened.assert_text_contains("durable-memory-first-marker")?;
            assert_eq!(opened.data()?["metadata"]["kind"], "fact");
            ensure!(opened.data()?.get("memory_type").is_none());
            let unversioned = user.call_tool(
                "memory.write",
                json!({"name":NAME,"kind":"fact","content":"unversioned-overwrite"}),
            ).await?;
            unversioned.assert_failed()?;
            unversioned.assert_text_contains(&format!("expected_sha256={revision}"))?;
            let unchanged = call(&user, "memory.get", json!({"name":NAME})).await?;
            assert_eq!(unchanged.data()?["sha256"], revision);
            unchanged.assert_text_contains("durable-memory-first-marker")?;
            for invalid in [
                json!({"name":"old-input","content":"missing kind"}),
                json!({"name":"old-input","content":"old field","memory_type":"project"}),
                json!({"name":"old-input","content":"old field","kind":"fact","memory_type":"project"}),
            ] {
                user.call_tool("memory.write", invalid).await?.assert_failed()?;
            }
            let search = call(&user, "memory.search", json!({"query":NAME,"limit":3})).await?;
            ensure!(
                search.data()?["results"]
                    .as_array()
                    .context("memory results")?
                    .iter()
                    .any(|memory| memory["name"] == NAME),
                "newly written memory was not indexed"
            );
            call(
                &user,
                "memory.write",
                json!({
                    "name":NAME,"description":"Memory regression fixture.",
                    "content":SECOND,"kind":"fact","expected_sha256":revision
                }),
            )
            .await?;
            user.call_tool(
                "memory.write",
                json!({"name":NAME,"kind":"fact","content":"stale-memory-overwrite","expected_sha256":revision}),
            )
            .await?
            .assert_failed()?;

            // Prove durable storage by restarting the actual Agena product,
            // keeping only the case's data, model and isolated workspace.
            case.restart().await?;
            let restored_user = self::user(&case).await?;
            let restored = call(&restored_user, "memory.get", json!({"name":NAME})).await?;
            restored.assert_text_contains("durable-memory-updated-marker")?;
            ensure!(!restored.text().contains("stale-memory-overwrite"));

            let without_retrieval = self::user(&case).await?;
            case.model()
                .expect(RequestMatch::user_text(NAME))
                .check(|request| {
                    ensure!(
                        !request
                            .body
                            .to_string()
                            .contains("durable-memory-updated-marker"),
                        "disabled automatic retrieval leaked the saved memory into a new session"
                    );
                    Ok(())
                })
                .reply(Reply::text("memory-retrieval-disabled"));
            without_retrieval
                .send(NAME)
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("memory-retrieval-disabled")?;

            let edit: Value = case
                .settings()
                .set(
                    SettingsLayer::Global,
                    "plugins.list.\"agena.memory\".settings.retrieval.enabled",
                    json!(true),
                    EditOptions::default(),
                )
                .await?
                .require_success()?
                .json()?;
            ensure!(
                edit["reload_requested"] == true && edit["reload"].is_object(),
                "memory retrieval settings were not reloaded: {edit}"
            );
            let with_retrieval = self::user(&case).await?;
            case.model()
                .expect(RequestMatch::user_text(NAME))
                .check(|request| {
                    ensure!(
                        request
                            .body
                            .to_string()
                            .contains("durable-memory-updated-marker"),
                        "enabled memory hook did not add saved context to the real model request"
                    );
                    Ok(())
                })
                .reply(Reply::text("memory-retrieval-enabled"));
            with_retrieval
                .send(NAME)
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("memory-retrieval-enabled")?;

            with_retrieval
                .call_tool("memory.delete", json!({"name":NAME}))
                .await?
                .assert_failed()?;
            let current = call(&with_retrieval, "memory.get", json!({"name":NAME})).await?;
            let deleted = call(
                &with_retrieval,
                "memory.delete",
                json!({"name":NAME,"expected_sha256":current.data()?["sha256"]}),
            ).await?;
            assert_eq!(deleted.data()?["deleted"], true);
            assert_eq!(deleted.data()?["sha256"], current.data()?["sha256"]);
            with_retrieval
                .call_tool("memory.get", json!({"name":NAME}))
                .await?
                .assert_failed()?;
            let removed = call(&with_retrieval, "memory.search", json!({"query":NAME})).await?;
            assert_eq!(
                removed.data()?["results"]
                    .as_array()
                    .context("memory results")?
                    .len(),
                0
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inline_ui_publishes_and_updates_session_content() -> TestResult {
    fixture()
        .expect_plugin("agena.inline_ui")
        .run(|case| async move {
            let user = user(&case).await?;
            let initial = json!({
                "version":1,"title":"Test progress",
                "blocks":[{"type":"paragraph","id":"status","text":"ui-working-marker"}]
            });
            let prepared = call(
                &user, "inline_ui.prepare", json!({"document":initial,"phase":"working"}),
            ).await?;
            let reference = string(prepared.data()?, "/inline_ui_prepared/reference")?;
            let revision = prepared.data()?["revision"].as_u64().context("initial UI revision")?;
            assert_eq!(revision, 1);
            let first = call(&user, "inline_ui.read", json!({"reference":reference})).await?;
            first.assert_text_contains("ui-working-marker")?;
            let first = text_payload_json(&first)?;
            assert_eq!(first["document"], initial);
            assert_eq!(first["phase"], "working");

            // Preparation does not publish a message. The scripted model must
            // embed the actual reference in its ordinary assistant response.
            let answer = format!("Progress follows.\n\n\u{0060}\u{0060}\u{0060}inline-ui\n{reference}\n\u{0060}\u{0060}\u{0060}\n\nWork continues.");
            case.model()
                .expect(RequestMatch::user_text("publish-prepared-ui-marker"))
                .reply(Reply::text(answer.clone()));
            let published = user.send("publish-prepared-ui-marker").await?.wait_settled().await?;
            published.assert_succeeded()?;
            published.assert_assistant_text(&answer)?;

            let complete = json!({
                "version":1,"title":"Test progress",
                "blocks":[{"type":"paragraph","id":"status","text":"ui-complete-marker"}]
            });
            let update = call(
                &user, "inline_ui.update",
                json!({"update":{
                    "reference":reference,"expected_revision":revision,
                    "document":complete,"phase":"complete"
                }}),
            ).await?;
            let update = text_payload_json(&update)?;
            assert_eq!(update["revision"], 2);
            assert_eq!(update["phase"], "complete");
            user.call_tool(
                "inline_ui.update",
                json!({"update":{
                    "reference":reference,"expected_revision":revision,
                    "document":initial
                }}),
            ).await?.assert_failed()?;
            let final_ui = call(&user, "inline_ui.read", json!({"reference":reference})).await?;
            final_ui.assert_text_contains("ui-complete-marker")?;
            let final_ui = text_payload_json(&final_ui)?;
            assert_eq!(final_ui["revision"], 2);
            assert_eq!(final_ui["document"], complete);

            let other_session = self::user(&case).await?;
            other_session.call_tool("inline_ui.read", json!({"reference":reference}))
                .await?.assert_failed()?;
            user.call_tool(
                "inline_ui.prepare",
                json!({"document":{"version":2,"blocks":[{"type":"paragraph","id":"bad","text":"unsupported-version"}]}}),
            ).await?.assert_failed()?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn report_preserves_structured_findings_and_validates_locations() -> TestResult {
    fixture()
        .expect_plugin("agena.report")
        .seed_file("workspace/review.rs", b"fn example() {\n    let unsafe_value = 0;\n}\n".to_vec())
        .run(|case| async move {
            let user = user(&case).await?;
            let findings = call(
                &user, "report.findings",
                json!({
                    "summary":"Two concrete review findings.",
                    "findings":[
                        {
                            "severity":"high","file":"review.rs","line":2,"end_line":2,
                            "title":"review-high-marker","body":"Check the supplied input before use.",
                            "confidence":0.9,"code":"REVIEW001"
                        },
                        {
                            "severity":"info","file":"review.rs","line":1,"end_line":3,
                            "title":"review-info-marker","body":"The fixture deliberately keeps its public shape."
                        }
                    ]
                }),
            ).await?;
            findings.assert_text_contains("review-high-marker")?;
            findings.assert_text_contains("review-info-marker")?;
            let data = findings.data()?;
            assert_eq!(data["counts"]["high"], 1);
            assert_eq!(data["counts"]["info"], 1);
            assert_eq!(data["counts"]["critical"], 0);
            assert_eq!(data["findings"][0]["code"], "REVIEW001");
            assert_eq!(data["findings"][1]["confidence"], 1.0);
            assert_eq!(data["findings"][1]["file"], "review.rs");
            assert_eq!(data["findings"][1]["line"], 1);
            assert_eq!(data["findings"][1]["end_line"], 3);
            // data() came from the public durable tool-detail API, so these are
            // the structured findings clients can later render, not just prose.
            assert_eq!(data["summary"], "Two concrete review findings.");

            user.call_tool(
                "report.findings",
                json!({"findings":[{
                    "severity":"high","file":"review.rs","line":3,"end_line":1,
                    "title":"invalid-range","body":"Must not publish an inverted location."
                }]}),
            ).await?.assert_failed()?;
            let clean = call(
                &user, "report.findings", json!({"summary":"Follow-up review is clean.","findings":[]}),
            ).await?;
            clean.assert_text_contains("Follow-up review is clean.")?;
            assert_eq!(clean.data()?["findings"], json!([]));
            assert_eq!(clean.data()?["counts"]["high"], 0);
            Ok(())
        })
        .await
}
