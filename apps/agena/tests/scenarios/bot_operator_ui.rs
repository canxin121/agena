//! Real browser and terminal acceptance of the same canonical Bot conversation.
//! Models and workspaces are isolated fixtures; no operator service is touched.
use std::{path::PathBuf, time::Duration};

use agena_testkit::{Reply, RequestMatch, TestResult};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

use super::fixture;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Chrome, Node, Web dependencies and tmux; real Web/TUI Bot acceptance"]
async fn bot_web_and_terminal_share_normal_conversation() -> TestResult {
    fixture()
        .without_password()
        .case_timeout(Duration::from_secs(480))
        .run(|case| async move {
            let source = case.user().new_session().await?;
            let http = case.http().await;
            let source_before: Value = http.get(&format!("/api/v1/sessions/{}", source.id()))
                .await?.require_success()?.json()?;
            let source_version = source_before["version"].as_i64().context("source metadata version")?;
            let model = case.model();
            let organize = model
                .expect(RequestMatch::user_text("bot-ui-web-message"))
                .reply(Reply::tool_call("bot-ui-organize", "bot.sessions", json!({
                    "action":"organize", "session_id":source.id(), "favorite":true, "pinned":true,
                    "expected_version":source_version, "user_quote":"bot-ui-web-message",
                    "request_key":"browser-organizes-native-session"
                })));
            let browser = model
                .expect(RequestMatch::tool_output("bot-ui-organize"))
                .after(&organize)
                .check_tool_output_contains("bot-ui-organize", "succeeded")
                .reply(Reply::text(format!(
                    "bot-ui-web-reply: organized the [ordinary project conversation](/chat?sessionId={}).",
                    source.id()
                )));
            let project = model
                .expect(RequestMatch::user_text("bot-ui-project-message"))
                .reply(Reply::text(
                    "bot-ui-project-reply: ordinary project conversation.",
                ));
            let terminal = model
                .expect(RequestMatch::user_text("bot-ui-tui-message"))
                .check(|request| {
                    ensure!(
                        request.body.to_string().contains("bot-ui-web-reply"),
                        "TUI must continue the same Bot transcript previously used in Web"
                    );
                    Ok(())
                })
                .reply(Reply::text(
                    "bot-ui-tui-reply: shared Bot history verified.",
                ));
            let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            let evidence = std::env::temp_dir()
                .join(format!("agena-bot-ui-evidence-{}", uuid::Uuid::new_v4()));
            for (runtime, script) in [
                ("node", "scripts/agena/tests/probe_bot_web.cjs"),
                ("python3", "scripts/agena/tests/probe_bot_tui.py"),
            ] {
                let output = tokio::time::timeout(
                    Duration::from_secs(230),
                    tokio::process::Command::new(runtime)
                        .arg(repository.join(script))
                        .env("BOT_BACKEND", http.base_url())
                        .env("BOT_PROJECT_SESSION", source.id().to_string())
                        .env("BOT_EXPECT_ORGANIZED", "1")
                        .env("BOT_EVIDENCE", &evidence)
                        .env("AGENA_TEST_BINARY", env!("CARGO_BIN_EXE_agena"))
                        .current_dir(case.workspace())
                        .kill_on_drop(true)
                        .output(),
                )
                .await
                .context("Bot UI acceptance probe timed out")??;
                println!(
                    "Bot evidence: {}\n{}",
                    evidence.display(),
                    String::from_utf8_lossy(&output.stdout)
                );
                ensure!(
                    output.status.success(),
                    "{script} failed:\n{}\n{}",
                    String::from_utf8_lossy(&output.stderr),
                    String::from_utf8_lossy(&output.stdout)
                );
            }
            let source_after: Value = http.get(&format!("/api/v1/sessions/{}", source.id()))
                .await?.require_success()?.json()?;
            ensure!(source_after["favorite"] == true && source_after["pinned"] == true,
                "Bot organization must update the same native session used by the human clients");
            organize.finished().await?;
            browser.finished().await?;
            project.finished().await?;
            terminal.finished().await?;
            Ok(())
        })
        .await
}
