use std::{collections::BTreeMap, os::unix::fs::PermissionsExt as _, path::Path, time::Duration};

use agena_api::commands::CreateSessionParams;
use agena_domain::ExternalAgent;
use agena_testkit::{TestAgenaBuilder, TestResult, UserSession};
use anyhow::{Context as _, Result, ensure};

fn configured_peer(version: &str) -> Result<(tempfile::TempDir, TestAgenaBuilder)> {
    let bin = tempfile::tempdir()?;
    let peer = bin.path().join("claude-agent-acp");
    std::fs::write(&peer, include_str!("plugins/fake_acp_cancellation.py"))?;
    std::fs::set_permissions(&peer, std::fs::Permissions::from_mode(0o755))?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin.path().to_owned()).chain(std::env::split_paths(&path)),
    )?;
    Ok((
        bin,
        super::fixture().env("PATH", path).seed_file(
            "workspace/.acp-cancellation-version",
            version.as_bytes().to_vec(),
        ),
    ))
}

async fn create_agent_session(ordinary: &UserSession) -> Result<i64> {
    let workspace_id = ordinary
        .snapshot()
        .await?
        .session
        .workspace_id
        .context("workspace")?;
    Ok(ordinary
        .client()
        .create_session_configured(CreateSessionParams {
            workspace_id,
            external_agent: Some(ExternalAgent::ClaudeCode),
            title: "ACP cancellation".into(),
            parent_id: None,
            auto_title: false,
            profile: None,
            prompts: None,
        })
        .await?
        .id)
}

async fn wait_for_marker(workspace: &Path, marker: &str) -> Result<()> {
    let path = workspace.join(marker);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.is_file() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .with_context(|| format!("ACP peer did not write {marker}"))?;
    Ok(())
}

async fn abort_setup(version: &'static str) -> TestResult {
    let (_bin, builder) = configured_peer(version)?;
    builder
        .run(|case| async move {
            tokio::fs::write(case.workspace().join(".acp-hold-setup"), "").await?;
            let ordinary = case.user().new_session().await?;
            let session = create_agent_session(&ordinary).await?;
            let user = case.user().attach_session(session).await?;
            let execution = user.send("stop while opening").await?;
            wait_for_marker(case.workspace(), ".acp-setup-started").await?;
            execution.cancel().await?;
            execution.wait_settled().await?.assert_cancelled()?;
            wait_for_marker(case.workspace(), ".acp-setup-cancelled").await?;
            ensure!(case.workspace().join(".acp-rejected-v2-init").is_file() == (version == "1"));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_v1_aborts_the_single_pending_setup_rpc() -> TestResult {
    abort_setup("1").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_v2_aborts_the_single_pending_setup_rpc() -> TestResult {
    abort_setup("2").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_v2_timeout_withdraws_only_the_outgoing_rpc() -> TestResult {
    let (_bin, builder) = configured_peer("2")?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let session = create_agent_session(&ordinary).await?;
            let user = case.user().attach_session(session).await?;
            let client = user.client();
            let execution = user.send("timeout one native request").await?;
            let permission = execution.wait_user_input().await?;
            let work = client.external_agent_work(session, None).await?;
            let terminal = work
                .iter()
                .find(|work| work.target_id == "surviving-terminal")
                .context("advertised terminal")?;
            ensure!(
                client
                    .external_agent_work(session, Some(terminal.cancel_id))
                    .await
                    .is_err()
            );
            wait_for_marker(case.workspace(), ".acp-target-timeout").await?;
            permission
                .answer(BTreeMap::from([("0".into(), vec!["Reject once".into()])]))
                .await?;
            let result = execution.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("Independent work continued")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_v2_session_stop_answers_only_the_withdrawn_permission() -> TestResult {
    let (_bin, builder) = configured_peer("2")?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let session = create_agent_session(&ordinary).await?;
            let user = case.user().attach_session(session).await?;
            let execution = user.send("stop foreground").await?;
            user.wait_state("both native permissions are pending", |snapshot| {
                snapshot.session.state.pending_interactive_requests().len() == 2
            })
            .await?;
            execution.cancel().await?;
            execution.wait_settled().await?.assert_cancelled()?;
            wait_for_marker(case.workspace(), ".acp-foreground-stopped").await?;
            Ok(())
        })
        .await
}
