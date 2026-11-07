use super::fixture;
use agena_testkit::{Reply, RequestMatch, TestResult};
use anyhow::ensure;
use serde_json::json;
use std::collections::BTreeMap;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_input_reply_returns_to_the_actual_model_tool_call() -> TestResult {
    fixture()
        .run(|case| async move {
            let model = case.model();
            let ask = model
                .expect(RequestMatch::user_text("ask-input-marker"))
                .reply(Reply::tool_call(
                    "call_ask_input",
                    "interaction.ask",
                    json!({
                        "questions": [{
                            "header": "Color",
                            "question": "Which color should be used?",
                            "options": [
                                {"label": "Blue", "description": "Use blue."},
                                {"label": "Green", "description": "Use green."}
                            ],
                            "multiple": false
                        }]
                    }),
                ));
            model
                .expect(RequestMatch::tool_output("call_ask_input"))
                .after(&ask)
                .check_tool_output_contains("call_ask_input", "Green")
                .reply(Reply::text("green-selected"));
            let user = case.user().new_session().await?;
            let run = user.send("ask-input-marker").await?;
            let question = run.wait_user_input().await?;
            question.mark_presented().await?;
            question
                .answer(BTreeMap::from([("0".to_owned(), vec!["Green".to_owned()])]))
                .await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("green-selected")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_permission_model_is_also_scripted_over_real_http() -> TestResult {
    fixture()
        .run(|case| async move {
            let target = case.workspace().join("auto-approved.txt");
            let target_rule = target.to_string_lossy().into_owned();
            let model = case.model();
            let call = model
                .expect(RequestMatch::user_text("auto-approval-marker").streaming(true))
                .reply(Reply::tool_call(
                    "call_auto_write",
                    "fs.write",
                    json!({"path": "auto-approved.txt", "content": "auto-approved-content"}),
                ));
            let review_target = target_rule.clone();
            let approval = model
                .expect(
                    RequestMatch::any()
                        .declares_function("approve_action")
                        .streaming(false),
                )
                .after(&call)
                .check(move |request| {
                    ensure!(
                        request.user_text().contains(&review_target),
                        "approval request did not identify the case-owned file: {}",
                        request.body
                    );
                    Ok(())
                })
                .reply(Reply::approve("This test explicitly requested the write.").held());
            model
                .expect(RequestMatch::tool_output("call_auto_write"))
                .after(&approval)
                .reply(Reply::text("automatic-approval-complete"));
            let user = case.user().new_session().await?;
            // The fixture is under the OS temporary directory. Its shipped
            // <tmp>/** allow rule precedes workspace defaults, so select this
            // exact file for review through the normal user permission API.
            user.client().set_session_permission(user.id(), serde_json::from_value(json!({
                "path": {
                    "workspace": {"read": "allow", "write": "auto"},
                    "rules": {(target_rule): {"read": "allow", "write": "auto"}}
                },
                "approval_model": {
                    "provider": "testkit", "adapter": "openai_responses", "model": "fake-model"
                }
            }))?).await?;
            let run = user.send("auto-approval-marker").await?;
            approval.observed().await?;
            ensure!(
                !target.exists(),
                "write occurred before the approval model decided"
            );
            approval.release();
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("automatic-approval-complete")?;
            ensure!(tokio::fs::read_to_string(target).await? == "auto-approved-content");
            approval.finished().await?;
            Ok(())
        })
        .await
}
