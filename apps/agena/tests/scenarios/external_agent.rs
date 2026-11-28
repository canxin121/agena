use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

use agena_api::{
    Scope,
    commands::CreateSessionParams,
    live::ToolDetailSection,
    resource::{SessionUsageLimitBasis, UserInputReplyKind},
};
use agena_client::SubscriptionEvent;
use agena_domain::{
    ActivityId, ComposerDocument, ExternalAgent, ExternalAgentOptionChange, ExternalAgentOptionKind,
};
use agena_testkit::{Reply, RequestMatch, TestAgenaBuilder, TestResult, UserSession};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use super::fixture;

fn peers() -> Result<(tempfile::TempDir, TestAgenaBuilder)> {
    let bin = tempfile::tempdir()?;
    for executable in ["agy", "claude-agent-acp"] {
        let path = bin.path().join(executable);
        std::fs::write(&path, include_str!("plugins/fake_acp.py"))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin.path().to_owned()).chain(std::env::split_paths(&path)),
    )?;
    let builder = fixture()
        // Exercise runtime permissions on hosts that cannot start an MXC worker.
        .configure_global(|config| config["sandbox"] = json!(false))
        .env("PATH", path)
        .env(
            "AGENA_TEST_NEVER_FORWARD",
            "test fixture private environment",
        )
        .env("CLAUDE_CODE_EXECUTABLE", "use bundled Claude Code instead")
        .expect_plugin("agena.fs")
        .seed_file(
            "workspace/acp-input.txt",
            b"acp callback content\n".to_vec(),
        )
        .seed_file(
            "workspace/acp-image.png",
            STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGNgYPgPAAEDAQAIicLsAAAAAElFTkSuQmCC")?,
        );
    Ok((bin, builder))
}

fn composer() -> Result<ComposerDocument> {
    Ok(serde_json::from_value(json!([
        {"type": "text", "text": "check composer"},
        {"type": "activity", "activity": {"id": ActivityId::new(), "payload": {
            "activity_type": "resource", "delivery": "model_input", "kind": "image",
            "name": "acp-image.png", "reference": {"reference_type": "workspace_path", "path": "acp-image.png"},
        }}},
        {"type": "activity", "activity": {"id": ActivityId::new(), "payload": {
            "activity_type": "resource", "kind": "file", "name": "acp-input.txt",
            "reference": {"reference_type": "workspace_path", "path": "acp-input.txt"},
        }}},
        {"type": "activity", "activity": {"id": ActivityId::new(), "payload": {
            "activity_type": "command_reference", "name": "inspect", "skill_id": "example@1/inspect",
            "source": "agena.skills", "content_hash": "1234",
        }}},
    ]))?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acp_v1_interruption_preserves_the_native_link_without_resubmitting() -> TestResult {
    recover_interrupted_acp_prompt("1", "before-response", "external_agent.prompt_uncertain", 1)
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acp_v2_retries_an_uninserted_prompt_with_its_original_key() -> TestResult {
    recover_interrupted_acp_prompt("2", "before-insertion", "", 2).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acp_v2_reconciles_a_lost_acknowledgment_in_native_history() -> TestResult {
    recover_interrupted_acp_prompt("2", "before-response", "external_agent.prompt_accepted", 2)
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acp_v2_reconciles_a_lost_acknowledgment_after_a_successful_markerless_replay() -> TestResult
{
    recover_interrupted_acp_prompt(
        "2",
        "markerless-replay",
        "external_agent.prompt_accepted",
        2,
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acp_v2_retains_an_acknowledged_prompt_without_resubmitting() -> TestResult {
    recover_interrupted_acp_prompt("2", "after-response", "external_agent.prompt_accepted", 1).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acp_v1_retries_after_an_explicit_preflight_rejection() -> TestResult {
    recover_interrupted_acp_prompt("1", "explicit-rejection", "", 2).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acp_v2_retries_after_an_explicit_preflight_rejection_without_replay() -> TestResult {
    recover_interrupted_acp_prompt("2", "explicit-rejection", "", 2).await
}

async fn recover_interrupted_acp_prompt(
    version: &'static str,
    mode: &'static str,
    retry_error: &'static str,
    submissions: usize,
) -> TestResult {
    let bin = tempfile::tempdir()?;
    let peer = bin.path().join("claude-agent-acp");
    std::fs::write(&peer, include_str!("plugins/fake_acp_recovery.py"))?;
    std::fs::set_permissions(&peer, std::fs::Permissions::from_mode(0o755))?;
    let path = std::env::join_paths(std::iter::once(bin.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))?;
    fixture()
        .env("PATH", path)
        .seed_file("workspace/.recovery-version", version.as_bytes().to_vec())
        .seed_file("workspace/.recovery-mode", mode.as_bytes().to_vec())
        .run(|case| async move {
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
                    external_agent: Some(ExternalAgent::ClaudeCode),
                    workspace_id,
                    title: "Recover native submission".into(),
                    parent_id: None,
                    auto_title: false,
                    profile: None,
                    prompts: None,
                })
                .await?;
            let user = case.user().attach_session(created.id).await?;
            user.send("warm up")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            let failed = user
                .send("interrupted prompt")
                .await?
                .wait_settled()
                .await?;
            failed.assert_failed()?;
            let retry = user
                .continue_run(Default::default())
                .await?
                .wait_settled()
                .await?;
            if retry_error.is_empty() {
                retry.assert_succeeded()?;
            } else {
                retry.assert_failed()?;
                let parts = user.parts().await?;
                ensure!(
                    parts.iter().any(|part| part.kind == "error"
                        && part.message_id == Some(retry.status.message_id)
                        && part.content["problem"]["code"] == retry_error),
                    "{parts:?}"
                );
            }
            ensure!(retry.status.message_id == failed.status.message_id);
            let followup = user.send("follow up").await?.wait_settled().await?;
            followup.assert_succeeded()?;
            followup.assert_assistant_text("Continued on the original conversation")?;
            let wire =
                tokio::fs::read_to_string(case.workspace().join(".recovery-wire.jsonl")).await?;
            let requests = wire
                .lines()
                .map(serde_json::from_str::<Value>)
                .collect::<Result<Vec<_>, _>>()?;
            ensure!(
                requests
                    .iter()
                    .all(|request| request["id"] == "recovery-native")
            );
            let retries = requests
                .iter()
                .filter(|request| request["input"] == "interrupted prompt")
                .collect::<Vec<_>>();
            ensure!(retries.len() == submissions, "{wire}");
            ensure!(
                retries
                    .iter()
                    .all(|request| request["key"] == retries[0]["key"])
            );
            ensure!(retries[0]["key"].is_string() == (version == "2"));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_request_limit_preserves_native_continuation() -> TestResult {
    let (_bin, builder) = peers()?;
    builder
        .run(|case| async move {
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
                    external_agent: Some(ExternalAgent::Antigravity),
                    workspace_id,
                    title: "ACP request limit".into(),
                    parent_id: None,
                    auto_title: false,
                    profile: None,
                    prompts: None,
                })
                .await?;
            let user = case.user().attach_session(created.id).await?;
            let limited = user
                .send("reach native request limit")
                .await?
                .wait_settled()
                .await?;
            limited.assert_succeeded()?;
            limited.assert_text_contains("Partial native answer")?;
            ensure!(limited.parts.iter().any(|part| {
                part.kind == "notice" && part.content["kind"] == "external_agent_turn_limit"
            }));
            let continued = user
                .send("continue after native request limit")
                .await?
                .wait_settled()
                .await?;
            continued.assert_succeeded()?;
            continued.assert_text_contains("Continued native answer")?;
            Ok(())
        })
        .await
}

async fn read_policy(user: &UserSession, path: PathBuf, mode: &str) -> Result<()> {
    user.client()
        .set_session_permission(
            user.id(),
            serde_json::from_value(json!({
                "tools": {"default": "allow"},
                "path": {"rules": { (path.to_string_lossy()): mode }}
            }))?,
        )
        .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projects_acp_rich_content_and_session_activity() -> TestResult {
    let (_bin, builder) = peers()?;
    let wav = b"RIFF\x26\x00\x00\x00WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00\x40\x1f\x00\x00\x80\x3e\x00\x00\x02\x00\x10\x00data\x02\x00\x00\x00\x00\x00";
    builder
        .seed_file("workspace/acp-audio.wav", wav.to_vec())
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let id = ordinary
                .client()
                .create_session_configured(CreateSessionParams {
                    external_agent: Some(ExternalAgent::ClaudeCode),
                    workspace_id: ordinary
                        .snapshot()
                        .await?
                        .session
                        .workspace_id
                        .context("workspace")?,
                    title: "ACP activity".into(),
                    parent_id: None,
                    auto_title: false,
                    profile: None,
                    prompts: None,
                })
                .await?
                .id;
            let user = case.user().attach_session(id).await?;
            user.client()
                .set_session_permission(
                    id,
                    serde_json::from_value(json!({"tools":{"default":"allow"},"path":{"rules":{
                        (case.workspace().join("acp-input.txt").to_string_lossy()):"readonly",
                        (case.workspace().join("acp-audio.wav").to_string_lossy()):"readonly"
                    }}}))?,
                )
                .await?;
            let activity = |kind, path| {
                json!({"type":"activity","activity":{"id":ActivityId::new(),"payload":{
                    "activity_type":"resource","delivery":"model_input","kind":kind,"name":path,
                    "reference":{"reference_type":"workspace_path","path":path},
                }}})
            };
            let document = serde_json::from_value(json!([
                {"type":"text","text":"check rich projection"},
                activity("file","acp-input.txt"),activity("audio","acp-audio.wav"),
            ]))?;
            let run = user
                .send_document(document, Default::default())
                .await?
                .wait_settled()
                .await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("rich projection accepted")?;
            let details = user.snapshot().await?.execution.external_agent_details;
            let cost = details.cost.context("native cost")?;
            ensure!(cost.amount.as_f64() == Some(1.25) && cost.currency == "credits");
            let billed = user.client().session_cost_summary(id).await?;
            ensure!(billed.external_agent_cost == Some(cost) && billed.recorded_cost_usd == 0.0);
            ensure!(
                details
                    .notices
                    .iter()
                    .any(|notice| notice.title == "Native notice")
            );
            let compact = details
                .compactions
                .iter()
                .find(|item| item.id == "compact")
                .context("compaction")?;
            ensure!(compact.status == "completed" && compact.summary == "Captured summary.");
            let child = details
                .subagents
                .get("native-child")
                .context("native subagent")?;
            ensure!(child.state.as_deref() == Some("idle") && child.messages.len() == 2);
            ensure!(
                child.messages[0].text == "Inspect the fixture"
                    && child.messages[1].text == "Subagent response"
            );

            let part = run
                .parts
                .iter()
                .find(|part| {
                    part.kind == "tool_call" && part.content["name"] == "Claude Code · Native edit"
                })
                .context("native edit")?;
            let input = user
                .client()
                .session_tool_detail(id, part.part_id, ToolDetailSection::Input)
                .await?;
            ensure!(
                input.value["raw_input"]["scope"] == "test",
                "{:?}",
                input.value
            );
            ensure!(input.value["locations"][0]["line"] == 7);
            let output = user
                .client()
                .session_tool_detail(id, part.part_id, ToolDetailSection::Output)
                .await?;
            ensure!(output.value["payload"]["external_agent"]["output"]["modified"] == true);
            ensure!(output.value["attachments"][0]["mime"] == "image/png");
            let human = user
                .client()
                .session_tool_detail(id, part.part_id, ToolDetailSection::Presentation)
                .await?;
            let blocks = human.value["blocks"].as_array().context("tool blocks")?;
            ensure!(blocks.iter().any(|block| {
                block["type"] == "diff"
                    && block["diff"]
                        .as_str()
                        .is_some_and(|diff| diff.contains("-before") && diff.contains("+after"))
            }));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_forms_stream_and_withdraw_independently_of_the_turn() -> TestResult {
    let (_bin, builder) = peers()?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let created = ordinary
                .client()
                .create_session_configured(CreateSessionParams {
                    external_agent: Some(ExternalAgent::ClaudeCode),
                    workspace_id: ordinary
                        .snapshot()
                        .await?
                        .session
                        .workspace_id
                        .context("workspace")?,
                    title: "ACP interactions".into(),
                    parent_id: None,
                    auto_title: false,
                    profile: None,
                    prompts: None,
                })
                .await?;
            let user = case.user().attach_session(created.id).await?;

            let pending = user.send("check elicitation").await?;
            let request = pending.wait_user_input().await?;
            let form = request
                .request
                .request
                .as_user_input()
                .context("ACP form")?;
            ensure!(form.body_markdown == "Choose execution settings.");
            ensure!(form.questions.len() == 7 && form.questions[6].optional);
            ensure!(form.questions[1].multiple && !form.questions[1].optional);
            ensure!(form.questions[0].options[0].label == "Europe");
            pending
                .wait_text_contains("streamed while the form was open")
                .await?;
            let mut answers = BTreeMap::from([
                ("0".into(), vec!["Europe".into()]),
                ("1".into(), vec!["Logs".into(), "Statistics".into()]),
                ("2".into(), vec!["9".into()]),
                ("3".into(), vec!["0.25".into()]),
                ("4".into(), vec!["False".into()]),
                ("5".into(), vec!["staging".into()]),
            ]);
            ensure!(
                request.answer(answers.clone()).await.is_err(),
                "accepted an out-of-range integer"
            );
            ensure!(pending.wait_user_input().await?.id() == request.id());
            answers.insert("2".into(), vec!["4".into()]);
            request.answer(answers).await?;
            let run = pending.wait_settled().await?;
            run.assert_succeeded()?;
            run.assert_text_contains("received typed answers")?;

            let pending = user.send("dismiss elicitation").await?;
            pending
                .wait_user_input()
                .await?
                .reply_user_input(UserInputReplyKind::Cancel, BTreeMap::new(), None)
                .await?;
            let run = pending.wait_settled().await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("form declined")?;

            let pending = user.send("withdraw elicitation").await?;
            let withdrawn = pending.wait_user_input().await?;
            tokio::fs::write(case.workspace().join(".acp-withdraw-form"), "").await?;
            pending
                .wait_text_contains("form withdrawn and permission still waiting")
                .await?;
            user.wait_state("only the independent permission stays pending", |state| {
                let requests = state.session.state.pending_interactive_requests();
                requests.len() == 1 && requests[0].request.request_id() != withdrawn.id()
            })
            .await?;
            ensure!(
                withdrawn.answer(BTreeMap::new()).await.is_err(),
                "accepted a reply to a withdrawn request"
            );
            pending
                .wait_user_input()
                .await?
                .answer(BTreeMap::from([("0".into(), vec!["Deny".into()])]))
                .await?;
            let run = pending.wait_settled().await?;
            run.assert_succeeded()?;
            run.assert_text_contains("independent permission resolved")?;

            let pending = user.send("withdraw permission").await?;
            pending.wait_user_input().await?;
            tokio::fs::write(case.workspace().join(".acp-withdraw-permission"), "").await?;
            let run = pending.wait_settled().await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("permission withdrawn")?;

            let pending = user.send("cancel form turn").await?;
            user.wait_state("both interactions are pending", |state| {
                state.session.state.pending_interactive_requests().len() == 2
            })
            .await?;
            pending.cancel().await?;
            pending.wait_settled().await?.assert_cancelled()?;
            ensure!(case.workspace().join(".acp-cancelled-pending").is_file());
            ensure!(
                user.snapshot()
                    .await?
                    .session
                    .state
                    .pending_interactive_requests()
                    .is_empty()
            );
            ensure!(case.model().requests().is_empty());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_plans_and_context_follow_the_native_conversation() -> TestResult {
    let (_bin, builder) = peers()?;
    builder
        .expect_plugin("agena.plan")
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let id = ordinary
                .client()
                .create_session_configured(CreateSessionParams {
                    external_agent: Some(ExternalAgent::ClaudeCode),
                    workspace_id: ordinary
                        .snapshot()
                        .await?
                        .session
                        .workspace_id
                        .context("workspace")?,
                    title: "ACP plans and context".into(),
                    parent_id: None,
                    auto_title: false,
                    profile: None,
                    prompts: None,
                })
                .await?
                .id;
            let user = case.user().attach_session(id).await?;
            let initial = user.snapshot().await?.usage;
            ensure!(
                initial.model_context_window_tokens.is_none() && initial.limit_tokens.is_none()
            );
            user.client().session_plan(id, false).await?;

            let pending = user.send("show project plan").await?;
            pending
                .wait_text_contains("plan visible before prompt completion")
                .await?;
            let usage = user.snapshot().await?.usage;
            ensure!(usage.current_tokens == 1200 && usage.limit_tokens == Some(4000));
            ensure!(usage.limit_basis == Some(SessionUsageLimitBasis::ContextWindow));
            ensure!(usage.input_limit_tokens.is_none());
            let plan = user.client().session_plan(id, false).await?;
            ensure!(
                plan.output_text.contains("Legacy step")
                    && plan.output_text.contains("An older approach.")
            );
            ensure!(plan.output_text.contains("- [x] Inspect workspace"));
            ensure!(
                plan.payload.as_ref().context("plan payload")?["current_step"]["title"]
                    == "Refresh notes"
            );

            let mut changes = user
                .client()
                .stream_changes(Scope::Session { session_id: id })
                .await?;
            tokio::fs::write(case.workspace().join(".acp-plan-advance"), "").await?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let event = changes.recv().await.context("session changes ended")??;
                    if let SubscriptionEvent::RuntimeSignal(signal) = event
                        && signal.kind == "external_agent_state_changed"
                        && signal.session_id == Some(id)
                    {
                        return Ok::<(), anyhow::Error>(());
                    }
                }
            })
            .await
            .context("live ACP state notification")??;
            drop(changes);
            tokio::fs::write(case.workspace().join(".acp-plan-finish"), "").await?;
            let run = pending.wait_settled().await?;
            run.assert_succeeded()?;
            run.assert_text_contains("project plan revised")?;
            let expected = user.client().session_plan(id, false).await?;
            ensure!(expected.output_text.contains("A revised approach."));
            ensure!(expected.output_text.contains("- [x] Review result"));
            for removed in ["Legacy step", "Refresh notes", "An older approach."] {
                ensure!(!expected.output_text.contains(removed));
            }
            ensure!(
                expected.payload.as_ref().context("plan payload")?["plan"]["phase"] == "completed"
            );
            let usage = user.snapshot().await?.usage;
            ensure!(usage.current_tokens == 400 && usage.model_context_window_tokens == Some(2000));
            let cost = user.client().session_cost_summary(id).await?;
            ensure!(cost.total_tokens == 0 && cost.recorded_cost_usd == 0.0);

            let user = case.user().attach_session(id).await?;
            ensure!(
                user.client().session_plan(id, false).await?.output_text == expected.output_text
            );
            ensure!(user.snapshot().await?.usage.current_tokens == 400);
            let loaded = user.send("check load").await?.wait_settled().await?;
            loaded.assert_succeeded()?;
            loaded.assert_assistant_text("loaded")?;
            let restored = user.client().session_plan(id, false).await?;
            ensure!(
                restored
                    .output_text
                    .contains("The plan restored by Claude.")
            );
            ensure!(restored.output_text.contains("- [x] Review result"));
            let usage = user.snapshot().await?.usage;
            ensure!(usage.current_tokens == 550 && usage.limit_tokens == Some(2400));

            user.client().clear_session(id, true).await?;
            let cleared = user.client().session_plan(id, false).await?;
            ensure!(!cleared.output_text.contains("The plan restored by Claude."));
            ensure!(user.snapshot().await?.usage.limit_tokens.is_none());

            let pending = user.send("cancel plan progress").await?;
            pending
                .wait_text_contains("waiting with a native plan")
                .await?;
            ensure!(
                user.client()
                    .session_plan(id, false)
                    .await?
                    .output_text
                    .contains("Stale if aborted")
            );
            pending.cancel().await?;
            pending.wait_settled().await?.assert_cancelled()?;
            ensure!(
                !user
                    .client()
                    .session_plan(id, false)
                    .await?
                    .output_text
                    .contains("Stale if aborted")
            );
            ensure!(user.snapshot().await?.usage.limit_tokens.is_none());
            ensure!(case.model().requests().is_empty());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_commands_follow_their_native_conversation() -> TestResult {
    let (_bin, builder) = peers()?;
    builder
        .seed_file("workspace/.acp-catalog", Vec::new())
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace_id = ordinary
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("workspace id")?;
            let create = |agent: ExternalAgent| {
                let title = agent.label().into();
                ordinary
                    .client()
                    .create_session_configured(CreateSessionParams {
                        external_agent: Some(agent),
                        workspace_id,
                        title,
                        parent_id: None,
                        auto_title: false,
                        profile: None,
                        prompts: None,
                    })
            };
            let claude_id = create(ExternalAgent::ClaudeCode).await?.id;
            let antigravity_id = create(ExternalAgent::Antigravity).await?.id;
            let claude = case.user().attach_session(claude_id).await?;
            let antigravity = case.user().attach_session(antigravity_id).await?;
            let preview = antigravity
                .client()
                .preview_external_agent_options(antigravity_id)
                .await?;
            ensure!(preview.options.is_empty() && preview.preview_commands.is_empty());
            ensure!(!case.workspace().join(".acp-options-opened").exists());
            ensure!(
                claude
                    .snapshot()
                    .await?
                    .execution
                    .external_agent_commands
                    .is_empty()
            );

            let pending = claude.send("/agent:compact").await?;
            pending
                .wait_text_contains("native compact received")
                .await?;
            let commands = claude.snapshot().await?.execution.external_agent_commands;
            ensure!(
                commands
                    .iter()
                    .map(|command| (command.name.as_str(), command.argument_hint.as_deref()))
                    .eq([("compact", None), ("mcp:repo:review", Some("<scope>"))]),
                "unexpected live catalog: {commands:?}"
            );
            ensure!(commands[0].description == "Compact the native context");
            ensure!(
                antigravity
                    .snapshot()
                    .await?
                    .execution
                    .external_agent_commands
                    .is_empty()
            );
            ensure!(
                ordinary
                    .snapshot()
                    .await?
                    .execution
                    .external_agent_commands
                    .is_empty()
            );
            tokio::fs::write(case.workspace().join(".acp-catalog-visible"), "").await?;
            let run = pending.wait_settled().await?;
            run.assert_succeeded()?;
            let parts = claude.parts().await?;
            let input = parts
                .iter()
                .find(|part| part.role == "user" && part.kind == "text")
                .context("saved user input")?;
            let mut text = Vec::new();
            claude
                .client()
                .write_part_text(claude_id, input, &mut text)
                .await?;
            ensure!(text == b"/agent:compact");

            let run = antigravity
                .send("/agent:status")
                .await?
                .wait_settled()
                .await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("native status received")?;
            let other_commands = antigravity
                .snapshot()
                .await?
                .execution
                .external_agent_commands;
            ensure!(
                other_commands
                    .iter()
                    .map(|command| command.name.as_str())
                    .eq(["status", "compact"])
            );
            let run = antigravity
                .send("/agent:compact")
                .await?
                .wait_settled()
                .await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("antigravity native compact received")?;
            ensure!(
                antigravity
                    .snapshot()
                    .await?
                    .execution
                    .external_agent_details
                    .compactions
                    .iter()
                    .any(|item| item.id == "acp-compact-1" && item.status == "completed")
            );
            read_policy(
                &antigravity,
                case.workspace().join("acp-input.txt"),
                "readonly",
            )
            .await?;
            let followup = antigravity
                .send("/agent:compact inspect after compacting")
                .await?
                .wait_settled()
                .await?;
            followup.assert_succeeded()?;
            followup.assert_assistant_text("read through Agena during compact")?;
            ensure!(claude.snapshot().await?.execution.external_agent_commands == commands);

            let claude = case.user().attach_session(claude_id).await?;
            ensure!(claude.snapshot().await?.execution.external_agent_commands == commands);
            let run = claude
                .send(r#"/agent:mcp:repo:review  "scope"  all"#)
                .await?
                .wait_settled()
                .await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("native review received exact arguments")?;
            let replacement = claude.snapshot().await?.execution.external_agent_commands;
            ensure!(replacement.len() == 1 && replacement[0].name == "review");

            let run = claude.send("//agent:compact").await?.wait_settled().await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("escaped command received as text")?;
            ensure!(
                claude
                    .snapshot()
                    .await?
                    .execution
                    .external_agent_commands
                    .is_empty()
            );
            let antigravity = case.user().attach_session(antigravity_id).await?;
            ensure!(
                antigravity
                    .snapshot()
                    .await?
                    .execution
                    .external_agent_commands
                    == other_commands
            );
            antigravity
                .client()
                .clear_session(antigravity_id, true)
                .await?;
            ensure!(
                antigravity
                    .snapshot()
                    .await?
                    .execution
                    .external_agent_commands
                    .is_empty()
            );
            ensure!(case.model().requests().is_empty());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_options_validate_model_and_thinking_before_the_first_prompt() -> TestResult
{
    let (_bin, builder) = peers()?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace_id = ordinary
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("workspace id")?;
            ensure!(
                ordinary
                    .client()
                    .external_agent_options(ordinary.id(), None)
                    .await
                    .is_err()
            );
            let created = ordinary
                .client()
                .create_session_configured(CreateSessionParams {
                    external_agent: Some(ExternalAgent::Antigravity),
                    workspace_id,
                    title: "Agent options".into(),
                    parent_id: None,
                    auto_title: false,
                    profile: None,
                    prompts: None,
                })
                .await?;
            let sid = created.id.to_string();
            let user = case.user().attach_session(created.id).await?;
            let change = |kind, value: &str| {
                Some(ExternalAgentOptionChange {
                    kind,
                    value: Some(value.into()),
                })
            };
            let model = ExternalAgentOptionKind::Model;
            let thought = ExternalAgentOptionKind::ThoughtLevel;

            let options = user
                .client()
                .external_agent_options(created.id, None)
                .await?;
            ensure!(options.selection.is_empty() && options.options.len() == 3);
            ensure!(options.options[0].kind == model && options.options[0].choices.len() == 2);
            ensure!(
                options.options[1]
                    .choices
                    .iter()
                    .all(|choice| choice.value != "high")
            );
            ensure!(
                user.client()
                    .external_agent_options(created.id, change(thought.clone(), "high"))
                    .await
                    .is_err()
            );

            let cli = case
                .run_cli(["sessions", "agent-options", &sid, "--model", "deep"])
                .await?;
            let after: agena_domain::ExternalAgentOptions =
                serde_json::from_str(&cli.require_success()?.stdout)?;
            ensure!(after.selection.get(&model).map(String::as_str) == Some("deep"));
            ensure!(
                after.options[1]
                    .choices
                    .iter()
                    .any(|choice| choice.value == "high")
            );
            let options = user
                .client()
                .external_agent_options(created.id, change(thought.clone(), "high"))
                .await?;
            ensure!(options.selection.get(&thought).map(String::as_str) == Some("high"));
            ensure!(user.parts().await?.is_empty());
            let created_sessions =
                tokio::fs::read_to_string(case.workspace().join(".acp-new-sessions")).await?;
            ensure!(
                created_sessions.lines().count() == 1,
                "expected a single bound native session across option checks: {created_sessions}"
            );

            let user = case.user().attach_session(created.id).await?;
            let run = user.send("check options").await?.wait_settled().await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("model deep, thinking high")?;
            let options = user
                .client()
                .external_agent_options(created.id, change(model.clone(), "fast"))
                .await?;
            ensure!(options.selection.get(&model).map(String::as_str) == Some("fast"));
            ensure!(!options.selection.contains_key(&thought));
            let run = user.send("check options").await?.wait_settled().await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("model fast, thinking low")?;
            let created_sessions =
                tokio::fs::read_to_string(case.workspace().join(".acp-new-sessions")).await?;
            ensure!(
                created_sessions.lines().count() == 1,
                "expected prompts and later option updates to reuse the same native session: {created_sessions}"
            );
            ensure!(case.model().requests().is_empty());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn antigravity_fences_cancelled_tools_and_keeps_native_history() -> TestResult {
    let (_bin, builder) = peers()?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let resource = ordinary.snapshot().await?.session;
            ensure!(resource.external_agent.is_none());
            case.model()
                .expect(RequestMatch::user_text("ordinary model route"))
                .reply(Reply::text("ordinary model route reply"));
            let ordinary_run = ordinary
                .send("ordinary model route")
                .await?
                .wait_settled()
                .await?;
            ordinary_run.assert_succeeded()?;
            ordinary_run.assert_assistant_text("ordinary model route reply")?;
            let created = ordinary
                .client()
                .create_session_configured(CreateSessionParams {
                    external_agent: Some(ExternalAgent::Antigravity),
                    workspace_id: resource.workspace_id.context("workspace id")?,
                    title: "Antigravity ACP".into(),
                    parent_id: None,
                    auto_title: true,
                    profile: None,
                    prompts: None,
                })
                .await?;
            ensure!(created.external_agent == Some(ExternalAgent::Antigravity));
            let user = case.user().attach_session(created.id).await?;
            let path = case.workspace().join("acp-input.txt");
            read_policy(&user, path.clone(), "readonly").await?;
            let run = user.send("read fixture").await?.wait_settled().await?;
            run.assert_succeeded()?;
            run.assert_text_contains("MCP returned the file contents")?;
            ensure!(
                run.parts
                    .iter()
                    .any(|part| part.kind == "tool_call" && part.state == "completed")
            );

            let user = case.user().attach_session(created.id).await?;
            let follow_up = user.send("check resume").await?.wait_settled().await?;
            follow_up.assert_succeeded()?;
            follow_up.assert_assistant_text("resumed")?;

            read_policy(&user, path, "deny").await?;
            let blocked = user.send("deny fixture").await?.wait_settled().await?;
            blocked.assert_succeeded()?;
            blocked.assert_assistant_text("MCP blocked by Agena policy")?;

            let running = user.send("cancel this").await?;
            running
                .wait_text_contains("waiting for cancellation")
                .await?;
            running.cancel().await?;
            let cancelled = running.wait_settled().await?;
            cancelled.assert_cancelled()?;
            ensure!(cancelled.parts.iter().any(|part| part.kind == "tool_call"
                && part.content["name"] == "Antigravity · Wait natively"
                && part.state == "failed"));
            ensure!(
                case.workspace().join(".acp-cancelled").is_file(),
                "ACP did not receive cancellation"
            );
            let url = tokio::fs::read_to_string(case.workspace().join(".acp-cancel-url")).await?;
            let endpoint = url
                .strip_prefix("http://")
                .and_then(|url| url.strip_suffix("/mcp"))
                .context("fixture MCP endpoint")?;
            let closed = tokio::time::timeout(
                Duration::from_secs(1),
                tokio::net::TcpStream::connect(endpoint),
            )
            .await;
            ensure!(
                !matches!(closed, Ok(Ok(_))),
                "MCP listener outlived its turn"
            );

            let user = case.user().attach_session(created.id).await?;
            let recovered = user
                .send("continue after cancellation")
                .await?
                .wait_settled()
                .await?;
            recovered.assert_succeeded()?;
            recovered.assert_text_contains("continued after cancellation")?;
            ensure!(!recovered.parts.iter().any(|part| part.kind == "notice"
                && part.content["kind"] == "external_agent_context_reset"));

            let next = user.send("check resume").await?.wait_settled().await?;
            next.assert_succeeded()?;
            next.assert_assistant_text("resumed")?;
            user.client().clear_session(user.id(), true).await?;
            let cleared = user.send("check clear").await?.wait_settled().await?;
            cleared.assert_succeeded()?;
            cleared.assert_assistant_text("cleared")?;
            ensure!(
                case.model().requests().len() == 1,
                "external session used an Agena model provider"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_code_loads_sessions_and_projects_native_tools_and_composer_input() -> TestResult {
    let (_bin, builder) = peers()?;
    builder
        .run(|case| async move {
            let workspace = case
                .workspace()
                .to_str()
                .context("non-UTF-8 fixture location")?;
            let created = case
                .run_cli([
                    "sessions",
                    "create",
                    "--workspace",
                    workspace,
                    "--external-agent",
                    "claude-code",
                ])
                .await?;
            let state: Value = serde_json::from_str(&created.require_success()?.stdout)?;
            ensure!(
                state["session"]["external_agent"] == "claude-code",
                "{state}"
            );
            let id = state["session"]["id"]
                .as_i64()
                .context("created session id")?;
            let user = case.user().attach_session(id).await?;
            let options = user.client().external_agent_options(id, None).await?;
            ensure!(options.options.len() == 3 && options.selection.is_empty());
            ensure!(case.workspace().join(".acp-options-inspected").exists());
            let pending = user.send("check permission").await?;
            let request = pending.wait_user_input().await?;
            pending
                .wait_text_contains("claude-agent-acp started")
                .await?;
            let native_tool = |part: &&agena_api::part::PartResource| {
                part.kind == "tool_call"
                    && part.content["name"] == "Claude Code · Read fixture natively"
            };
            ensure!(
                user.parts()
                    .await?
                    .iter()
                    .filter(native_tool)
                    .any(|part| part.state == "running")
            );
            request
                .answer(BTreeMap::from([("0".into(), vec!["Allow".into()])]))
                .await?;
            let run = pending.wait_settled().await?;
            run.assert_succeeded()?;
            run.assert_text_contains("native permission accepted")?;
            let mut native = run.parts.iter().filter(native_tool);
            let part = native.next().context("native tool in the transcript")?;
            ensure!(native.next().is_none() && part.state == "completed");
            let output = user
                .client()
                .session_tool_detail(user.id(), part.part_id, ToolDetailSection::Output)
                .await?;
            ensure!(
                output.value["payload"]["text"] == "native stdout",
                "{}",
                output.value
            );

            let loaded = user.send("check load").await?.wait_settled().await?;
            loaded.assert_succeeded()?;
            loaded.assert_assistant_text("loaded")?;
            ensure!(loaded.parts.iter().any(|part| part.kind == "notice"
                && part.content["kind"] == "external_agent_system_prompt_unavailable"));
            ensure!(!loaded.parts.iter().any(|part| part.kind == "notice"
                && part.content["kind"] == "external_agent_context_reset"));
            read_policy(&user, case.workspace().join("acp-input.txt"), "readonly").await?;
            let mcp = user.send("read fixture").await?.wait_settled().await?;
            mcp.assert_succeeded()?;
            mcp.assert_assistant_text("MCP returned the file contents")?;
            ensure!(
                mcp.parts
                    .iter()
                    .filter(|part| part.kind == "tool_call")
                    .count()
                    == 1
            );

            read_policy(&user, case.workspace().join("acp-image.png"), "readonly").await?;
            let rich = user
                .send_document(composer()?, Default::default())
                .await?
                .wait_settled()
                .await?;
            rich.assert_succeeded()?;
            rich.assert_assistant_text("image and references received")?;

            let image = user.send("check tool image").await?.wait_settled().await?;
            image.assert_succeeded()?;
            image.assert_assistant_text("MCP returned the image")?;
            read_policy(&user, case.workspace().join("acp-image.png"), "deny").await?;
            let denied = user.send("deny tool image").await?.wait_settled().await?;
            denied.assert_succeeded()?;
            denied.assert_assistant_text("MCP blocked the image")?;
            read_policy(&user, case.workspace().join("acp-image.png"), "readonly").await?;

            tokio::fs::write(case.workspace().join(".acp-no-image"), "").await?;
            let unsupported = user
                .send_document(composer()?, Default::default())
                .await?
                .wait_settled()
                .await?;
            unsupported.assert_failed()?;
            ensure!(user.parts().await?.iter().any(|part| {
                part.kind == "error"
                    && part.message_id == Some(unsupported.status.message_id)
                    && part.content["message"].as_str().is_some_and(|message| {
                        message.contains("does not support image input over ACP")
                    })
            }));
            ensure!(case.model().requests().is_empty());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acp_rest_parity_mode_cli_and_long_streamed_reply_succeed() -> TestResult {
    let (_bin, builder) = peers()?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace_id = ordinary
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("workspace id")?;
            let created = ordinary
                .client()
                .create_session_configured(CreateSessionParams {
                    external_agent: Some(ExternalAgent::ClaudeCode),
                    workspace_id,
                    title: "Initial ACP View".into(),
                    parent_id: None,
                    auto_title: false,
                    profile: None,
                    prompts: None,
                })
                .await?;
            let acp_id = created.id;
            let server = case.http().await;

            // GET /api/v1/acp/sessions lists ephemeral ACP views for the workspace.
            let list_resp = server
                .get(&format!("/api/v1/acp/sessions?workspace_id={workspace_id}"))
                .await?;
            ensure!(
                list_resp.status.is_success(),
                "list status {}",
                list_resp.status
            );
            let listed: Value = list_resp.json()?;
            ensure!(
                listed["items"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item["id"] == acp_id)),
                "{listed}"
            );

            // PUT /api/v1/acp/sessions/{id} updates the view instead of returning 405.
            let put_resp = server
                .send_json(
                    reqwest::Method::PUT,
                    &format!("/api/v1/acp/sessions/{acp_id}"),
                    &json!({"title": "Renamed ACP View"}),
                )
                .await?;
            ensure!(
                put_resp.status.is_success(),
                "put status {}",
                put_resp.status
            );
            let updated: Value = put_resp.json()?;
            ensure!(updated["title"] == "Renamed ACP View", "{updated}");

            // DefaultBodyLimit rejects oversized /agent-work payloads with 413.
            let oversized = "x".repeat(2048);
            let limit_resp = server
                .send_json(
                    reqwest::Method::POST,
                    &format!("/api/v1/acp/sessions/{acp_id}/agent-work"),
                    &json!({"cancel_id": oversized}),
                )
                .await?;
            ensure!(
                limit_resp.status == reqwest::StatusCode::PAYLOAD_TOO_LARGE,
                "expected 413, got {}",
                limit_resp.status
            );

            // CLI --mode flag selects the advertised mode option.
            let mode_cli = case
                .run_cli([
                    "sessions",
                    "agent-options",
                    &acp_id.to_string(),
                    "--mode",
                    "default",
                ])
                .await?;
            let mode_json: Value = serde_json::from_str(&mode_cli.require_success()?.stdout)?;
            ensure!(mode_json["selection"]["mode"] == "default", "{mode_json}");

            // Streamed reply exceeding 64 KiB (80+ KiB) is preserved intact without truncation.
            let user = case.user().attach_session(acp_id).await?;
            let run = user.send("long reply").await?.wait_settled().await?;
            run.assert_succeeded()?;
            let reply_part = run
                .parts
                .iter()
                .rev()
                .find(|part| part.role == "assistant" && part.kind == "text")
                .context("long assistant text part")?;
            let mut text = Vec::new();
            user.client()
                .write_part_text(acp_id, reply_part, &mut text)
                .await?;
            let text = String::from_utf8(text)?;
            ensure!(
                text.len() > 80 * 1024 && text.contains("[00]") && text.contains("[39]"),
                "expected >80 KiB reply with all 40 chunks, got {} bytes",
                text.len()
            );
            ensure!(
                !text.contains("[ACP message truncated.]"),
                "long reply should not be truncated under the 512 KiB document cap"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_generic_acp_agent_catalog_and_execution() -> TestResult {
    let host = tempfile::tempdir()?;
    let custom_bin = host.path().join("my-custom-acp");
    std::fs::write(&custom_bin, include_str!("plugins/fake_acp_custom.py"))?;
    std::fs::set_permissions(&custom_bin, std::fs::Permissions::from_mode(0o755))?;
    let state_dir = host.path().join("custom-state");
    std::fs::create_dir_all(&state_dir)?;
    let routes = host.path().join("providers.json");
    std::fs::write(
        &routes,
        serde_json::to_vec(&json!({
            "codex-custom": [
                {
                    "action": "set",
                    "providerId": "main",
                    "apiType": "openai",
                    "baseUrl": "https://custom-acp.example.invalid/v1"
                }
            ]
        }))?,
    )?;
    let custom_bin_str = custom_bin.to_string_lossy().into_owned();
    let state_dir_str = state_dir.to_string_lossy().into_owned();
    let expected_bin = custom_bin_str.clone();
    fixture()
        .configure_global(move |config| {
            config["sandbox"] = json!(false);
            config["acp"] = json!({
                "agents": {
                    "codex-custom": {
                        "name": "Codex Custom",
                        "command": custom_bin_str,
                        "args": ["--acp-stdio", "--profile", "smoke"],
                        "env": {
                            "CUSTOM_ACP_TOKEN": "custom-secret-123"
                        },
                        "client_tools": false,
                        "writable_paths": [state_dir_str]
                    },
                    "antigravity": {
                        "disabled": true
                    }
                }
            });
        })
        .env("AGENA_ACP_PROVIDERS_FILE", routes)
        .env("AGENA_TEST_NEVER_FORWARD", "must-not-leak")
        .expect_plugin("agena.fs")
        .seed_file("workspace/acp-input.txt", b"acp callback content\n".to_vec())
        .run(move |case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace_id = ordinary
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("workspace id")?;
            let server = case.http().await;

            // GET /api/v1/acp/agents lists enabled built-ins + custom agents and omits env/writable_paths.
            let catalog_resp = server.get("/api/v1/acp/agents").await?;
            ensure!(
                catalog_resp.status.is_success(),
                "catalog status {}",
                catalog_resp.status
            );
            let catalog_raw = String::from_utf8_lossy(&catalog_resp.body).into_owned();
            ensure!(!catalog_raw.contains("CUSTOM_ACP_TOKEN"), "{catalog_raw}");
            ensure!(!catalog_raw.contains("custom-secret-123"), "{catalog_raw}");
            ensure!(!catalog_raw.contains("custom-state"), "{catalog_raw}");
            let catalog: Vec<agena_domain::ExternalAgentCatalogEntry> = catalog_resp.json()?;
            let ids: Vec<&str> = catalog.iter().map(|entry| entry.id.id()).collect();
            ensure!(ids == vec!["claude-code", "codex-custom"], "{ids:?}");
            let custom = &catalog[1];
            ensure!(
                custom.name == "Codex Custom"
                    && !custom.builtin
                    && custom.command == expected_bin
                    && custom.args == vec!["--acp-stdio", "--profile", "smoke"]
                    && custom.available == Some(true),
                "{custom:?}"
            );

            // CLI `agena sessions agent-sessions agents` returns the same dynamic catalog.
            let cli_agents = case
                .run_cli(["sessions", "agent-sessions", "agents"])
                .await?;
            let cli_catalog: Vec<agena_domain::ExternalAgentCatalogEntry> =
                serde_json::from_str(&cli_agents.require_success()?.stdout)?;
            ensure!(cli_catalog == catalog, "{cli_catalog:?} vs {catalog:?}");

            // Disabled built-in and unconfigured custom IDs are rejected on session creation.
            for rejected in ["antigravity", "unknown-acp"] {
                let resp = server
                    .send_json(
                        reqwest::Method::POST,
                        "/api/v1/acp/sessions",
                        &json!({
                            "workspace_id": workspace_id,
                            "title": "Rejected",
                            "external_agent": rejected
                        }),
                    )
                    .await?;
                ensure!(
                    !resp.status.is_success(),
                    "expected {rejected} to be rejected, got {}",
                    resp.status
                );
            }

            // Create and execute a custom ACP session via CLI + REST.
            let workspace = case.workspace().to_str().context("workspace str")?;
            let created = case
                .run_cli([
                    "sessions",
                    "create",
                    "--workspace",
                    workspace,
                    "--external-agent",
                    "codex-custom",
                ])
                .await?;
            let state: Value = serde_json::from_str(&created.require_success()?.stdout)?;
            ensure!(
                state["session"]["external_agent"] == "codex-custom",
                "{state}"
            );
            let session_id = state["session"]["id"].as_i64().context("session id")?;
            let user = case.user().attach_session(session_id).await?;

            // AGENA_ACP_PROVIDERS_FILE keyed by custom agent ID applies before opening sessions.
            let providers = user.client().external_agent_providers(session_id).await?;
            ensure!(providers.supported && providers.providers.len() == 1);
            let current = providers.providers[0]
                .current
                .as_ref()
                .context("provider route")?;
            ensure!(
                current.api_type == "openai"
                    && current.base_url == "https://custom-acp.example.invalid/v1"
            );

            // Auth inspection reports the custom agent's configured command and base_args.
            let auth = user
                .client()
                .external_agent_auth(
                    session_id,
                    agena_domain::ExternalAgentAuthAction::Inspect { terminal: true },
                )
                .await?;
            ensure!(auth.agent == ExternalAgent::Custom("codex-custom".into()));
            ensure!(auth.command.as_deref() == Some(expected_bin.as_str()));
            ensure!(auth.base_args == vec!["--acp-stdio", "--profile", "smoke"]);

            // Prompt runs pure standard ACP without vendor _meta and accesses Agena MCP tools.
            read_policy(&user, case.workspace().join("acp-input.txt"), "readonly").await?;
            let run = user
                .send("check custom acp")
                .await?
                .wait_settled()
                .await?;
            run.assert_succeeded()?;
            run.assert_text_contains("custom acp ok: acp callback content")?;
            ensure!(case.model().requests().is_empty());
            Ok(())
        })
        .await
}
