//! Completion must reach a conversation's live tail without a snapshot poll.
use std::time::{Duration, Instant};

use agena_api::{Scope, live::SessionChangeResource};
use agena_client::SubscriptionEvent;
use agena_testkit::{Reply, RequestMatch, TestResult};
use anyhow::{Context, ensure};

use super::{
    bot_profiles::{create, ensure_named},
    fixture,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires local Chrome, Node, Web dependencies and tmux"]
async fn live_reply_web_and_tui_without_refresh() -> TestResult {
    fixture().without_password().case_timeout(Duration::from_secs(240)).run(|case| async move {
        let a = create(&case, "live-ui-a", "Live Bot A", None).await?;
        let b = create(&case, "live-ui-b", "Live Bot B", None).await?;
        let a_session = ensure_named(&case, a.id).await?.session_id;
        let b_session = ensure_named(&case, b.id).await?.session_id;
        let project = case.user().new_session().await?;
        let mut cases = Vec::new();
        for (label, session_id, bot_id) in [
            ("bot-a", a_session, Some(a.id.to_string())),
            ("bot-b", b_session, Some(b.id.to_string())),
            ("project", project.id(), None),
        ] {
            let web_prompt = format!("live-web-{label}-prompt");
            let web_reply = format!("LIVE-WEB-{label}-COMPLETE");
            let tui_prompt = format!("live-tui-{label}-prompt");
            let tui_reply = format!("LIVE-TUI-{label}-COMPLETE");
            case.model().expect(RequestMatch::user_text(&web_prompt)).reply(Reply::text(&web_reply)).with_timeout(Duration::from_secs(200));
            case.model().expect(RequestMatch::user_text(&tui_prompt)).reply(Reply::text(&tui_reply)).with_timeout(Duration::from_secs(200));
            cases.push(serde_json::json!({"label":label,"session_id":session_id,"bot_id":bot_id,"web_prompt":web_prompt,"web_reply":web_reply,"tui_prompt":tui_prompt,"tui_reply":tui_reply}));
        }
        let repository = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let evidence = std::env::temp_dir().join(format!("agena-live-reply-evidence-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&evidence).await?;
        println!("LIVE_REPLY_EVIDENCE {}", evidence.display());
        for (runtime, script) in [("node", "probe_live_reply_web.cjs"), ("python3", "probe_live_reply_tui.py")] {
            let output = tokio::time::timeout(Duration::from_secs(150), tokio::process::Command::new(runtime)
                .arg(repository.join("scripts/agena/tests").join(script))
                .env("LIVE_BACKEND", case.http().await.base_url())
                .env("LIVE_CASES", serde_json::to_string(&cases)?)
                .env("LIVE_EVIDENCE", &evidence)
                .env("AGENA_TEST_BINARY", env!("CARGO_BIN_EXE_agena"))
                .current_dir(case.workspace()).kill_on_drop(true).output()).await.context("live UI probe deadline")??;
            println!("{}", String::from_utf8_lossy(&output.stdout));
            ensure!(output.status.success(), "{script} failed:\n{}\nEvidence: {}", String::from_utf8_lossy(&output.stderr), evidence.display());
        }
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_completion_reaches_project_and_workspace_free_bot_sessions() -> TestResult {
    fixture()
        .run(|case| async move {
            let bot = create(&case, "live-regression", "Live delivery regression", None).await?;
            let bot_id = ensure_named(&case, bot.id).await?.session_id;
            let project = case.user().new_session().await?;
            let client = case.client().await;
            for (session_id, prompt, reply) in [
                (
                    project.id(),
                    "live-project-prompt",
                    "live project completion",
                ),
                (bot_id, "live-global-bot-prompt", "live Bot completion"),
            ] {
                let session = case.user().attach_session(session_id).await?;
                if session_id == bot_id {
                    ensure!(session.snapshot().await?.session.workspace_id.is_none());
                }
                let mut stream = client.stream_changes(Scope::Session { session_id }).await?;
                case.model()
                    .expect(RequestMatch::user_text(prompt))
                    .reply(Reply::text(reply));
                let started = Instant::now();
                let observe = async {
                    tokio::time::timeout(Duration::from_secs(8), async {
                        let mut events = 0;
                        while let Some(event) = stream.recv().await {
                            if let SubscriptionEvent::SessionChanged(
                                SessionChangeResource::PartUpdated {
                                    session_id: owner,
                                    part,
                                },
                            ) = event?
                            {
                                ensure!(
                                    owner == session_id,
                                    "another session's transcript was delivered"
                                );
                                events += 1;
                                if part.role == "assistant"
                                    && part.is_message_boundary()
                                    && matches!(
                                        part.run_state.as_deref().unwrap_or(&part.state),
                                        "completed" | "succeeded"
                                    )
                                {
                                    return Ok::<_, anyhow::Error>(events);
                                }
                            }
                        }
                        anyhow::bail!("session stream ended before completion")
                    })
                    .await
                    .context("terminal reply was not delivered live within 8s")?
                };
                let send = async {
                    session
                        .send(prompt)
                        .await?
                        .wait_settled()
                        .await?
                        .assert_succeeded()?;
                    Ok::<_, anyhow::Error>(())
                };
                let (events, ()) = tokio::try_join!(observe, send)?;
                println!(
                    "LIVE_COMPLETION session={session_id} events={events} submit_to_terminal_ms={}",
                    started.elapsed().as_millis()
                );
            }
            Ok(())
        })
        .await
}
