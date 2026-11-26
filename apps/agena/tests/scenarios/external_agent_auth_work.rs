use std::{os::unix::fs::PermissionsExt as _, time::Duration};

use agena_api::commands::CreateSessionParams;
use agena_domain::{
    ExternalAgent, ExternalAgentAuthAction, ExternalAgentAuthStatus, ExternalAgentSessionAction,
    ExternalAgentWorkKind,
};
use agena_testkit::{TestAgenaBuilder, TestResult, UserSession};
use anyhow::{Context as _, Result, ensure};

fn configured_agents(version: &str) -> Result<(tempfile::TempDir, TestAgenaBuilder)> {
    let bin = tempfile::tempdir()?;
    for agent in [ExternalAgent::ClaudeCode, ExternalAgent::Antigravity] {
        let executable = bin.path().join(agent.command().0);
        std::fs::write(&executable, include_str!("plugins/fake_acp_auth_work.py"))?;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin.path().to_owned()).chain(std::env::split_paths(&path)),
    )?;
    Ok((
        bin,
        super::fixture()
            .env("PATH", path)
            .seed_file("workspace/.auth-work-version", version.as_bytes().to_vec()),
    ))
}

async fn create_agent(ordinary: &UserSession, agent: ExternalAgent) -> Result<i64> {
    let workspace_id = ordinary
        .snapshot()
        .await?
        .session
        .workspace_id
        .context("workspace")?;
    Ok(ordinary
        .client()
        .create_session_configured(CreateSessionParams {
            external_agent: Some(agent),
            workspace_id,
            title: "Native account and work".into(),
            parent_id: None,
            auto_title: false,
            profile: None,
            prompts: None,
        })
        .await?
        .id)
}

async fn auth_status_scope(version: &'static str) -> TestResult {
    let (_bin, builder) = configured_agents(version)?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let session = create_agent(&ordinary, ExternalAgent::ClaudeCode).await?;
            let sibling = create_agent(&ordinary, ExternalAgent::ClaudeCode).await?;
            let ignored = create_agent(&ordinary, ExternalAgent::Antigravity).await?;
            let user = case.user().attach_session(session).await?;
            let client = user.client();
            let run = user.send("report native account").await?;
            run.wait_settled().await?.assert_succeeded()?;

            let command = case
                .run_cli(["sessions", "agent-auth", &session.to_string(), "--live"])
                .await?;
            let stdout = &command.require_success()?.stdout;
            let status: ExternalAgentAuthStatus = serde_json::from_str(stdout)?;
            ensure!(status.state == "authenticated" && status.accounts[0].label == "Live account");
            ensure!(status.accounts[0].description.as_deref() == Some("Native account"));
            ensure!(!stdout.contains("do-not-export"));
            ensure!(client.external_agent_live_auth(sibling).await?.is_none());

            let inspect = client
                .external_agent_auth(
                    session,
                    ExternalAgentAuthAction::Inspect { terminal: false },
                )
                .await?;
            ensure!(inspect.state.as_deref() == Some("not_required") && inspect.logout_supported);
            ensure!(inspect.methods.is_empty());
            let logout = client
                .external_agent_auth(session, ExternalAgentAuthAction::Logout)
                .await?;
            ensure!(logout.state.as_deref() == Some("not_required"));
            ensure!(client.external_agent_live_auth(session).await? == Some(status));

            let unsupported = case.user().attach_session(ignored).await?;
            unsupported
                .send("report native account")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            ensure!(client.external_agent_live_auth(ignored).await?.is_none());
            ensure!(client.external_agent_work(ignored, None).await?.is_empty());
            ensure!(case.workspace().join(".agy-downgraded").is_file());
            ensure!(
                case.workspace()
                    .join(".claude-agent-acp-downgraded")
                    .is_file()
                    == (version == "1")
            );

            tokio::fs::write(
                case.workspace()
                    .join(format!(".auth-work-update-{version}")),
                "",
            )
            .await?;
            let idle = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Some(status) = client.external_agent_live_auth(session).await?
                        && status.state == "unknown"
                    {
                        break anyhow::Ok(status);
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await??;
            ensure!(idle.accounts.is_empty());

            client
                .external_agent_session(session, ExternalAgentSessionAction::Close)
                .await?;
            ensure!(client.external_agent_live_auth(session).await?.is_none());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_v1_auth_status_tracks_only_its_owner() -> TestResult {
    auth_status_scope("1").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_v2_auth_status_tracks_only_its_owner() -> TestResult {
    auth_status_scope("2").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_v1_cancels_negotiated_air_tasks_on_the_root() -> TestResult {
    let (_bin, builder) = configured_agents("1")?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let session = create_agent(&ordinary, ExternalAgent::ClaudeCode).await?;
            let user = case.user().attach_session(session).await?;
            let client = user.client();
            user.send("control legacy tasks")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            let work = client.external_agent_work(session, None).await?;
            ensure!(work.len() == 2, "{work:?}");
            let child = work
                .iter()
                .find(|work| work.target_id == "child-task")
                .context("child task")?;
            ensure!(
                child.kind == ExternalAgentWorkKind::AsyncTask
                    && child.session_id == "auth-work-child"
            );
            let stopped = client
                .external_agent_work(session, Some(child.cancel_id))
                .await?;
            ensure!(stopped.len() == 1 && stopped[0].target_id == "refused");
            ensure!(
                client
                    .external_agent_work(session, Some(child.cancel_id))
                    .await
                    .is_err()
            );
            ensure!(
                client
                    .external_agent_work(session, Some(stopped[0].cancel_id))
                    .await
                    .is_err()
            );
            ensure!(client.external_agent_work(session, None).await?.is_empty());
            Ok(())
        })
        .await
}
