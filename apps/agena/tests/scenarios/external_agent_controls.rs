use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, time::Duration};

use agena_api::commands::CreateSessionParams;
use agena_domain::{
    EXTERNAL_AGENT_URL_OPEN_OPTION, ExternalAgent, ExternalAgentAuth,
    ExternalAgentAuthAction as Auth, ExternalAgentAuthFlow, ExternalAgentOptionChange,
    ExternalAgentOptionKind as Kind, ExternalAgentOptionType, ExternalAgentWorkItem,
    ExternalAgentWorkKind,
};
use agena_testkit::{TestAgenaBuilder, TestResult, UserSession};
use anyhow::{Context, Result, ensure};

fn configured_agents(version: &str) -> Result<(tempfile::TempDir, TestAgenaBuilder)> {
    let bin = tempfile::tempdir()?;
    for name in ["agy", "claude-agent-acp"] {
        let executable = bin.path().join(name);
        std::fs::write(&executable, include_str!("plugins/fake_acp_controls.py"))?;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin.path().to_owned()).chain(std::env::split_paths(&path)),
    )?;
    Ok((
        bin,
        super::fixture().env("PATH", path).seed_file(
            "workspace/.acp-controls-version",
            version.as_bytes().to_vec(),
        ),
    ))
}

async fn create_with_agent(ordinary: &UserSession, agent: ExternalAgent) -> Result<i64> {
    let workspace_id = ordinary
        .snapshot()
        .await?
        .session
        .workspace_id
        .context("workspace")?;
    let created = ordinary
        .client()
        .create_session_configured(CreateSessionParams {
            external_agent: Some(agent),
            workspace_id,
            title: "Native controls".into(),
            parent_id: None,
            auto_title: false,
            profile: None,
            prompts: None,
        })
        .await?;
    Ok(created.id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_config_url_and_permission() -> TestResult {
    let (_bin, builder) = configured_agents("1")?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let session = create_with_agent(&ordinary, ExternalAgent::ClaudeCode).await?;
            let user = case.user().attach_session(session).await?;
            let client = user.client();
            let options = client.external_agent_options(session, None).await?;
            let toggle = options
                .options
                .iter()
                .find(|option| option.id == "fast")
                .context("fast toggle")?;
            ensure!(
                toggle.kind == Kind::Config("fast".into())
                    && toggle.value_type == ExternalAgentOptionType::Boolean
            );
            ensure!(toggle.current_value == "false");
            let prompt = |kind, value: &str| {
                Some(ExternalAgentOptionChange {
                    kind,
                    value: Some(value.into()),
                })
            };
            case.run_cli([
                "sessions",
                "agent-options",
                &session.to_string(),
                "--option",
                "config:fast=true",
            ])
            .await?
            .require_success()?;
            client
                .external_agent_options(session, prompt(Kind::Mode, "plan"))
                .await?;

            let running = user.send("exercise native controls").await?;
            let permission = running.wait_user_input().await?;
            let asked = permission
                .request
                .request
                .as_user_input()
                .context("native approval")?;
            let offered: Vec<_> = asked.questions[0]
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect();
            ensure!(
                offered
                    == [
                        "Approve edit",
                        "Remember approval",
                        "Deny edit",
                        "Remember denial"
                    ]
            );
            permission
                .answer(BTreeMap::from([(
                    "0".into(),
                    vec!["Remember approval".into()],
                )]))
                .await?;
            let request = running.wait_user_input().await?;
            let asked = request
                .request
                .request
                .as_user_input()
                .context("authorization request")?;
            ensure!(asked.kind.as_str() == "external_agent_url");
            ensure!(asked.questions[0].question == "https://example.invalid/connect?state=test");
            request
                .answer(BTreeMap::from([(
                    "0".into(),
                    vec![EXTERNAL_AGENT_URL_OPEN_OPTION.into()],
                )]))
                .await?;

            let completed = running.wait_settled().await?;
            completed.assert_succeeded()?;
            completed.assert_assistant_text("native controls succeeded")?;
            let parts = user.parts().await?;
            ensure!(
                parts
                    .iter()
                    .filter(|part| part.content["name"] == "external_agent_authorization")
                    .count()
                    == 2
            );
            let live = user
                .snapshot()
                .await?
                .execution
                .external_agent_options
                .context("published native configuration")?;
            ensure!(
                live.options
                    .iter()
                    .any(|option| option.kind == Kind::Mode && option.current_value == "review")
            );
            ensure!(live.selection.get(&Kind::Mode).map(String::as_str) == Some("plan"));

            let legacy_id = create_with_agent(&ordinary, ExternalAgent::Antigravity).await?;
            let legacy = case.user().attach_session(legacy_id).await?;
            let available = legacy
                .client()
                .external_agent_options(legacy_id, None)
                .await?;
            ensure!(available.options[0].kind == Kind::Mode && available.options[0].legacy_mode);
            legacy
                .client()
                .external_agent_options(legacy_id, prompt(Kind::Mode, "plan"))
                .await?;
            let completed = legacy.send("check old modes").await?.wait_settled().await?;
            completed.assert_succeeded()?;
            completed.assert_assistant_text("legacy mode succeeded")?;
            let live = legacy
                .snapshot()
                .await?
                .execution
                .external_agent_options
                .context("published native mode")?;
            ensure!(live.options[0].current_value == "review");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_authenticate_and_logout() -> TestResult {
    let (_bin, builder) = configured_agents("1")?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let session = create_with_agent(&ordinary, ExternalAgent::ClaudeCode).await?;
            let client = ordinary.client();
            let auth = client
                .external_agent_auth(session, Auth::Inspect { terminal: false })
                .await?;
            ensure!(auth.methods.len() == 1 && auth.methods[0].id == "browser");
            ensure!(auth.state.as_deref() == Some("unauthenticated") && auth.logout_supported);
            let terminal = client
                .external_agent_auth(session, Auth::Inspect { terminal: true })
                .await?;
            ensure!(terminal.methods.len() == 2);
            let ExternalAgentAuthFlow::Terminal { args, env } = &terminal.methods[1].flow else {
                anyhow::bail!("did not discover the terminal login");
            };
            ensure!(args == &["--login"] && env.get("NO_BROWSER").map(String::as_str) == Some("1"));
            ensure!(
                client
                    .external_agent_auth(
                        session,
                        Auth::Login {
                            method_id: "terminal".into()
                        }
                    )
                    .await
                    .is_err()
            );
            let session_id = session.to_string();
            let command = ["sessions", "agent-auth", session_id.as_str()];
            let inspect = case.run_cli(command).await?;
            let inspect: ExternalAgentAuth = serde_json::from_str(&inspect.require_success()?.stdout)?;
            ensure!(inspect.methods.len() == 1 && inspect.methods[0].id == "browser");

            let inspect = case
                .run_cli(command.into_iter().chain(["--terminal"]))
                .await?;
            let inspect: ExternalAgentAuth = serde_json::from_str(&inspect.require_success()?.stdout)?;
            ensure!(inspect.methods.len() == 2 && inspect.methods[1].id == "terminal");
            ensure!(matches!(&inspect.methods[1].flow, ExternalAgentAuthFlow::Terminal { env, .. } if env.is_empty()));

            let omitted = case
                .run_cli(command.into_iter().chain(["--login", "terminal"]))
                .await?;
            ensure!(omitted.require_failure()?.stderr.contains("Use --terminal"));

            let noninteractive = case
                .run_cli(command.into_iter().chain(["--terminal", "--login", "terminal"]))
                .await?;
            ensure!(noninteractive.require_failure()?.stderr.contains("interactive terminal"));

            let login = case
                .run_cli(command.into_iter().chain(["--login", "browser"]))
                .await?;
            let login: ExternalAgentAuth = serde_json::from_str(&login.require_success()?.stdout)?;
            ensure!(login.state.as_deref() == Some("authenticated"));
            ensure!(login.accounts[0].label == "Test account");
            let logout = case
                .run_cli(command.into_iter().chain(["--logout"]))
                .await?;
            let logout: ExternalAgentAuth = serde_json::from_str(&logout.require_success()?.stdout)?;
            ensure!(
                logout.state.as_deref() == Some("unauthenticated") && logout.accounts.is_empty()
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_v2_auth_and_config_option_wire_shapes() -> TestResult {
    let (_bin, builder) = configured_agents("2")?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let session = create_with_agent(&ordinary, ExternalAgent::ClaudeCode).await?;
            let client = ordinary.client();
            let inspect = client
                .external_agent_auth(session, Auth::Inspect { terminal: true })
                .await?;
            ensure!(inspect.methods.len() == 2);
            ensure!(
                matches!(&inspect.methods[1].flow, ExternalAgentAuthFlow::Terminal { env, .. }
            if env.get("NO_BROWSER").map(String::as_str) == Some("1"))
            );
            let auth = client
                .external_agent_auth(
                    session,
                    Auth::Login {
                        method_id: "browser".into(),
                    },
                )
                .await?;
            ensure!(auth.state.as_deref() == Some("authenticated"));
            let settings = client
                .external_agent_options(
                    session,
                    Some(ExternalAgentOptionChange {
                        kind: Kind::Mode,
                        value: Some("plan".into()),
                    }),
                )
                .await?;
            ensure!(
                settings
                    .options
                    .iter()
                    .any(|option| option.kind == Kind::Mode && option.current_value == "plan")
            );
            let settings = client
                .external_agent_options(
                    session,
                    Some(ExternalAgentOptionChange {
                        kind: Kind::Config("fast".into()),
                        value: Some("true".into()),
                    }),
                )
                .await?;
            ensure!(
                settings
                    .options
                    .iter()
                    .any(|option| option.id == "fast" && option.current_value == "true")
            );
            let auth = client.external_agent_auth(session, Auth::Logout).await?;
            ensure!(auth.state.as_deref() == Some("unauthenticated"));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_agent_v2_cancels_only_live_advertised_work() -> TestResult {
    let (_bin, builder) = configured_agents("2")?;
    builder
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let session = create_with_agent(&ordinary, ExternalAgent::ClaudeCode).await?;
            let user = case.user().attach_session(session).await?;
            let client = user.client();
            let run = user.send("check targeted cancellation").await?;
            run.wait_text_contains("Targets are visible").await?;

            let work = client.external_agent_work(session, None).await?;
            ensure!(work.len() == 4, "{work:?}");
            let target = |kind, id| -> Result<_> {
                work.iter()
                    .find(|item| item.kind == kind && item.target_id == id)
                    .with_context(|| format!("target {kind:?} {id}"))
            };
            let revoked = target(ExternalAgentWorkKind::Terminal, "revoked")?.cancel_id;
            let terminal = target(ExternalAgentWorkKind::Terminal, "term")?;
            let tool = target(ExternalAgentWorkKind::ToolCall, "job")?;
            let child = target(ExternalAgentWorkKind::Subagent, "work-child")?;
            ensure!(
                tool.session_id == child.session_id && tool.title.as_deref() == Some("Worker job")
            );
            ensure!(
                client
                    .external_agent_work(ordinary.id(), Some(terminal.cancel_id))
                    .await
                    .is_err()
            );

            tokio::fs::write(case.workspace().join(".acp-work-revoke"), []).await?;
            tokio::time::timeout(Duration::from_secs(5), async {
                while client
                    .external_agent_work(session, None)
                    .await?
                    .iter()
                    .any(|item| item.cancel_id == revoked)
                {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                anyhow::Ok(())
            })
            .await??;
            ensure!(
                client
                    .external_agent_work(session, Some(revoked))
                    .await
                    .is_err()
            );

            let list = case
                .run_cli(["sessions", "agent-work", &session.to_string()])
                .await?;
            let listed: Vec<ExternalAgentWorkItem> =
                serde_json::from_str(&list.require_success()?.stdout)?;
            ensure!(listed.len() == 3);
            case.run_cli([
                "sessions",
                "agent-work",
                &session.to_string(),
                "--cancel",
                &terminal.cancel_id.to_string(),
            ])
            .await?
            .require_success()?;
            ensure!(
                client
                    .external_agent_work(session, Some(terminal.cancel_id))
                    .await
                    .is_err()
            );

            ensure!(
                client
                    .external_agent_work(session, Some(tool.cancel_id))
                    .await
                    .is_err()
            );
            let refreshed = client.external_agent_work(session, None).await?;
            let retry = refreshed
                .iter()
                .find(|item| item.kind == ExternalAgentWorkKind::ToolCall)
                .context("retried native tool")?;
            ensure!(retry.cancel_id != tool.cancel_id);
            client
                .external_agent_work(session, Some(retry.cancel_id))
                .await?;
            client
                .external_agent_work(session, Some(child.cancel_id))
                .await?;

            let completed = run.wait_settled().await?;
            completed.assert_succeeded()?;
            completed.assert_text_contains("Parent kept going")?;
            ensure!(client.external_agent_work(session, None).await?.is_empty());

            tokio::fs::write(case.workspace().join(".acp-work-idle"), []).await?;
            let late = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let items = client.external_agent_work(session, None).await?;
                    if let Some(item) = items.into_iter().find(|item| item.target_id == "late") {
                        break anyhow::Ok(item);
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await??;
            ensure!(
                client
                    .external_agent_work(session, Some(late.cancel_id))
                    .await?
                    .is_empty()
            );
            ensure!(
                client
                    .external_agent_work(session, Some(late.cancel_id))
                    .await
                    .is_err()
            );
            Ok(())
        })
        .await
}
