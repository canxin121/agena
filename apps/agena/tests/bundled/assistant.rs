//! Registration coverage is tied to real model/tool/runtime behavior, not a
//! successful manifest read. Detailed edge cases live in system scenarios.
use super::{BundledBehaviorCase, fixture};
use agena_testkit::{Reply, RequestMatch, TestResult};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

pub(super) const CASES: &[BundledBehaviorCase] = &[
    (
        "agena.companion",
        companion_changes_workspace_preferences_through_a_real_user_turn,
    ),
    ("agena.bot", bot_creates_a_real_visible_project_session),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn companion_changes_workspace_preferences_through_a_real_user_turn() -> TestResult {
    fixture().run(|case|async move{
        let model=case.model();let source=case.user().new_session().await?;
        let first=model.expect(RequestMatch::user_text("保存这个助手称呼"))
            .reply(Reply::tool_call("companion-registration","companion.manage",json!({"action":"configure","user_quote":"保存这个助手称呼","expected_revision":0,"patch":{"name":"registered-companion-behavior","tone":"direct"}})));
        model.expect(RequestMatch::tool_output("companion-registration")).after(&first).check_tool_output_contains("companion-registration","saved").reply(Reply::text("称呼已经保存。"));
        source.send("保存这个助手称呼").await?.wait_settled().await?.assert_succeeded()?;
        ensure!(source.client().assistant_snapshot(source.id()).await?.profile.preferences.name=="registered-companion-behavior");
        let next=case.user().new_session().await?;
        model.expect(RequestMatch::user_text("新会话验证人格"))
            .check(|request|{ensure!(request.body.to_string().contains("registered-companion-behavior"));Ok(())}).reply(Reply::text("已经继承当前工作区的人格。"));
        next.send("新会话验证人格").await?.wait_settled().await?.assert_succeeded()?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_creates_a_real_visible_project_session() -> TestResult {
    fixture()
        .run(|case| async move {
            let client = case.client().await;
            let bot = case
                .user()
                .attach_session(client.ensure_bot_session().await?.session_id)
                .await?;
            let http = case.http().await;
            let resource: Value = http
                .get(&format!("/api/v1/sessions/{}", bot.id()))
                .await?
                .require_success()?
                .json()?;
            let workspace = resource["workspace_id"].as_i64().context("Bot workspace")?;
            let model = case.model();
            let start = model.expect(RequestMatch::user_text("create a project conversation"))
            .reply(Reply::tool_call("bot-registration", "bot.sessions", json!({
                "action":"create", "workspace_id":workspace, "title":"Registered Bot behavior",
                "user_quote":"create a project conversation", "request_key":"builtin-bot-create"
            })));
            model
                .expect(RequestMatch::tool_output("bot-registration"))
                .after(&start)
                .check_tool_output_contains("bot-registration", "session_id")
                .reply(Reply::text("Project conversation created."));
            bot.send("create a project conversation")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            let sessions: Value = http
                .get("/api/v1/sessions")
                .await?
                .require_success()?
                .json()?;
            ensure!(sessions.to_string().contains("Registered Bot behavior"));
            ensure!(http.get("/api/v1/tasks").await?.status == 404);
            Ok(())
        })
        .await
}
