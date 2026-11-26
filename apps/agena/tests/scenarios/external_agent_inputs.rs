use std::{collections::BTreeMap, os::unix::fs::PermissionsExt as _};

use agena_api::commands::CreateSessionParams;
use agena_domain::{EXTERNAL_AGENT_URL_OPEN_OPTION, ExternalAgent, ExternalAgentAuthAction};
use agena_testkit::TestResult;
use anyhow::{Context as _, ensure};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_authentication_and_idle_interactions_use_the_normal_input_channel() -> TestResult {
    let bin = tempfile::tempdir()?;
    let agent = ExternalAgent::ClaudeCode;
    let executable = bin.path().join(agent.command().0);
    std::fs::write(&executable, include_str!("plugins/fake_acp_inputs.py"))?;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(bin.path().to_owned()).chain(std::env::split_paths(&path)),
    )?;
    super::fixture()
        .env("PATH", path)
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace_id = ordinary
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("workspace")?;
            let client = ordinary.client().clone();
            let created = client
                .create_session_configured(CreateSessionParams {
                    external_agent: Some(agent),
                    workspace_id,
                    title: "Independent ACP interactions".into(),
                    parent_id: None,
                    auto_title: false,
                    profile: None,
                    prompts: None,
                })
                .await?;
            let session = created.id;
            let user = case.user().attach_session(session).await?;
            let sign_in = tokio::spawn(async move {
                client
                    .external_agent_auth(
                        session,
                        ExternalAgentAuthAction::Login {
                            method_id: "interactive".into(),
                        },
                    )
                    .await
            });
            let login = user.wait_user_input().await?;
            ensure!(
                login
                    .request
                    .request
                    .as_user_input()
                    .context("login form")?
                    .body_markdown
                    == "Confirm sign-in"
            );
            let presented = login.mark_presented().await?;
            ensure!(
                presented
                    .session
                    .state
                    .pending_interactive_requests()
                    .iter()
                    .filter_map(|item| item.request.as_user_input())
                    .any(|item| item.presented_at.is_some())
            );
            let answers = |value: &str| BTreeMap::from([("0".into(), vec![value.into()])]);
            ensure!(login.answer(answers("99")).await.is_err());
            login.answer(answers("7")).await?;
            ensure!(sign_in.await??.state.as_deref() == Some("authenticated"));

            let completed = user.send("Start").await?.wait_settled().await?;
            completed.assert_succeeded()?;
            tokio::fs::write(case.workspace().join(".ask-while-idle"), "").await?;
            let permission = user.wait_user_input().await?;
            ensure!(
                permission
                    .request
                    .request
                    .as_user_input()
                    .context("permission")?
                    .questions[0]
                    .question
                    .contains("Background permission")
            );
            permission.answer(answers("Reject once")).await?;
            user.wait_state("withdrawable link published", |state| {
                let requests = state.session.state.pending_interactive_requests();
                requests.len() == 1
                    && requests.iter().any(|item| {
                        item.request
                            .as_user_input()
                            .is_some_and(|input| input.body_markdown == "Withdrawn link")
                    })
            })
            .await?;
            let withdraw = user.wait_user_input().await?;
            ensure!(
                withdraw
                    .request
                    .request
                    .as_user_input()
                    .context("withdrawable link")?
                    .body_markdown
                    == "Withdrawn link"
            );
            tokio::fs::write(case.workspace().join(".withdraw-idle"), "").await?;
            user.wait_state("withdrawn input retired and next link shown", |state| {
                let requests = state.session.state.pending_interactive_requests();
                requests.len() == 1
                    && requests.iter().any(|item| {
                        item.request
                            .as_user_input()
                            .is_some_and(|input| input.body_markdown == "Open link")
                    })
            })
            .await?;
            ensure!(
                withdraw
                    .answer(answers(EXTERNAL_AGENT_URL_OPEN_OPTION))
                    .await
                    .is_err()
            );
            user.wait_user_input()
                .await?
                .answer(answers(EXTERNAL_AGENT_URL_OPEN_OPTION))
                .await?;
            user.wait_state(
                "authorization complete with independent question remaining",
                |state| {
                    state
                        .execution
                        .external_agent_details
                        .notices
                        .iter()
                        .any(|note| note.title == "Authorization completed")
                        && state
                            .session
                            .state
                            .pending_interactive_requests()
                            .iter()
                            .any(|item| {
                                item.request
                                    .as_user_input()
                                    .is_some_and(|input| input.body_markdown == "Unanswered form")
                            })
                },
            )
            .await?;
            let old = user.wait_user_input().await?;
            user.send("Resume")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            ensure!(old.answer(answers("7")).await.is_err());
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
