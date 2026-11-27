use std::{collections::BTreeMap, os::unix::fs::PermissionsExt as _, time::Duration};

use agena_api::{
    commands::{Command, CommandResult, CreateSessionParams, ForkSessionParams},
    live::ToolDetailSection,
    part::PartResource,
};
use agena_domain::{
    ContentDocument, ContentKind, ContentPayload, EXTERNAL_AGENT_URL_OPEN_OPTION, ExternalAgent,
    ViewBlock,
};
use agena_testkit::{TestAgenaBuilder, TestResult, UserSession};
use anyhow::{Context as _, Result, ensure};

use super::fixture;

fn transport_fixture(agent: ExternalAgent) -> Result<(tempfile::TempDir, TestAgenaBuilder)> {
    let bin = tempfile::tempdir()?;
    let executable = bin.path().join(agent.command().0);
    std::fs::write(&executable, include_str!("plugins/fake_acp_transport.py"))?;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin.path().to_owned()).chain(std::env::split_paths(&path)),
    )?;
    let builder = fixture()
        .env("PATH", path)
        .configure_global(|config| config["sandbox"] = serde_json::json!(false))
        .seed_file(
            "workspace/native.txt",
            b"native fixture first line\nnative fixture second line\n".to_vec(),
        )
        .seed_file(
            "workspace/private.txt",
            b"not available to the ACP client callback".to_vec(),
        )
        .expect_plugin("agena.fs");
    Ok((bin, builder))
}

fn v1_callback_fixture() -> Result<(tempfile::TempDir, TestAgenaBuilder)> {
    let (bin, builder) = transport_fixture(ExternalAgent::ClaudeCode)?;
    Ok((
        bin,
        builder.seed_file("workspace/.acp-force-v1", Vec::new()),
    ))
}

async fn attach(case: &agena_testkit::TestCase, agent: ExternalAgent) -> Result<UserSession> {
    let ordinary = case.user().new_session().await?;
    let workspace_id = ordinary
        .snapshot()
        .await?
        .session
        .workspace_id
        .context("workspace ID")?;
    let created = ordinary
        .client()
        .create_session_configured(CreateSessionParams {
            workspace_id,
            title: "ACP transport".into(),
            external_agent: Some(agent),
            auto_title: false,
            parent_id: None,
            profile: None,
            prompts: None,
        })
        .await?;
    case.user().attach_session(created.id).await
}

fn native_reply_blocks(part: &PartResource) -> Result<&[ViewBlock]> {
    ensure!(part.kind == "text" && part.content["external_agent_document"] == true);
    Ok(&part
        .presentation
        .as_ref()
        .context("rich reply presentation")?
        .blocks)
}

async fn read_tool_preview(user: &UserSession, part: &PartResource) -> Result<ContentDocument> {
    let preview = part
        .presentation
        .as_ref()
        .and_then(|presentation| {
            presentation.blocks.iter().find_map(|block| match block {
                ViewBlock::Content { resource, .. } if resource.kind == ContentKind::Structured => {
                    Some(resource.resource_id)
                }
                _ => None,
            })
        })
        .context("native live tool document")?;
    let page = user
        .client()
        .read_content(user.id(), preview, None, 64 * 1024)
        .await?;
    let mut document = ContentDocument::default();
    for chunk in page.chunks {
        match chunk.payload {
            ContentPayload::StructuredSnapshot { document: next } => document = next,
            ContentPayload::Structured { event, .. } => {
                document = document.updated(&event).map_err(anyhow::Error::msg)?;
            }
            _ => {}
        }
    }
    Ok(document)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_replaces_same_id_rich_messages_and_preserves_forked_history() -> TestResult {
    let (_bin, builder) = transport_fixture(ExternalAgent::ClaudeCode)?;
    builder
        .run(|case| async move {
            let user = attach(&case, ExternalAgent::ClaudeCode).await?;
            let run = user.send("edit the same message").await?;
            run.wait_text_contains("Draft trailer").await?;
            let parts = user.parts().await?;
            let answer = parts
                .iter()
                .find(|part| {
                    part.content["text"]
                        .as_str()
                        .is_some_and(|text| text.starts_with("First draft"))
                })
                .context("first reply")?;
            let thought = parts
                .iter()
                .find(|part| part.kind == "think")
                .context("thought")?;
            let other = parts
                .iter()
                .find(|part| part.content["text"] == "Other message")
                .context("interleaved reply")?;
            let tool = parts
                .iter()
                .find(|part| {
                    part.kind == "tool_call"
                        && part.content["name"] == "Claude Code · Between messages"
                })
                .context("interleaved tool")?;
            let [
                ViewBlock::Markdown { .. },
                ViewBlock::Media {
                    artifact: old_asset,
                    ..
                },
                ViewBlock::Text { .. },
                ViewBlock::Markdown { .. },
            ] = native_reply_blocks(answer)?
            else {
                anyhow::bail!("unexpected first reply: {:?}", answer.presentation);
            };
            ensure!(old_asset.mime == "image/png");
            ensure!(answer.part_id < tool.part_id && tool.part_id < other.part_id);
            let fork = user
                .client()
                .command(Command::ForkSession(ForkSessionParams {
                    session_id: user.id(),
                    at_message_id: Some(answer.part_id),
                    title: None,
                    conversation_mode: None,
                    expected_version: None,
                }))
                .await?;
            let CommandResult::Execution(fork) = fork else {
                anyhow::bail!("unexpected fork result");
            };
            let branch = case.user().attach_session(fork.session.id).await?;

            tokio::fs::write(case.workspace().join(".acp-replace-message"), "").await?;
            run.wait_text_contains("Replacement observed").await?;
            let next = user.parts().await?;
            let current = next
                .iter()
                .find(|part| part.part_id == answer.part_id)
                .context("updated reply")?;
            ensure!(
                current.revision > answer.revision
                    && current.content["text"]
                        .as_str()
                        .is_some_and(|text| !text.contains("draft") && !text.contains("Old link"))
            );
            let [
                ViewBlock::Markdown { text: start, .. },
                ViewBlock::Text { text: link, .. },
                ViewBlock::Media {
                    artifact: new_asset,
                    ..
                },
                ViewBlock::Markdown { text: end, .. },
            ] = native_reply_blocks(current)?
            else {
                anyhow::bail!("unexpected replacement: {:?}", current.presentation);
            };
            ensure!(start == "Final opener" && end == "Final trailer");
            ensure!(link.contains("https://example.invalid/new") && new_asset.mime == "audio/wav");
            ensure!(
                next.iter()
                    .find(|part| part.part_id == thought.part_id)
                    .context("cleared thought")?
                    .content["raw"][0]
                    == ""
            );
            ensure!(
                next.iter()
                    .find(|part| part.part_id == other.part_id)
                    .context("cleared reply")?
                    .content["text"]
                    == ""
            );

            tokio::fs::write(case.workspace().join(".acp-finish-edit"), "").await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_text_contains("Final trailerAfter reset")?;
            let saved = user.parts().await?;
            let saved_answer = saved
                .iter()
                .find(|part| part.part_id == answer.part_id)
                .context("stored reply")?;
            ensure!(saved_answer.presentation == current.presentation);
            ensure!(
                saved
                    .iter()
                    .find(|part| part.part_id == thought.part_id)
                    .context("continued thought")?
                    .content["raw"][0]
                    == "Continued thought"
            );
            let frozen = branch.parts().await?;
            let frozen_answer = frozen
                .iter()
                .find(|part| part.part_id == answer.part_id)
                .context("forked reply")?;
            ensure!(
                frozen_answer.presentation == answer.presentation
                    && frozen_answer.content["text"] == answer.content["text"]
            );
            ensure!(
                branch
                    .client()
                    .download_session_media_file(branch.id(), &old_asset.uri)
                    .await?
                    .1
                    .starts_with(b"\x89PNG")
            );
            ensure!(
                user.client()
                    .download_session_media_file(user.id(), &new_asset.uri)
                    .await?
                    .1
                    .starts_with(b"RIFF")
            );
            ensure!(
                branch
                    .client()
                    .download_session_media_file(branch.id(), &new_asset.uri)
                    .await
                    .is_err()
            );

            tokio::fs::write(case.workspace().join(".acp-edit-while-parked"), "").await?;
            let parked = user
                .wait_state("ACP parked message edit", |state| {
                    state
                        .execution
                        .external_agent_commands
                        .iter()
                        .any(|command| command.name == "parked-edit")
                })
                .await?;
            let messages = &parked.execution.external_agent_details.messages;
            ensure!(messages.iter().any(|message| message.id == "idle-only"));
            ensure!(messages.iter().all(|message| message.id != "answer"));
            let parked_parts = user.parts().await?;
            let parked_answer = parked_parts
                .iter()
                .find(|part| part.part_id == answer.part_id)
                .context("parked reply entry")?;
            ensure!(parked_answer.revision > saved_answer.revision);
            ensure!(parked_answer.content["text"] == "Reply revised while parked");
            ensure!(native_reply_blocks(parked_answer)?.iter().any(|block| {
                matches!(block, ViewBlock::Media { artifact, .. } if artifact.uri == old_asset.uri)
            }));
            ensure!(
                parked_parts
                    .iter()
                    .find(|part| part.part_id == thought.part_id)
                    .context("parked thought entry")?
                    .content["raw"][0]
                    == ""
            );

            let reconnect = user.send("resume edited message").await?;
            reconnect
                .wait_text_contains("Resume reached a new prompt")
                .await?;
            let reopened = user.parts().await?;
            let reopened_answer = reopened
                .iter()
                .find(|part| part.part_id == answer.part_id)
                .context("reconnected reply entry")?;
            ensure!(
                reopened_answer.content["text"]
                    == "Reply revised on reconnectAppended during resume"
            );
            ensure!(native_reply_blocks(reopened_answer)?.iter().any(|block| {
                matches!(block, ViewBlock::Media { artifact, .. } if artifact.uri == new_asset.uri)
            }));
            ensure!(
                !reopened
                    .iter()
                    .any(|part| part.content["text"] == "Unattached reply during reconnect")
            );
            let resumed = user.snapshot().await?.execution;
            ensure!(
                resumed
                    .external_agent_details
                    .messages
                    .iter()
                    .any(|message| message.id == "resume-only")
            );
            ensure!(resumed.external_agent_commands.is_empty());
            ensure!(
                resumed
                    .external_agent_options
                    .as_ref()
                    .is_none_or(|config| config.options.is_empty())
            );

            tokio::fs::write(case.workspace().join(".acp-finish-resume-edit"), "").await?;
            reconnect.wait_settled().await?.assert_succeeded()?;
            let later = user.parts().await?;
            ensure!(
                later
                    .iter()
                    .find(|part| part.part_id == answer.part_id)
                    .context("edited reply entry in the later turn")?
                    .content["text"]
                    == "Reply revised on reconnectAppended during resumeAppended in the second turn"
            );
            ensure!(
                later
                    .iter()
                    .find(|part| part.part_id == thought.part_id)
                    .context("continued thought entry in the later turn")?
                    .content["raw"][0]
                    == "Thought continued after reconnect"
            );
            let result = user
                .send("continue after editing old history")
                .await?
                .wait_settled()
                .await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("Third turn resumed")?;

            let frozen = branch.parts().await?;
            let frozen_answer = frozen
                .iter()
                .find(|part| part.part_id == answer.part_id)
                .context("forked reply after later edits")?;
            ensure!(frozen_answer.presentation == answer.presentation);
            ensure!(
                branch
                    .client()
                    .download_session_media_file(branch.id(), &new_asset.uri)
                    .await
                    .is_err()
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_waits_for_foreground_and_background_and_accepts_steering_and_mcp() -> TestResult {
    let (_bin, builder) = transport_fixture(ExternalAgent::ClaudeCode)?;
    builder
        .run(|case| async move {
            let user = attach(&case, ExternalAgent::ClaudeCode).await?;
            let run = user.send("start preview").await?;
            run.wait_text_contains("Preview is still running").await?;
            ensure!(!run.status().await?.phase.is_terminal());
            user.client()
                .steer_external_agent(user.id(), "Please inspect the fixture")
                .await?;
            run.wait_text_contains("Foreground ended").await?;
            run.wait_text_contains("Background is still pending").await?;
            ensure!(!run.status().await?.phase.is_terminal());
            let active = user.snapshot().await?;
            ensure!(active.execution.external_agent_commands.iter().map(|cmd| cmd.name.as_str()).eq(["setup-command"]));
            ensure!(active.execution.external_agent_commands[0].argument_hint.is_none());
            let options = active.execution.external_agent_options.as_ref().context("setup response options")?;
            ensure!(options.options.iter().map(|option| option.id.as_str()).eq(["setup-setting"]));
            let details = &active.execution.external_agent_details;
            ensure!(details.suggestions.len() == 1);
            let suggestion = details.suggestions[0].composer_document();
            ensure!(details.terminals.len() == 2);
            let terminal = details
                .terminals
                .iter()
                .find(|item| item.session_id == "native-transport")
                .context("root terminal")?;
            ensure!(terminal.command.as_deref() == Some("build fixture") && terminal.exited);
            ensure!(
                terminal.exit_code == Some(0) && terminal.output == "Terminal ☃",
                "{terminal:?}"
            );
            let child = details
                .terminals
                .iter()
                .find(|item| item.session_id == "native-terminal-child")
                .context("child terminal")?;
            ensure!(child.id == terminal.id && child.output == "Worker output");
            let parts = user.parts().await?;
            let tool = parts
                .iter()
                .find(|part| {
                    part.kind == "tool_call" && part.content["name"] == "Claude Code · Background"
                })
                .context("running ACP tool")?;
            ensure!(tool.state == "running");
            let live_output = user
                .client()
                .session_tool_detail(user.id(), tool.part_id, ToolDetailSection::Output)
                .await?;
            ensure!(
                live_output.part_state.as_deref() == Some("running") && live_output.value.is_null()
            );
            let document = read_tool_preview(&user, tool).await?;
            ensure!(
                document
                    .blocks
                    .iter()
                    .any(|block| block.text_value() == Some("Live preview.")),
                "{document:?}"
            );
            ensure!(
                document.blocks.iter().any(|block| matches!(
                    block, ViewBlock::Log { text, .. } if text == "Terminal ☃"
                )),
                "{document:?}"
            );
            ensure!(document.blocks.iter().any(|block| matches!(
                block, ViewBlock::Diff { diff, .. } if diff.contains("-old") && diff.contains("+new")
            )));
            ensure!(document.blocks.iter().any(|block| matches!(block,
                ViewBlock::Table { rows, .. } if rows.iter().any(|row| row[0] == "copy"
                    && row[1].as_str().is_some_and(|path| path.ends_with("/asset-copy.png"))
                    && row[2].as_str().is_some_and(|path| path.ends_with("/asset.png")))
            )));
            tokio::fs::write(case.workspace().join(".acp-finish-job"), "").await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_text_contains("Background finished")?;
            let completed = user
                .client()
                .session_tool_detail(user.id(), tool.part_id, ToolDetailSection::Output)
                .await?;
            ensure!(completed.part_state.as_deref() == Some("completed"));
            ensure!(
                completed.revision > live_output.revision
                    && completed.value["payload"]["text"] == "Live preview."
                    && completed.value["payload"]["external_agent"]["content"][0]["output"]
                        == "Terminal ☃"
            );
            tokio::fs::write(case.workspace().join(".acp-after-settlement"), "").await?;
            user.wait_state(
                "ACP passive command update and suggestion withdrawal after settlement",
                |state| {
                    state
                        .execution
                        .external_agent_commands
                        .iter()
                        .any(|command| command.name == "passive")
                        && state
                            .execution
                            .external_agent_details
                            .suggestions
                            .is_empty()
                },
            )
            .await?;
            let result = user
                .send_document(suggestion, Default::default())
                .await?
                .wait_settled()
                .await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("Preview resumed")?;
            ensure!(case.model().requests().is_empty());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_permission_subject_updates_the_existing_tool_before_answer() -> TestResult {
    let (_bin, builder) = transport_fixture(ExternalAgent::ClaudeCode)?;
    builder
        .run(|case| async move {
            let user = attach(&case, ExternalAgent::ClaudeCode).await?;
            let run = user.send("authorize native edit").await?;
            let pending = run.wait_user_input().await?;
            let parts = user.parts().await?;
            let tool = parts
                .iter()
                .find(|part| {
                    part.kind == "tool_call" && part.content["name"] == "Claude Code · Pending edit"
                })
                .context("pending native edit")?;
            let preview = read_tool_preview(&user, tool).await?;
            ensure!(preview.blocks.iter().any(|block| matches!(block,
            ViewBlock::Diff { diff, .. } if diff.contains("-earlier\n+approved"))));
            ensure!(
                !preview
                    .blocks
                    .iter()
                    .any(|block| block.text_value() == Some("Earlier preview"))
            );

            pending
                .answer(BTreeMap::from([("0".into(), vec!["Allow".into()])]))
                .await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("Approved the edit")?;
            ensure!(
                result
                    .parts
                    .iter()
                    .any(|part| part.part_id == tool.part_id && part.state == "completed")
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_resume_serves_bound_mcp_and_keeps_permission_pending_into_the_turn() -> TestResult {
    let (_bin, builder) = transport_fixture(ExternalAgent::ClaudeCode)?;
    builder
        .seed_file("workspace/.acp-resume-callbacks", Vec::new())
        .run(|case| async move {
            let user = attach(&case, ExternalAgent::ClaudeCode).await?;
            user.send("start work to recover")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("Ready to resume")?;
            let run = user.send("continue recovered work").await?;
            let pending = run.wait_user_input().await?;
            pending
                .answer(BTreeMap::from([("0".into(), vec!["Allow".into()])]))
                .await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("Returned from resumed callbacks")?;
            ensure!(case.model().requests().is_empty());
            Ok(())
        })
        .await
}

async fn withdraw_tool_requests(v1: bool) -> TestResult {
    let agent = ExternalAgent::ClaudeCode;
    let (_bin, builder) = if v1 {
        v1_callback_fixture()
    } else {
        transport_fixture(agent)
    }?;
    builder
        .run(move |case| async move {
            let user = attach(&case, agent).await?;
            user.client()
                .set_session_permission(
                    user.id(),
                    serde_json::from_value(serde_json::json!({
                        "tools": {"default": "allow", "names": {"agena.fs.write": "ask"}},
                        "path": {"workspace": "readwrite"},
                    }))?,
                )
                .await?;
            let run = user.send("withdraw tool requests").await?;
            let approval = run.wait_permission().await?;
            tokio::fs::write(case.workspace().join(".acp-withdraw-permission"), "").await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("Another tool completed after the withdrawals")?;
            ensure!(approval.allow_once().await.is_err());
            for name in if v1 {
                &["fs.write"][..]
            } else {
                &["agena.fs.write", "agena.shell.exec"][..]
            } {
                ensure!(
                    result.parts.iter().any(|part| part.kind == "tool_call"
                        && part.content["name"] == *name
                        && part.state == "cancelled"),
                    "expected {name} to be cancelled: {:?}",
                    result.parts,
                );
            }
            ensure!(
                tokio::fs::read_to_string(case.workspace().join("native.txt")).await?
                    == "native fixture first line\nnative fixture second line\n"
            );
            ensure!(!case.workspace().join(".acp-shell-effect").exists());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_withdrawing_a_client_file_request_cancels_its_approval() -> TestResult {
    withdraw_tool_requests(true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_withdrawing_an_mcp_request_cancels_the_exact_tool() -> TestResult {
    withdraw_tool_requests(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_lost_connection_settles_the_pending_client_write() -> TestResult {
    let agent = ExternalAgent::ClaudeCode;
    let (_bin, builder) = v1_callback_fixture()?;
    builder
        .run(move |case| async move {
            let user = attach(&case, agent).await?;
            user.client()
                .set_session_permission(
                    user.id(),
                    serde_json::from_value(serde_json::json!({
                        "tools": {"default": "allow", "names": {"agena.fs.write": "ask"}},
                        "path": {"workspace": "readwrite"},
                    }))?,
                )
                .await?;
            let run = user.send("disconnect during a client write").await?;
            let approval = run.wait_permission().await?;
            tokio::fs::write(case.workspace().join(".acp-disconnect"), "").await?;
            let result = run.wait_settled().await?;
            result.assert_failed()?;
            ensure!(result.parts.iter().any(|part| {
                part.kind == "tool_call"
                    && part.content["name"] == "fs.write"
                    && part.state == "cancelled"
                    && part.content["error"]["failure"]["code"] == "internal.cancelled_as_failure"
            }));
            ensure!(approval.allow_once().await.is_err());
            ensure!(
                tokio::fs::read_to_string(case.workspace().join("native.txt")).await?
                    == "native fixture first line\nnative fixture second line\n"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_peer_uses_policy_scoped_file_and_terminal_callbacks() -> TestResult {
    let (_bin, builder) = v1_callback_fixture()?;
    builder
        .run(|case| async move {
            let user = attach(&case, ExternalAgent::ClaudeCode)
                .await?
                .with_timeout(Duration::from_secs(40));
            user.client().set_session_permission(
            user.id(),
            serde_json::from_value(serde_json::json!({
                "tools": {"default": "allow"},
                "path": {
                    "workspace": {"read": "allow", "write": "allow"},
                    "rules": {(case.workspace().join("private.txt").to_string_lossy()): "deny"}
                },
            }))?,
        ).await?;
            let run = user.send("client callbacks").await?;
            run.wait_text_contains("Checking local callbacks").await?;
            user.client()
                .steer_external_agent(user.id(), "Check the allowed file and terminal")
                .await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_text_contains("File and terminal callbacks finished")?;
            ensure!(
                tokio::fs::read_to_string(case.workspace().join("native.txt")).await?
                    == "client rewrite\n"
            );
            ensure!(case.model().requests().is_empty());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_authorization_completes_after_the_turn_settles() -> TestResult {
    let (_bin, builder) = transport_fixture(ExternalAgent::Antigravity)?;
    builder
        .seed_file("workspace/.acp-setup-authorization", Vec::new())
        .run(|case| async move {
            let user = attach(&case, ExternalAgent::Antigravity).await?;
            let run = user.send("check setup authorization").await?;
            let request = run.wait_user_input().await?;
            let input = request
                .request
                .request
                .as_user_input()
                .context("setup authorization")?;
            ensure!(input.questions[0].question == "https://example.invalid/setup");
            request
                .answer(BTreeMap::from([(
                    "0".into(),
                    vec![EXTERNAL_AGENT_URL_OPEN_OPTION.into()],
                )]))
                .await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("Setup authorization accepted")?;
            tokio::fs::write(case.workspace().join(".acp-after-settlement"), "").await?;
            let state = user
                .wait_state("ACP authorization completion after settlement", |state| {
                    state
                        .execution
                        .external_agent_commands
                        .iter()
                        .any(|command| command.name == "passive")
                })
                .await?;
            let notices = &state.execution.external_agent_details.notices;
            ensure!(notices.len() == 2, "{notices:?}");
            ensure!(
                notices[0].title == "Authorization completed"
                    && notices[1].title == "Idle fixture marker"
            );
            Ok(())
        })
        .await
}
