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
    (
        "agena.work",
        work_delegates_and_stops_a_real_worker_from_the_same_conversation,
    ),
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
async fn work_delegates_and_stops_a_real_worker_from_the_same_conversation() -> TestResult {
    fixture().run(|case|async move{
        let model=case.model();let source=case.user().new_session().await?;
        let first=model.expect(RequestMatch::user_text("后台检查并等待我的后续要求"))
            .reply(Reply::tool_call("work-registration-start","work.manage",json!({"action":"start","user_quote":"后台检查并等待我的后续要求","instruction":"registered-work-behavior-marker"})));
        model.expect(RequestMatch::tool_output("work-registration-start")).after(&first).check_tool_output_contains("work-registration-start","accepted").reply(Reply::text("后台已经开始。"));
        let worker=model.expect(RequestMatch::user_text("registered-work-behavior-marker")).reply(Reply::text("held-worker-result").held());
        source.send("后台检查并等待我的后续要求").await?.wait_settled().await?.assert_succeeded()?;worker.observed().await?;
        let rows:Vec<Value>=case.http().await.get(&format!("/api/v1/tasks?source_session_id={}",source.id())).await?.require_success()?.json()?;
        let task=rows.first().context("created work")?;ensure!(task["worker_session_id"]!=source.id());
        let stop=model.expect(RequestMatch::user_text("停止刚才的后台检查"))
            .reply(Reply::tool_call("work-registration-stop","work.manage",json!({"action":"cancel","task_id":task["id"],"user_quote":"停止刚才的后台检查"})));
        model.expect(RequestMatch::tool_output("work-registration-stop")).after(&stop).check_tool_output_contains("work-registration-stop","accepted").reply(Reply::text("已停止。"));
        source.send("停止刚才的后台检查").await?.wait_settled().await?.assert_succeeded()?;worker.disconnected().await?;
        Ok(())
    }).await
}
