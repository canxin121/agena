use std::{os::unix::fs::PermissionsExt as _, time::Duration};

use agena_api::commands::CreateSessionParams;
use agena_domain::ExternalAgent;
use agena_testkit::TestResult;
use anyhow::{Context as _, ensure};
use serde_json::json;

use super::fixture;

async fn exercise(agent: ExternalAgent, version: &str, transport: &'static str) -> TestResult {
    let bin = tempfile::tempdir()?;
    let executable = bin.path().join(agent.command().0);
    std::fs::write(&executable, include_str!("plugins/fake_acp_mcp.py"))?;
    std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o755))?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin.path().to_owned()).chain(std::env::split_paths(&path)),
    )?;
    fixture()
        .env("PATH", path)
        .env(
            "AGENA_ACP_MCP_TRANSPORT",
            if transport == "none" { "sse" } else { "http" },
        )
        .configure_global(|config| config["sandbox"] = json!(false))
        .seed_file("workspace/.acp-mcp-version", version.as_bytes().to_vec())
        .seed_file("workspace/.acp-transport", transport.as_bytes().to_vec())
        .seed_file("workspace/shared.txt", b"expected MCP payload\n".to_vec())
        .seed_file("workspace/private.txt", b"withheld MCP payload\n".to_vec())
        .expect_plugin("agena.fs")
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace_id = ordinary
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("workspace ID")?;
            let session = ordinary
                .client()
                .create_session_configured(CreateSessionParams {
                    workspace_id,
                    title: "MCP transport".into(),
                    external_agent: Some(agent),
                    auto_title: false,
                    parent_id: None,
                    profile: None,
                    prompts: None,
                })
                .await?;
            let user = case.user().attach_session(session.id).await?;
            let private = case.workspace().join("private.txt");
            user.client()
                .set_session_permission(
                    session.id,
                    serde_json::from_value(json!({
                        "tools": {"default": "allow"},
                        "path": {"rules": {(private.to_string_lossy()): "deny"}}
                    }))?,
                )
                .await?;
            let run = user
                .send("exercise MCP transport")
                .await?
                .wait_settled()
                .await?;
            run.assert_succeeded()?;
            run.assert_assistant_text("transport verified")?;
            ensure!(
                run.parts
                    .iter()
                    .any(|part| part.kind == "tool_call" && part.content["name"] == "agena.fs.read")
                    == (transport != "none")
            );
            if transport != "none" {
                let marker = case.workspace().join(".acp-grant-closed");
                tokio::time::timeout(Duration::from_secs(5), async {
                    while !marker.is_file() {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .context("ACP MCP endpoint was not revoked")?;
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_stdio_connects_without_a_transport_capability() -> TestResult {
    exercise(ExternalAgent::Antigravity, "1", "stdio").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_legacy_sse_authenticates_both_channels() -> TestResult {
    exercise(ExternalAgent::ClaudeCode, "1", "sse").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_stdio_uses_the_tagged_descriptor() -> TestResult {
    exercise(ExternalAgent::ClaudeCode, "2", "stdio").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_can_run_without_any_mcp_transport() -> TestResult {
    exercise(ExternalAgent::ClaudeCode, "2", "none").await
}
