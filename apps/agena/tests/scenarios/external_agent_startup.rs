use std::{os::unix::fs::PermissionsExt as _, path::Path};

use agena_api::{commands::CreateSessionParams, resource::RunOptions};
use agena_domain::{
    ActivityId, ActivityPayload, ComposerActivity, ComposerDocument, ComposerNode, ExternalAgent,
    ExternalAgentOptionChange, ExternalAgentOptionKind as Kind, ExternalAgentSessionAction,
    ExternalAgentSessionResult, SkillReferenceActivity,
};
use agena_testkit::{TestAgenaBuilder, TestResult};
use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};

fn fixture(version: &str) -> Result<(tempfile::TempDir, TestAgenaBuilder)> {
    let bin = tempfile::tempdir()?;
    let agent = bin.path().join("claude-agent-acp");
    std::fs::write(&agent, include_str!("plugins/fake_acp_startup.py"))?;
    std::fs::set_permissions(agent, std::fs::Permissions::from_mode(0o755))?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin.path().to_owned()).chain(std::env::split_paths(&path)),
    )?;
    let builder = super::fixture()
        .env("PATH", path)
        .seed_file("workspace/.acp-startup-version", version.as_bytes().to_vec())
        .configure_global(|config| {
            config["profiles"]["acp-startup"] = json!({"extends": "main", "context": {"preload_skills": ["startup-fixture"]}});
            config["prompts"] = json!({
                "order": ["startup-fixture"], "library": [{"id": "startup-fixture", "body": "ACP_INHERITED_STARTUP_FIXTURE"}],
            });
        })
        .seed_file("workspace/AGENTS.md", b"ACP_PROJECT_GUIDANCE_FIXTURE\n".to_vec())
        .seed_file("workspace/shared/scope.txt", Vec::new())
        .seed_file("workspace/.agena/skills/startup-fixture/SKILL.md",
            b"---\nname: startup-fixture\ndescription: ACP startup fixture\n---\n\nACP_PROFILE_SKILL_FIXTURE\n".to_vec());
    Ok((bin, builder))
}

fn text(text: &str) -> ComposerNode {
    ComposerNode::Text {
        text: text.to_owned(),
    }
}

fn skill(name: &str) -> ComposerNode {
    ComposerNode::activity(ComposerActivity {
        id: ActivityId::new(),
        payload: ActivityPayload::SkillReference(SkillReferenceActivity::external(
            ExternalAgent::ClaudeCode,
            name.to_owned(),
            String::new(),
        )),
        provenance: Default::default(),
    })
}

async fn records(path: &Path, filename: &str) -> Result<Vec<Value>> {
    let lines = tokio::fs::read_to_string(path.join(filename)).await?;
    Ok(lines
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?)
}

async fn startup(version: &'static str) -> TestResult {
    let (_bin, builder) = fixture(version)?;
    builder.run(|case| async move {
        let ordinary = case.user().new_session().await?;
        let workspace_id = ordinary.snapshot().await?.session.workspace_id.context("workspace")?;
        let client = ordinary.client();
        let created = client.create_session_configured(CreateSessionParams {
            workspace_id,
            external_agent: Some(ExternalAgent::ClaudeCode),
            title: "ACP startup parity".into(),
            profile: Some("acp-startup".into()),
            parent_id: None,
            auto_title: false,
            prompts: None,
        }).await?;
        let session = created.id;
        client.external_agent_session(session, ExternalAgentSessionAction::SetDirectories {
            directories: vec!["shared".into()],
        }).await?;
        let preview = client.session_prompt_preview(session).await?;
        ensure!(preview.text.contains("ACP_INHERITED_STARTUP_FIXTURE"));
        ensure!(preview.text.contains("ACP_PROJECT_GUIDANCE_FIXTURE"));
        ensure!(preview.text.contains("ACP_PROFILE_SKILL_FIXTURE"));
        ensure!(!case.workspace().join(".acp-startup-prompts.jsonl").exists());
        let startup_view = client.get_session_state(session).await?;
        ensure!(startup_view.execution.external_agent_command_preview);
        let defaults = client.preview_external_agent_options(session).await?;
        ensure!(defaults.options.len() == 2);
        if version == "2" {
            ensure!(defaults.preview_commands.iter().map(|command| command.name.as_str()).eq(["preview-only", "summarize"]));
            ensure!(defaults.preview_commands[1].skill_inline);
        } else {
            ensure!(defaults.preview_commands.is_empty());
        }
        let query = records(case.workspace(), ".acp-startup-options.jsonl").await?;
        ensure!(query.len() == 1 && query[0]["method"] == "session/options");
        ensure!(!case.workspace().join(".acp-startup-prompts.jsonl").exists());
        for (kind, value) in [(Kind::Model, "second"), (Kind::Config("fast-native".into()), "true")] {
            client.external_agent_options(session, Some(ExternalAgentOptionChange {
                kind, value: Some(value.to_owned()),
            })).await?;
        }
        let user = case.user().attach_session(session).await?;
        let first = user.send("/agent:status now").await?.wait_settled().await?;
        first.assert_succeeded()?;
        first.assert_assistant_text("Startup configuration applied")?;
        let first_setup = records(case.workspace(), ".acp-startup-prompts.jsonl").await?;
        ensure!(first_setup == [json!({"method": "session/new", "text": preview.text})]);
        let setup_options = records(case.workspace(), ".acp-startup-options.jsonl").await?;
        ensure!(setup_options.last() == Some(&json!({"method":"session/set_config_option", "model":"second", "fast":true})));

        let opened = user.snapshot().await?.execution;
        ensure!(!opened.external_agent_command_preview);
        ensure!(!opened.external_agent_commands.iter().any(|command| command.name == "preview-only"));
        ensure!(client.preview_external_agent_options(session).await?.preview_commands.is_empty());
        let document = if version == "2" {
            let commands = opened.external_agent_commands;
            ensure!(commands.iter().filter(|command| command.skill_inline).count() == 2);
            ComposerDocument(vec![text("before "), skill("summarize"), text(" between "), skill("chart"), text(" after")])
        } else {
            ComposerDocument(vec![text("/agent:status now")])
        };
        let second = user.send_document(document, RunOptions {
            system: Some("ACP_EXPLICIT_STARTUP_FIXTURE".into()),
            ..Default::default()
        }).await?.wait_settled().await?;
        second.assert_succeeded()?;
        second.assert_assistant_text("Startup configuration applied")?;
        let setup = records(case.workspace(), ".acp-startup-prompts.jsonl").await?;
        let expected = if version == "2" { "session/resume" } else { "session/load" };
        ensure!(setup[1]["method"] == expected);
        let appended = setup[1]["text"].as_str().context("run system prompt")?;
        ensure!(appended.contains("ACP_EXPLICIT_STARTUP_FIXTURE"));
        ensure!(!appended.contains("ACP_INHERITED_STARTUP_FIXTURE"));

        let fork = client.external_agent_session(session, ExternalAgentSessionAction::Fork {
            title: None, before_message_id: None,
        }).await?;
        ensure!(matches!(fork, ExternalAgentSessionResult::Forked { .. }));
        let setup = records(case.workspace(), ".acp-startup-prompts.jsonl").await?;
        ensure!(setup[2]["method"] == "session/fork");
        ensure!(setup[2]["text"] == preview.text);
        ensure!(case.model().requests().is_empty());
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_v1_applies_typed_configuration_and_appended_instructions() -> TestResult {
    startup("1").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_v2_applies_startup_context_and_multiple_inline_skills() -> TestResult {
    startup("2").await
}
