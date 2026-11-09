//! Permanent Bot navigation remains usable during approvals and after deletion.
use std::time::Duration;

use agena_testkit::{Reply, RequestMatch, TestResult};
use anyhow::{Context, ensure};
use reqwest::Method;
use serde_json::{Value, json};

use super::fixture;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_entry_reopens_while_a_project_permission_card_is_pending() -> TestResult {
    fixture()
        .case_timeout(Duration::from_secs(90))
        .run(|case| async move {
            let client = case.client().await;
            let id = client.ensure_bot_session().await?.session_id;
            let bot = case.user().attach_session(id).await?;
            let directory = case.workspace().join("permission-pending-project");
            tokio::fs::create_dir(&directory).await?;
            client
                .set_session_permission(
                    id,
                    serde_json::from_value(json!({
                        "path": {"rules": {(directory.to_string_lossy().as_ref()): {"read":"ask", "write":"deny"}}}
                    }))?,
                )
                .await?;
            let model = case.model();
            let start = model
                .expect(RequestMatch::user_text("bot-permission-reopen"))
                .reply(Reply::tool_call(
                    "bot-register-pending",
                    "bot.sessions",
                    json!({"action":"register_project", "path":directory,
                        "user_quote":"bot-permission-reopen", "request_key":"register-for-reopen-test"}),
                ));
            model
                .expect(RequestMatch::tool_output("bot-register-pending"))
                .after(&start)
                .check_tool_output_contains("bot-register-pending", "workspace_id")
                .reply(Reply::text("project registration approved"));
            let run = bot.send("bot-permission-reopen").await?;
            let approval = run.wait_permission().await?;
            let reopened = tokio::time::timeout(Duration::from_secs(3), client.ensure_bot_session())
                .await
                .context("Bot navigation was blocked by its own pending permission card")??;
            ensure!(reopened.session_id == id);
            approval.allow_once().await?;
            run.wait_settled().await?.assert_succeeded()?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_entry_recovers_after_human_deletes_its_conversation() -> TestResult {
    fixture()
        .case_timeout(Duration::from_secs(90))
        .run(|case| async move {
            let client = case.client().await;
            let old = client.ensure_bot_session().await?.session_id;
            let http = case.http().await;
            let value: Value = http
                .get(&format!("/api/v1/sessions/{old}"))
                .await?
                .require_success()?
                .json()?;
            let version = value["version"].as_i64().context("session version")?;
            let request = http
                .request(Method::DELETE, &format!("/api/v1/sessions/{old}"))?
                .header("if-match", format!("\"{version}\""));
            http.execute(request).await?.require_success()?;
            let mut requests = Vec::new();
            for _ in 0..8 {
                let client = client.clone();
                requests.push(tokio::spawn(
                    async move { client.ensure_bot_session().await },
                ));
            }
            let mut ids = Vec::new();
            for request in requests {
                ids.push(request.await??.session_id);
            }
            let replacement = ids[0];
            ensure!(
                replacement > 0 && replacement != old && ids.iter().all(|id| *id == replacement)
            );
            case.crash().await?;
            case.restart().await?;
            ensure!(case.client().await.ensure_bot_session().await?.session_id == replacement);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retired_task_routes_and_tools_are_absent() -> TestResult {
    fixture()
        .run(|case| async move {
            let id = case.client().await.ensure_bot_session().await?.session_id;
            let http = case.http().await;
            for path in [
                "/api/v1/tasks",
                "/api/v1/tasks/00000000-0000-0000-0000-000000000001",
            ] {
                ensure!(
                    http.get(path).await?.status == 404,
                    "retired route remains: {path}"
                );
            }
            for path in [
                "/api/v1/tasks",
                "/api/v1/chatbot/messages",
                "/api/v1/tasks/00000000-0000-0000-0000-000000000001/cancel",
                "/api/v1/tasks/00000000-0000-0000-0000-000000000001/resume",
                "/api/v1/tasks/00000000-0000-0000-0000-000000000001/accept",
            ] {
                ensure!(
                    http.send_json(Method::POST, path, &json!({})).await?.status == 404,
                    "retired mutation remains: {path}"
                );
            }
            let bot = case.user().attach_session(id).await?;
            let model = case.model();
            let start = model
                .expect(RequestMatch::user_text("bot-no-task-worker"))
                .reply(Reply::tool_call(
                    "bot-retired-tool",
                    "work.manage",
                    json!({"action":"start"}),
                ));
            model
                .expect(RequestMatch::tool_output("bot-retired-tool"))
                .after(&start)
                .check(|request| {
                    let output = request
                        .tool_output_text("bot-retired-tool")
                        .context("missing tool result")?;
                    let value: Value = serde_json::from_str(&output)?;
                    ensure!(
                        value["phase"] == "failed"
                            && value["result"]["error"]["category"] == "not_found"
                            && value["result"]["error"]["code"] == "tool.unavailable",
                        "retired tool must be unregistered, not permission-blocked: {output}"
                    );
                    Ok(())
                })
                .reply(Reply::text("The retired tool is absent."));
            bot.send("bot-no-task-worker")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            Ok(())
        })
        .await
}
