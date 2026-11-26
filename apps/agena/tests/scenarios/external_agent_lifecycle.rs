use std::os::unix::fs::PermissionsExt;

use agena_api::{live::ToolDetailSection, part::PartResource};
use agena_domain::{
    AttachmentItem, ExternalAgent, ExternalAgentSessionAction as SessionAction,
    ExternalAgentSessionResult as SessionResult, ExternalAgentWorkspaceAction as WorkspaceAction,
    ExternalAgentWorkspaceResult as WorkspaceResult,
};
use agena_testkit::TestResult;
use anyhow::{Context, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Method, StatusCode};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_history_import_resume_fork_and_lifecycle_controls() -> TestResult {
    exercise_native_lifecycle("1").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_history_v2_replay_boundary_and_lifecycle_controls() -> TestResult {
    exercise_native_lifecycle("2").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acp_views_reload_native_history_and_leave_the_database_untouched() -> TestResult {
    let (_bin, builder) = lifecycle_fixture("2")?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace = ordinary.snapshot().await?.session.workspace_id.context("workspace")?;
            let http = case.http().await;
            let history = format!("/api/v1/acp/workspaces/{workspace}/sessions");
            let open = WorkspaceAction::Open {
                agent: ExternalAgent::ClaudeCode,
                native_id: "native-project".into(),
            };
            let opened = http.send_json(Method::POST, &history, &open).await?.require_success()?;
            ensure!(opened.headers["cache-control"] == "no-store");
            let WorkspaceResult::Opened { session_id: first } = opened.json()? else {
                anyhow::bail!("expected an ACP view");
            };
            ensure!(agena_domain::is_acp_session_id(first));
            let first_path = format!("/api/v1/acp/sessions/{first}");
            ensure!(http.get(&first_path).await?.status == StatusCode::OK);
            ensure!(
                http.get(&format!("/api/v1/acp/sessions/{}", ordinary.id())).await?.status
                    == StatusCode::NOT_FOUND
            );

            tokio::fs::write(case.workspace().join(".native-outside-text"), "A newer native answer").await?;
            let WorkspaceResult::Opened { session_id: refreshed } = http
                .send_json(Method::POST, &history, &open).await?.require_success()?.json()?
            else {
                anyhow::bail!("expected a refreshed ACP view");
            };
            ensure!(refreshed != first && http.get(&first_path).await?.status == StatusCode::NOT_FOUND);
            let user = case.user().attach_session(refreshed).await?;
            let mut actual = Vec::new();
            for part in user.parts().await?.iter().filter(|part| part.role == "assistant") {
                user.client().write_part_text(refreshed, part, &mut actual).await?;
            }
            ensure!(String::from_utf8(actual)?.contains("A newer native answer"));

            let created: agena_api::resource::SessionResource = http
                .send_json(Method::POST, "/api/v1/acp/sessions", &json!({
                    "workspace_id": workspace, "title": "In-memory view", "external_agent": "claude-code"
                }))
                .await?.require_success()?.json()?;
            let path = format!("/api/v1/acp/sessions/{}", created.id);
            let png = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGNgYPgPAAEDAQAIicLsAAAAAElFTkSuQmCC")?;
            let upload: agena_api::resource::WorkspaceFileUploadResource = http
                .send_bytes(
                    Method::POST,
                    &format!("{path}/attachments?filename=inline.png&mime=image/png"),
                    Some("application/octet-stream"),
                    png,
                )
                .await?.require_success()?.json()?;
            ensure!(upload.path.starts_with(agena_domain::ACP_INLINE_ATTACHMENT_PREFIX));
            let submitted: agena_api::resource::SessionExecutionResource = http.send_json(Method::POST, &format!("{path}/messages"), &json!({
                "document": [
                    {"type": "text", "text": "check in-memory upload"},
                    {"type": "activity", "activity": {
                        "id": agena_domain::ActivityId::new(),
                        "provenance": {"content_hash": upload.sha256},
                        "payload": {
                            "activity_type": "resource", "delivery": "model_input", "kind": "image",
                            "name": upload.name, "media_type": upload.mime, "size_bytes": upload.size_bytes,
                            "reference": {"reference_type": "url", "url": upload.path}
                        }
                    }}
                ]
            })).await?.require_success()?.json()?;
            let session = case.user().attach_session(created.id).await?;
            let run = session
                .attach_run(submitted.receipt.context("ACP turn receipt")?)
                .wait_settled()
                .await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("Reply from native-new")?;

            let list: serde_json::Value = http.get("/api/v1/sessions").await?.require_success()?.json()?;
            ensure!(list["items"].as_array().context("Agena session list")?.len() == 1);
            ensure!(list["items"][0]["id"] == ordinary.id());
            ensure!(!case.data_dir().join("session-assets").join(created.id.to_string()).exists());
            case.restart().await?;
            let http = case.http().await;
            ensure!(http.get(&path).await?.status == StatusCode::NOT_FOUND);
            ensure!(http.get(&format!("/api/v1/sessions/{}", ordinary.id())).await?.status == StatusCode::OK);
            let inspect = tokio::process::Command::new("python3")
                .arg("-c")
                .arg(concat!(
                    "import pathlib, sqlite3, sys\n",
                    "with sqlite3.connect(pathlib.Path(sys.argv[1]).resolve().as_uri() + '?mode=ro', uri=True) as db:\n",
                    "    print(*(db.execute('SELECT count(*) FROM agena_' + table).fetchone()[0]\n",
                    "            for table in ['sessions', 'entries', 'executions', 'blobs', 'content_streams']))\n",
                ))
                .arg(case.data_dir().join("agena.db"))
                .output().await?;
            ensure!(inspect.status.success(), "{}", String::from_utf8_lossy(&inspect.stderr));
            ensure!(String::from_utf8(inspect.stdout)?.trim() == "1 0 0 0 0");
            let WorkspaceResult::Opened { session_id: reopened } = http
                .send_json(Method::POST, &history, &open).await?.require_success()?.json()?
            else {
                anyhow::bail!("expected ACP history after restart");
            };
            ensure!(reopened != refreshed && reopened != created.id);
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_native_skills_are_scoped_to_the_current_agent_capabilities() -> TestResult {
    let (_bin, builder) = lifecycle_fixture("2")?;
    builder
        .seed_file("workspace/.native-skills", Vec::new())
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace = ordinary
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("workspace")?;
            let client = ordinary.client();
            let WorkspaceResult::Imported { session_id } = client
                .external_agent_workspace(
                    workspace,
                    WorkspaceAction::Import {
                        agent: ExternalAgent::ClaudeCode,
                        native_id: "native-project".into(),
                    },
                )
                .await?
            else {
                anyhow::bail!("expected native history");
            };
            let user = case.user().attach_session(session_id).await?;
            let commands = user.snapshot().await?.execution.external_agent_commands;
            let [skill, builtin, status] = commands.as_slice() else {
                anyhow::bail!("unexpected commands: {commands:?}");
            };
            ensure!(skill.name == "summarize-fixture" && skill.skill);
            let path = skill.skill_path.as_deref().context("agent skill path")?;
            ensure!(path.ends_with("/summarize-fixture/SKILL.md"));
            ensure!(builtin.skill && builtin.skill_path.is_none());
            ensure!(status.name == "status" && !status.skill);
            let parts = user.parts().await?;
            let historic_user = parts
                .iter()
                .find(|part| part.role == "user")
                .context("user replay")?;
            let mut text = Vec::new();
            client
                .write_part_text(session_id, historic_user, &mut text)
                .await?;
            ensure!(text == b"[Skill: summarize-fixture] Imported user");

            let run = user
                .send("/agent:summarize-fixture Summarize these notes")
                .await?
                .wait_settled()
                .await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("Structured skill accepted")?;
            let commands = user.snapshot().await?.execution.external_agent_commands;
            ensure!(
                matches!(commands.as_slice(), [skill] if skill.skill && skill.name == "live-skill")
            );

            tokio::fs::write(case.workspace().join(".native-disable-skills"), "").await?;
            let run = user
                .send("/agent:summarize-fixture legacy arguments")
                .await?
                .wait_settled()
                .await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("Text fallback accepted")?;
            let commands = user.snapshot().await?.execution.external_agent_commands;
            ensure!(
                matches!(commands.as_slice(), [status] if !status.skill && status.name == "status")
            );
            Ok(())
        })
        .await
}

fn lifecycle_fixture(
    version: &'static str,
) -> anyhow::Result<(tempfile::TempDir, agena_testkit::TestAgenaBuilder)> {
    let bin = tempfile::tempdir()?;
    let executable = bin.path().join("claude-agent-acp");
    std::fs::write(&executable, include_str!("plugins/fake_acp_lifecycle.py"))?;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
    let path = std::env::join_paths(std::iter::once(bin.path().to_owned()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))?;
    let builder = super::fixture()
        .configure_global(|config| config["sandbox"] = json!(false))
        .env("PATH", path)
        .seed_file(
            "workspace/.native-protocol-version",
            version.as_bytes().to_vec(),
        );
    Ok((bin, builder))
}

async fn exercise_native_lifecycle(version: &'static str) -> TestResult {
    let (_bin, builder) = lifecycle_fixture(version)?;
    builder
        .seed_file("workspace/extra/readme.md", b"Project context".to_vec())
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace = ordinary.snapshot().await?.session.workspace_id.context("workspace")?;
            let client = ordinary.client();
            let agent = ExternalAgent::ClaudeCode;
            let WorkspaceResult::Page { sessions, next_cursor } = client
                .external_agent_workspace(workspace, WorkspaceAction::List { agent, cursor: None })
                .await?
            else {
                anyhow::bail!("expected native history");
            };
            let expected_cursor = if version == "2" { "" } else { "page2" };
            ensure!(sessions.is_empty() && next_cursor.as_deref() == Some(expected_cursor));
            let next = client.external_agent_workspace(workspace, WorkspaceAction::List { agent, cursor: next_cursor }).await?;
            ensure!(matches!(next, WorkspaceResult::Page { sessions, next_cursor: None } if sessions.len() == 1));
            ensure!(client.external_agent_workspace(workspace, WorkspaceAction::Import {
                agent, native_id: "foreign".into(),
            }).await.is_err());

            let import = || client.external_agent_workspace(workspace, WorkspaceAction::Import {
                agent, native_id: "native-project".into(),
            });
            let WorkspaceResult::Imported { session_id } = import().await? else {
                anyhow::bail!("expected imported session");
            };
            let WorkspaceResult::Imported { session_id: duplicate } = import().await? else {
                anyhow::bail!("expected idempotent import");
            };
            ensure!(session_id == duplicate);

            let user = case.user().attach_session(session_id).await?;
            let snapshot = user.snapshot().await?;
            ensure!(snapshot.session.title == "Native history");
            if version == "2" {
                let details = &snapshot.execution.external_agent_details;
                let child = details.subagents.get("native-replay-child").context("imported child")?;
                ensure!(child.parent_id == "native-project" && child.state.as_deref() == Some("idle"));
                ensure!(child.messages.iter().any(|message| message.text == "Child history and a live continuation"));
                let nested = details.subagents.get("native-replay-descendant").context("imported descendant")?;
                ensure!(nested.parent_id == "native-replay-child" && nested.stop_reason.as_deref() == Some("end_turn"));
                ensure!(nested.messages.iter().any(|message| message.text == "Descendant history"));
                ensure!(details.terminals.iter().any(|terminal| terminal.session_id == "native-replay-descendant"
                    && terminal.output.contains("Historical child terminal")));
                ensure!(!details.subagents.contains_key("foreign-child"));
            }
            let parts = user.parts().await?;
            let mut assistant = Vec::new();
            let mut historical_user = Vec::new();
            let mut reasoning = Vec::new();
            for part in &parts {
                if part.role == "user" && part.kind == "text" {
                    client.write_part_text(session_id, part, &mut historical_user).await?;
                } else if part.role == "assistant" && part.kind == "text" {
                    client.write_part_text(session_id, part, &mut assistant).await?;
                } else if part.role == "assistant" && part.kind == "think" {
                    client.write_part_text(session_id, part, &mut reasoning).await?;
                }
            }
            ensure!(historical_user == b"Imported user", "{parts:?}");
            ensure!(assistant == b"Imported assistant", "{parts:?}");
            ensure!(reasoning == b"Imported reasoning", "{parts:?}");

            let message = parts.iter().find(|part| part.role == "user").context("imported user")?;
            let resources: Vec<_> = message.content["document"].as_array().context("imported document")?
                .iter().filter_map(|node| node.pointer("/activity/payload")).collect();
            ensure!(resources.len() == 4);
            ensure!(resources[0]["reference"]["url"] == "https://example.com/acp-history");
            let audio = resources[1]["reference"]["uri"].as_str().context("imported audio URI")?;
            let document = resources[2]["reference"]["uri"].as_str().context("imported text resource URI")?;
            let (_, wave) = client.download_session_media_file_with_limit(session_id, audio, 4096).await?;
            ensure!(resources[1]["media_type"] == "audio/wav" && wave.starts_with(b"RIFF"));
            let (_, bytes) = client.download_session_media_file_with_limit(session_id, document, 4096).await?;
            ensure!(bytes == b"Archived resource\n");
            let blob = resources[3]["reference"]["uri"].as_str().context("imported binary resource URI")?;
            let (_, bytes) = client.download_session_media_file_with_limit(session_id, blob, 4096).await?;
            ensure!(resources[3]["media_type"] == "application/octet-stream" && bytes == b"\x00\xff\x01");
            ensure!(client.download_session_media_file_with_limit(ordinary.id(), audio, 4096).await.is_err());

            let tool = parts.iter().find(|part| part.kind == "tool_call" && part.content["name"] == "Claude Code · Historical edit")
                .context("imported tool call")?;
            ensure!(!parts.iter().any(|part| part.content["name"] == "Claude Code · Live tool"));
            let input = client.session_tool_detail(session_id, tool.part_id, ToolDetailSection::Input).await?;
            ensure!(input.value["raw_input"]["path"] == "notes.txt");
            let result = client.session_tool_detail(session_id, tool.part_id, ToolDetailSection::Output).await?;
            ensure!(result.value["payload"]["text"] == "Saved edit");
            ensure!(result.value["payload"]["external_agent"]["output"]["saved"] == true);
            let image: AttachmentItem = serde_json::from_value(result.value["attachments"][0].clone())?;
            let (_, png) = user.download_attachment(&image).await?;
            ensure!(image.mime == "image/png" && png.starts_with(b"\x89PNG\r\n\x1a\n"));
            let presentation = client.session_tool_detail(session_id, tool.part_id, ToolDetailSection::Presentation).await?;
            ensure!(presentation.value["blocks"].as_array().context("tool result blocks")?.iter().any(|block| {
                block["type"] == "diff" && block["diff"].as_str().is_some_and(|diff| diff.contains("-before") && diff.contains("+after"))
            }));
            ensure!(parts.iter().filter(|part| part.content["name"] == "Claude Code · Attachment").count() == 1);

            let prompt = serde_json::from_value(json!({
                "order": ["lifecycle"],
                "inline": [{"id": "lifecycle", "body": "Use the workspace lifecycle instructions."}],
            }))?;
            client.set_session_prompts(session_id, Some(prompt)).await?;
            let SessionResult::Directories { directories } = client.external_agent_session(session_id,
                SessionAction::SetDirectories { directories: vec!["extra".into()] }).await?
            else { anyhow::bail!("expected additional directories"); };
            ensure!(directories.len() == 1 && directories[0].ends_with("extra"));

            let run = user.send("Continue imported conversation").await?.wait_settled().await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("Reply from native-project")?;
            let SessionResult::Status { session: Some(info), native_status, .. } =
                client.external_agent_session(session_id, SessionAction::Status).await?
            else { anyhow::bail!("expected a linked native session"); };
            ensure!(info.title.as_deref() == Some("Native generated title"));
            ensure!(native_status.as_deref() == Some("saved"));
            ensure!(matches!(client.external_agent_session(session_id, SessionAction::Close).await?, SessionResult::Closed));

            let SessionResult::Title { title } = client.external_agent_session(
                session_id, SessionAction::SetTitle { title: "Project".into() },
            ).await? else { anyhow::bail!("expected acknowledged title"); };
            ensure!(title == "Remote Project" && user.snapshot().await?.session.title == title);

            let SessionResult::Forked { session_id: fork_id } =
                client.external_agent_session(session_id, SessionAction::Fork { title: Some("Native branch".into()), before_message_id: None }).await?
            else { anyhow::bail!("expected native fork"); };
            let fork = case.user().attach_session(fork_id).await?;
            ensure!(fork.snapshot().await?.session.parent_id == Some(session_id));
            ensure!(fork.download_attachment(&image).await?.1 == png);
            ensure!(client.download_session_media_file_with_limit(fork_id, audio, 4096).await?.1 == wave);
            let run = fork.send("Continue fork").await?.wait_settled().await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("Reply from native-fork")?;
            ensure!(matches!(client.external_agent_session(fork_id, SessionAction::Delete).await?, SessionResult::Deleted));
            ensure!(client.external_agent_session(fork_id, SessionAction::Status).await.is_err());
            ensure!(fork.parts().await.is_err());
            let requests = tokio::fs::read_to_string(case.workspace().join(".native-protocol.log")).await?;
            ensure!(requests.contains("close-prompted native-project"), "{requests}");
            ensure!(requests.contains("session/fork native-project"));
            ensure!(requests.contains("session/resume native-fork"));
            ensure!(requests.contains("session/delete native-fork"));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_v2_historical_forks_copy_exact_anchored_prefix() -> TestResult {
    let (_bin, builder) = lifecycle_fixture("2")?;
    builder
        .seed_file("workspace/.historical-fork-support", b"1".to_vec())
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace = ordinary
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("workspace")?;
            let client = ordinary.client();
            let WorkspaceResult::Imported { session_id } = client
                .external_agent_workspace(
                    workspace,
                    WorkspaceAction::Import {
                        agent: ExternalAgent::ClaudeCode,
                        native_id: "native-project".into(),
                    },
                )
                .await?
            else {
                anyhow::bail!("expected imported conversation");
            };
            let user = case.user().attach_session(session_id).await?;
            let user_messages = |parts: Vec<PartResource>| {
                parts
                    .into_iter()
                    .filter(|part| part.role == "user" && part.kind == "text")
                    .filter_map(|part| part.message_id)
                    .collect::<Vec<_>>()
            };
            let original_messages = user_messages(user.parts().await?);
            let [first, imported] = original_messages[..] else {
                anyhow::bail!("expected two imported user messages");
            };
            let status = || client.external_agent_session(session_id, SessionAction::Status);
            let fork = |before_message_id| {
                client.external_agent_session(
                    session_id,
                    SessionAction::Fork {
                        title: None,
                        before_message_id: Some(before_message_id),
                    },
                )
            };
            let SessionResult::Status {
                capabilities,
                fork_points,
                ..
            } = status().await?
            else {
                anyhow::bail!("expected native status");
            };
            ensure!(capabilities.fork_before_message);
            ensure!(fork_points.len() == 1 && fork_points[0].message_id == imported);
            ensure!(fork_points[0].label == "Imported cursor target");

            let live = user
                .send("Live cursor target")
                .await?
                .wait_settled()
                .await?;
            live.assert_succeeded()?;
            let unmatched = user.send("Unmatched cursor").await?.wait_settled().await?;
            unmatched.assert_succeeded()?;
            let SessionResult::Status { fork_points, .. } = status().await? else {
                anyhow::bail!("expected native status after resume");
            };
            ensure!(
                fork_points
                    .iter()
                    .map(|point| point.message_id)
                    .collect::<Vec<_>>()
                    == [live.status.message_id, imported]
            );
            ensure!(fork_points[0].label == "Live cursor target");
            let assistant = live
                .parts
                .iter()
                .find(|part| part.role == "assistant")
                .context("live assistant")?;
            for rejected in [first, unmatched.status.message_id, assistant.part_id] {
                ensure!(
                    fork(rejected).await.is_err(),
                    "unanchored message {rejected}"
                );
            }

            let SessionResult::Forked {
                session_id: imported_fork,
            } = fork(imported).await?
            else {
                anyhow::bail!("expected branch at imported cursor");
            };
            let branch = case.user().attach_session(imported_fork).await?;
            ensure!(branch.snapshot().await?.session.parent_id == Some(session_id));
            let imported_prefix = branch.parts().await?;
            ensure!(
                imported_prefix
                    .iter()
                    .any(|part| part.content["name"] == "Claude Code · Historical edit")
            );
            ensure!(user_messages(imported_prefix) == [first]);
            let continuation = branch
                .send("Continue from imported cut")
                .await?
                .wait_settled()
                .await?;
            continuation.assert_succeeded()?;
            continuation.assert_assistant_text("Reply from native-fork-imported")?;
            let SessionResult::Status { fork_points, .. } = client
                .external_agent_session(imported_fork, SessionAction::Status)
                .await?
            else {
                anyhow::bail!("expected branch native status");
            };
            ensure!(
                fork_points.len() == 1
                    && fork_points[0].message_id == continuation.status.message_id
            );
            ensure!(
                fork(continuation.status.message_id).await.is_err(),
                "foreign user message"
            );

            let SessionResult::Forked {
                session_id: live_fork,
            } = fork(live.status.message_id).await?
            else {
                anyhow::bail!("expected branch at live cursor");
            };
            let branch = case.user().attach_session(live_fork).await?;
            ensure!(user_messages(branch.parts().await?) == [first, imported]);
            let continuation = branch
                .send("Continue from live cut")
                .await?
                .wait_settled()
                .await?;
            continuation.assert_succeeded()?;
            continuation.assert_assistant_text("Reply from native-fork-live")?;
            ensure!(
                user_messages(user.parts().await?)
                    == [
                        first,
                        imported,
                        live.status.message_id,
                        unmatched.status.message_id
                    ]
            );

            let wire =
                tokio::fs::read_to_string(case.workspace().join(".native-protocol.log")).await?;
            ensure!(
                wire.lines()
                    .filter(|line| *line == "session/fork native-project")
                    .count()
                    == 2,
                "{wire}"
            );
            ensure!(
                wire.contains("fork-cursor fixture/opaque:imported")
                    && wire.contains("fork-cursor fixture/opaque:live"),
                "{wire}"
            );
            ensure!(
                wire.contains("session/resume native-fork-imported")
                    && wire.contains("session/resume native-fork-live"),
                "{wire}"
            );
            ensure!(!wire.contains("session/new"), "{wire}");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn standard_session_archive_v1_preserves_saved_history() -> TestResult {
    exercise_standard_archive(ExternalAgent::Antigravity, "agy", "1").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn standard_session_archive_v2_preserves_saved_history() -> TestResult {
    exercise_standard_archive(ExternalAgent::ClaudeCode, "claude-agent-acp", "2").await
}

async fn exercise_standard_archive(
    agent: ExternalAgent,
    executable: &str,
    version: &'static str,
) -> TestResult {
    use agena_domain::ExternalAgentArchiveFilter as Filter;

    let bin = tempfile::tempdir()?;
    let path = bin.path().join(executable);
    std::fs::write(&path, include_str!("plugins/fake_acp_archive.py"))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    let path = std::env::join_paths(std::iter::once(bin.path().to_owned()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))?;
    super::fixture()
        .env("PATH", path)
        .seed_file("workspace/.archive-version", version.as_bytes().to_vec())
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace = ordinary.snapshot().await?.session.workspace_id.context("workspace")?;
            let client = ordinary.client();
            let list = |archived, cursor| client.external_agent_workspace(workspace,
                WorkspaceAction::IndexList { agent, archived, cursor });
            let archive = |native_id: &str, archived| client.external_agent_workspace(workspace,
                WorkspaceAction::SetArchived { agent, native_id: native_id.to_owned(), archived });

            let WorkspaceResult::IndexPage { sessions, next_cursor, capabilities } = list(Filter::All, None).await?
            else { anyhow::bail!("expected archive-capable history"); };
            ensure!(capabilities.archive && !capabilities.index && !capabilities.watch && !capabilities.batch_get);
            ensure!(sessions.len() == 1 && sessions[0].session.native_id == "saved" && sessions[0].archived == Some(true));
            ensure!(next_cursor.as_deref() == Some(if version == "2" { "" } else { "second/include" }));
            ensure!(matches!(list(Filter::All, next_cursor).await?, WorkspaceResult::IndexPage { sessions, next_cursor: None, .. }
                if sessions.len() == 1 && sessions[0].session.native_id == "ready" && sessions[0].archived == Some(false)));
            ensure!(matches!(client.external_agent_workspace(workspace, WorkspaceAction::List { agent, cursor: None }).await?,
                WorkspaceResult::Page { sessions, next_cursor: None } if sessions.len() == 1 && sessions[0].native_id == "ready"));

            ensure!(archive("foreign", false).await.is_err());
            ensure!(archive("missing", true).await.is_err());
            for _ in 0..2 {
                ensure!(matches!(archive("ready", true).await?, WorkspaceResult::Archive { native_id, archived: true } if native_id == "ready"));
            }
            let WorkspaceResult::IndexPage { sessions, next_cursor, .. } = list(Filter::Archived, None).await?
            else { anyhow::bail!("expected archived conversations"); };
            ensure!(sessions.len() == 1 && sessions[0].session.native_id == "saved");
            ensure!(matches!(list(Filter::Archived, next_cursor).await?, WorkspaceResult::IndexPage { sessions, next_cursor: None, .. }
                if sessions.len() == 1 && sessions[0].session.native_id == "ready" && sessions[0].archived == Some(true)));
            for _ in 0..2 {
                ensure!(matches!(archive("ready", false).await?, WorkspaceResult::Archive { native_id, archived: false } if native_id == "ready"));
            }
            ensure!(matches!(list(Filter::Unarchived, None).await?, WorkspaceResult::IndexPage { sessions, .. }
                if sessions.len() == 1 && sessions[0].session.native_id == "ready" && sessions[0].archived == Some(false)));

            let WorkspaceResult::Imported { session_id } = client.external_agent_workspace(workspace,
                WorkspaceAction::Import { agent, native_id: "saved".into() }).await?
            else { anyhow::bail!("expected archived conversation import"); };
            let imported = case.user().attach_session(session_id).await?;
            ensure!(imported.parts().await?.iter().any(|part| part.role == "user"));
            ensure!(matches!(list(Filter::Archived, None).await?, WorkspaceResult::IndexPage { sessions, .. }
                if sessions.len() == 1 && sessions[0].session.native_id == "saved"));
            ensure!(matches!(client.external_agent_session(session_id, SessionAction::Delete).await?, SessionResult::Deleted));
            ensure!(matches!(list(Filter::All, None).await?, WorkspaceResult::IndexPage { sessions, next_cursor: None, .. }
                if sessions.len() == 1 && sessions[0].session.native_id == "ready"));
            ensure!(archive("saved", false).await.is_err());

            let wire = tokio::fs::read_to_string(case.workspace().join(".archive-wire")).await?;
            ensure!(wire.lines().filter(|line| *line == "session/archive ready").count() == 1, "{wire}");
            ensure!(wire.lines().filter(|line| *line == "session/unarchive ready").count() == 1, "{wire}");
            ensure!(wire.lines().filter(|line| *line == "session/delete saved").count() == 1, "{wire}");
            ensure!(wire.contains("closed-active ready") && wire.contains("session/list include next") && wire.contains("session/list only next"), "{wire}");
            ensure!(!wire.lines().filter(|line| !line.starts_with("session/list ")).any(|line| line.contains("foreign") || line.contains("missing")), "{wire}");
            Ok(())
        }).await
}

fn index_fixture(
    version: &'static str,
) -> anyhow::Result<(tempfile::TempDir, agena_testkit::TestAgenaBuilder)> {
    let bin = tempfile::tempdir()?;
    let executable = bin.path().join("claude-agent-acp");
    std::fs::write(&executable, include_str!("plugins/fake_acp_index.py"))?;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
    let path = std::env::join_paths(std::iter::once(bin.path().to_owned()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))?;
    let builder = super::fixture()
        .env("PATH", path)
        .seed_file(
            "workspace/.ordinary-acp-version",
            version.as_bytes().to_vec(),
        )
        .seed_file("workspace/.watch-mode", b"early".to_vec());
    Ok((bin, builder))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_history_index_scopes_archive_batches_and_notifications() -> TestResult {
    use agena_domain::ExternalAgentArchiveFilter as Filter;

    let (_bin, builder) = index_fixture("1")?;
    builder
        .seed_file("workspace/.index-supported", b"1".to_vec())
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace = ordinary.snapshot().await?.session.workspace_id.context("workspace")?;
            let client = ordinary.client();
            let agent = ExternalAgent::ClaudeCode;
            let list = |archived, cursor| client.external_agent_workspace(workspace,
                WorkspaceAction::IndexList { agent, archived, cursor });
            let batch = |native_ids: &[&str]| client.external_agent_workspace(workspace,
                WorkspaceAction::BatchGet { agent, native_ids: native_ids.iter().map(|id| (*id).into()).collect() });
            let archive = |native_id: &str, archived| client.external_agent_workspace(workspace,
                WorkspaceAction::SetArchived { agent, native_id: native_id.into(), archived });
            let watch = || client.external_agent_workspace(workspace,
                WorkspaceAction::Watch { agent, wait_seconds: Some(1) });

            let WorkspaceResult::IndexPage { sessions, next_cursor, capabilities } = list(Filter::All, None).await?
            else { anyhow::bail!("expected indexed page"); };
            ensure!(capabilities.index && capabilities.archive && capabilities.batch_get && capabilities.watch);
            ensure!(sessions.len() == 1 && sessions[0].session.native_id == "active");
            ensure!(next_cursor.as_deref() == Some("indexed-page-2"));
            let second = list(Filter::All, next_cursor).await?;
            ensure!(matches!(second, WorkspaceResult::IndexPage { sessions, next_cursor: None, .. }
                if sessions.iter().map(|row| row.session.native_id.as_str()).collect::<Vec<_>>() == ["archived", "busy"]));

            let result = batch(&["active", "archived", "foreign", "missing"]).await?;
            ensure!(matches!(result, WorkspaceResult::Batch { sessions } if sessions.len() == 2
                && sessions[0].session.native_id == "active" && sessions[0].model.as_deref() == Some("fixture")
                && sessions[1].session.native_id == "archived" && sessions[1].archived == Some(true)));
            ensure!(batch(&["active", "active"]).await.is_err());
            ensure!(archive("foreign", true).await.is_err());
            ensure!(archive("busy", true).await.is_err());
            ensure!(matches!(archive("active", true).await?, WorkspaceResult::Archive { archived: true, .. }));
            ensure!(matches!(archive("active", true).await?, WorkspaceResult::Archive { archived: true, .. }));
            ensure!(matches!(list(Filter::Archived, None).await?, WorkspaceResult::IndexPage { sessions, .. }
                if sessions.iter().map(|row| row.session.native_id.as_str()).collect::<Vec<_>>() == ["active", "archived"]));
            ensure!(matches!(list(Filter::Unarchived, None).await?, WorkspaceResult::IndexPage { sessions, .. }
                if sessions.len() == 1 && sessions[0].session.native_id == "busy"));
            ensure!(matches!(archive("active", false).await?, WorkspaceResult::Archive { archived: false, .. }));

            ensure!(matches!(watch().await?, WorkspaceResult::Watch { supported: true, changed: true }));
            tokio::fs::write(case.workspace().join(".watch-mode"), "remove").await?;
            ensure!(matches!(watch().await?, WorkspaceResult::Watch { supported: true, changed: true }));
            tokio::fs::write(case.workspace().join(".watch-mode"), "quiet").await?;
            ensure!(matches!(watch().await?, WorkspaceResult::Watch { supported: true, changed: false }));

            let WorkspaceResult::Imported { session_id } = client.external_agent_workspace(workspace,
                WorkspaceAction::Import { agent, native_id: "archived".into() }).await?
            else { anyhow::bail!("expected archived conversation import"); };
            ensure!(matches!(client.external_agent_session(session_id, SessionAction::Delete).await?,
                SessionResult::Deleted));
            let wire = tokio::fs::read_to_string(case.workspace().join(".index-wire")).await?;
            ensure!(wire.lines().filter(|line| *line == "index _session/archive").count() == 1, "{wire}");
            ensure!(wire.lines().filter(|line| *line == "index _session/list/unsubscribe").count() == 3, "{wire}");
            ensure!(wire.contains("ordinary session/load") && wire.contains("ordinary session/delete"), "{wire}");
            ensure!(!wire.contains("index session/delete") && !wire.contains("index session/load"), "{wire}");
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_index_absence_falls_back_to_ordinary_configured_protocol() -> TestResult {
    use agena_domain::ExternalAgentArchiveFilter as Filter;

    let (_bin, builder) = index_fixture("2")?;
    builder.run(|case| async move {
        let ordinary = case.user().new_session().await?;
        let workspace = ordinary.snapshot().await?.session.workspace_id.context("workspace")?;
        let client = ordinary.client();
        let agent = ExternalAgent::ClaudeCode;
        let request = |action| client.external_agent_workspace(workspace, action);
        let fallback = request(WorkspaceAction::IndexList { agent, archived: Filter::Unarchived, cursor: None }).await?;
        ensure!(matches!(fallback, WorkspaceResult::IndexPage { sessions, capabilities, .. }
            if !capabilities.index && !capabilities.archive && !capabilities.batch_get && !capabilities.watch
                && sessions.len() == 1 && sessions[0].session.native_id == "active" && sessions[0].archived.is_none()));
        ensure!(request(WorkspaceAction::IndexList { agent, archived: Filter::Archived, cursor: None }).await.is_err());
        ensure!(request(WorkspaceAction::BatchGet { agent, native_ids: vec!["active".into()] }).await.is_err());
        ensure!(request(WorkspaceAction::SetArchived { agent, native_id: "active".into(), archived: true }).await.is_err());
        ensure!(matches!(request(WorkspaceAction::Watch { agent, wait_seconds: Some(1) }).await?,
            WorkspaceResult::Watch { supported: false, changed: false }));
        let wire = tokio::fs::read_to_string(case.workspace().join(".index-wire")).await?;
        ensure!(wire.contains("ordinary session/list") && !wire.contains("_session/"), "{wire}");
        Ok(())
    }).await
}
