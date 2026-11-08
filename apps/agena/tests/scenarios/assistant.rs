//! Personality, durable memory and proactive contact through ordinary chat.
use super::fixture;
use agena_testkit::{ModelHandle, Reply, RequestMatch, SessionOptions, TestResult, UserSession};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::time::Duration;

async fn ask(
    source: &UserSession,
    model: &ModelHandle,
    text: &str,
    id: &str,
    tool: &str,
    input: Value,
    expected: &str,
) -> Result<()> {
    let start = model
        .expect(RequestMatch::user_text(text))
        .reply(Reply::tool_call(id, tool, input));
    model
        .expect(RequestMatch::tool_output(id))
        .after(&start)
        .check_tool_output_contains(id, expected)
        .reply(Reply::text(format!("assistant-action-complete-{id}")));
    source
        .send(text)
        .await?
        .wait_settled()
        .await?
        .assert_succeeded()?;
    Ok(())
}
fn notice(marker: &'static str) -> RequestMatch {
    RequestMatch::new(format!("scheduled contact {marker}"), move |r| {
        let b = r.body.to_string();
        b.contains("Scheduled assistant contact") && b.contains(marker)
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persona_is_applied_across_conversations_persists_and_does_not_cross_workspaces()
-> TestResult {
    fixture().case_timeout(Duration::from_secs(120)).seed_file("alternate-workspace/readme.txt",b"alternate".to_vec()).run(|case|async move{
        let model=case.model();let first=case.user().new_session().await?;
        ask(&first,&model,"以后叫你小鱼，叫我阿林，用中文直接一点","persona-save","companion.manage",json!({"action":"configure","user_quote":"以后叫你小鱼，叫我阿林，用中文直接一点","expected_revision":0,"patch":{"name":"小鱼-persona-marker","address_user":"阿林","tone":"direct","language":"中文"}}),"saved").await?;
        let next=case.user().new_session().await?;
        model.expect(RequestMatch::user_text("早上好-persona-check")).check(|r|{let s=r.body.to_string();ensure!(s.contains("小鱼-persona-marker")&&s.contains("阿林")&&s.contains("direct"));Ok(())}).reply(Reply::text("阿林，早上好。"));
        next.send("早上好-persona-check").await?.wait_settled().await?.assert_succeeded()?;
        let other=case.user().new_session_in_with(case.paths().root.join("alternate-workspace"),SessionOptions{title:"Other workspace".into(),..Default::default()}).await?;
        model.expect(RequestMatch::user_text("另一工作区-persona-check")).check(|r|{ensure!(!r.body.to_string().contains("小鱼-persona-marker"));Ok(())}).reply(Reply::text("使用此工作区自己的设置。"));
        other.send("另一工作区-persona-check").await?.wait_settled().await?.assert_succeeded()?;
        case.crash().await?;case.restart().await?;
        let restored=case.user().attach_session(next.id()).await?;
        let snapshot=restored.client().assistant_snapshot(restored.id()).await?;
        ensure!(snapshot.profile.preferences.name=="小鱼-persona-marker"&&snapshot.profile.revision==1);
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn natural_memory_recall_correction_mute_and_forget_use_real_scoped_storage() -> TestResult {
    fixture().case_timeout(Duration::from_secs(180)).run(|case|async move{
        let model=case.model();let source=case.user().new_session().await?;
        ask(&source,&model,"记住我的报告偏好：先给结论","remember-pref","memory.write",json!({"name":"report-style","content":"user-memory-original-7e81: reports begin with the conclusion.","kind":"preference","description":"报告偏好"}),"user-memory-original-7e81").await?;
        let fresh=case.user().new_session().await?;
        model.expect(RequestMatch::user_text("你好-memory-recall")).check(|r|{ensure!(r.body.to_string().contains("user-memory-original-7e81"),"saved personal preference was not recalled");Ok(())}).reply(Reply::text("会先给结论。"));
        fresh.send("你好-memory-recall").await?.wait_settled().await?.assert_succeeded()?;
        let snapshot=source.client().assistant_snapshot(source.id()).await?;
        let memory=snapshot.memories.iter().find(|m|m.name=="report-style").context("saved memory")?;
        ask(&source,&model,"纠正一下，报告先列行动项","correct-pref","memory.write",json!({"name":"report-style","content":"user-memory-corrected-68ab: reports start with action items.","kind":"preference","expected_sha256":memory.sha256}),"user-memory-corrected-68ab").await?;
        let corrected=case.user().new_session().await?;
        model.expect(RequestMatch::user_text("新的会话-memory-corrected")).check(|r|{let b=r.body.to_string();ensure!(b.contains("user-memory-corrected-68ab")&&!b.contains("user-memory-original-7e81"));Ok(())}).reply(Reply::text("先列行动项。"));
        corrected.send("新的会话-memory-corrected").await?.wait_settled().await?.assert_succeeded()?;
        ask(&source,&model,"先关闭自动记忆","mute-memory","companion.manage",json!({"action":"configure","user_quote":"先关闭自动记忆","expected_revision":0,"patch":{"memory_enabled":false}}),"saved").await?;
        let muted=case.user().new_session().await?;
        model.expect(RequestMatch::user_text("无记忆会话-memory-muted")).check(|r|{ensure!(!r.body.to_string().contains("user-memory-corrected-68ab"));Ok(())}).reply(Reply::text("不使用自动召回。"));
        muted.send("无记忆会话-memory-muted").await?.wait_settled().await?.assert_succeeded()?;
        let bypass=model.expect(RequestMatch::user_text("保持记忆关闭，再检查是否能保存"))
            .reply(Reply::tool_call("raw-memory-mute-guard","memory.write",json!({"name":"must-not-save","content":"raw bypass rejected","kind":"preference"})));
        model.expect(RequestMatch::tool_output("raw-memory-mute-guard")).after(&bypass)
            .check_tool_output_contains("raw-memory-mute-guard","memory is disabled").reply(Reply::text("记忆仍然关闭，没有保存。"));
        muted.send("保持记忆关闭，再检查是否能保存").await?.wait_settled().await?.assert_succeeded()?;
        let snapshot=source.client().assistant_snapshot(source.id()).await?;
        ensure!(snapshot.memories.len()==1,"muting must not pretend to delete saved memory");
        let memory=&snapshot.memories[0];
        ask(&source,&model,"删除刚才保存的报告偏好","forget-pref","memory.delete",json!({"name":memory.name,"expected_sha256":memory.sha256}),"deleted").await?;
        ensure!(source.client().assistant_snapshot(source.id()).await?.memories.is_empty());
        case.crash().await?;case.restart().await?;
        let restored=case.user().attach_session(source.id()).await?;let state=restored.client().assistant_snapshot(restored.id()).await?;
        ensure!(state.memories.is_empty()&&!state.profile.preferences.memory_enabled);
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduled_contact_arrives_in_original_chat_and_cannot_authorize_new_actions() -> TestResult
{
    fixture().case_timeout(Duration::from_secs(100)).run(|case|async move{
        let model=case.model();let source=case.user().new_session().await?;
        let start=model.expect(RequestMatch::user_text("两秒后提醒我喝水"))
            .reply(Reply::tool_call("schedule-water","companion.manage",json!({"action":"remind","user_quote":"两秒后提醒我喝水","reminder":{"text":"water-reminder-proof-392a","after_seconds":2}})));
        model.expect(RequestMatch::tool_output("schedule-water")).after(&start).check_tool_output_contains("schedule-water","scheduled").reply(Reply::text("已经安排，提醒会回到这里。"));
        let recursive=model.expect(notice("water-reminder-proof-392a")).reply(Reply::tool_call("reminder-cannot-mutate","companion.manage",json!({"action":"configure","user_quote":"两秒后提醒我喝水","expected_revision":0,"patch":{"name":"unauthorized-reminder-change"}})));
        let done=model.expect(RequestMatch::tool_output("reminder-cannot-mutate")).after(&recursive).check_tool_output_contains("reminder-cannot-mutate","current direct user").reply(Reply::text("到你安排的时间了，记得喝水。water-reminder-delivered"));
        source.send("两秒后提醒我喝水").await?.wait_settled().await?.assert_succeeded()?;done.finished().await?;
        source.wait_text_contains("water-reminder-delivered").await?;
        let snapshot=source.client().assistant_snapshot(source.id()).await?;
        ensure!(snapshot.profile.preferences.name=="Agena");ensure!(snapshot.reminders.len()==1&&snapshot.reminders[0].delivered==1);
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reminder_survives_restart_and_cancellation_works_from_natural_chat() -> TestResult {
    fixture().case_timeout(Duration::from_secs(120)).run(|case|async move{
        let model=case.model();let source=case.user().new_session().await?;
        ask(&source,&model,"三秒后问我是否完成检查","restart-reminder","companion.manage",json!({"action":"remind","user_quote":"三秒后问我是否完成检查","reminder":{"text":"restart-check-in-e825","kind":"check_in","after_seconds":3}}),"scheduled").await?;
        source.client().set_message_queue_paused(source.id(),true).await?;
        let original=source.client().assistant_snapshot(source.id()).await?.reminders[0].id;
        case.crash().await?;case.restart().await?;
        let restored=case.user().attach_session(source.id()).await?;
        let done=model.expect(notice("restart-check-in-e825")).reply(Reply::text("你之前安排了这次跟进：检查完成了吗？restart-check-in-delivered"));
        restored.client().set_message_queue_paused(restored.id(),false).await?;done.finished().await?;
        restored.wait_text_contains("restart-check-in-delivered").await?;
        let snapshot=restored.client().assistant_snapshot(restored.id()).await?;
        ensure!(snapshot.reminders[0].id==original&&snapshot.reminders[0].delivered==1);
        ask(&restored,&model,"一分钟后提醒我核对报告","cancel-create","companion.manage",json!({"action":"remind","user_quote":"一分钟后提醒我核对报告","reminder":{"text":"cancel-this-reminder","after_seconds":60}}),"scheduled").await?;
        let snapshot=restored.client().assistant_snapshot(restored.id()).await?;let pending=snapshot.reminders.iter().find(|r|r.spec.text=="cancel-this-reminder").unwrap();
        ask(&restored,&model,"取消刚才核对报告的提醒","cancel-reminder","companion.manage",json!({"action":"cancel_reminder","user_quote":"取消刚才核对报告的提醒","reminder_id":pending.id,"expected_revision":pending.revision}),"cancelled").await?;
        let snapshot=restored.client().assistant_snapshot(restored.id()).await?;ensure!(snapshot.reminders.iter().any(|r|r.id==pending.id&&r.state==agena_api::assistant::ReminderState::Cancelled));
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assistant_api_preserves_operator_auth_and_stale_memory_edits_are_rejected() -> TestResult {
    use reqwest::Method;
    fixture().password("companion-fixture-password").run(|case|async move{
        let source=case.user().new_session().await?;let http=case.http().await;
        ensure!(http.anonymous().get(&format!("/api/v1/sessions/{}/assistant",source.id())).await?.status.as_u16()==401);
        let name = "偏好".repeat(32);
        let body=json!({"name":name,"body":"original","description":"","kind":"preference","expected_sha256":null});
        let path=format!("/api/v1/sessions/{}/assistant/memory",source.id());
        let saved:Value=http.send_json(Method::POST,&path,&body).await?.require_success()?.json()?;
        let stale=http.send_json(Method::POST,&path,&json!({"name":name,"kind":"preference","body":"must-not-overwrite","expected_sha256":"wrong-revision"})).await?;
        ensure!(stale.status.as_u16()==409);
        let memory=source.client().assistant_snapshot(source.id()).await?.memories.pop().context("memory")?;
        ensure!(memory.body.trim()=="original"&&memory.sha256==saved["sha256"]);
        let updated: Value = http.send_json(Method::POST, &path, &json!({
            "name": name, "kind": "preference", "body": "updated", "expected_sha256": memory.sha256
        })).await?.require_success()?.json()?;
        ensure!(updated["name"] == name && updated["body"].as_str().is_some_and(|body| body.trim() == "updated"));
        http.send_json(Method::POST, &path, &json!({
            "name": name, "expected_sha256": updated["sha256"]
        })).await?.require_success()?;
        ensure!(source.client().assistant_snapshot(source.id()).await?.memories.is_empty());
        Ok(())
    }).await
}
