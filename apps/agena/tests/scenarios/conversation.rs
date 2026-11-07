use super::fixture;
use agena_testkit::{Reply, RequestMatch, TestResult, Usage};
use anyhow::ensure;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_uses_real_provider_and_preserves_conversation_context() -> TestResult {
    fixture()
        .run(|case| async move {
            let model = case.model();
            let first = model
                .expect(RequestMatch::user_text("first-user-marker").streaming(true))
                .named("initial user request")
                .check(|request| {
                    ensure!(
                        request.model() == Some("fake-model"),
                        "wrong configured model"
                    );
                    ensure!(
                        request.path == "/v1/responses",
                        "provider bypassed Responses endpoint"
                    );
                    Ok(())
                })
                .reply(Reply::text("first-local-answer").usage(Usage::tokens(21, 5)));
            let second = model
                .expect(RequestMatch::user_text("second-user-marker"))
                .after(&first)
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(
                        body.contains("first-user-marker"),
                        "history lost first user message"
                    );
                    ensure!(
                        body.contains("first-local-answer"),
                        "history lost real assistant output"
                    );
                    Ok(())
                })
                .reply(Reply::text("second-local-answer"));

            let user = case.user().new_session().await?;
            let first_run = user.send("first-user-marker").await?;
            let first_result = first_run.wait_settled().await?;
            first_result.assert_succeeded()?;
            first_result.assert_assistant_text("first-local-answer")?;
            let second_run = user.send("second-user-marker").await?;
            let result = second_run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("second-local-answer")?;
            ensure!(first_run.receipt().execution_id != second_run.receipt().execution_id);
            ensure!(first_run.receipt().message_id != second_run.receipt().message_id);
            second.finished().await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_partial_text_can_be_cancelled_at_the_real_http_boundary() -> TestResult {
    fixture()
        .run(|case| async move {
            let reply = case
                .model()
                .expect(RequestMatch::user_text("cancel-stream-marker"))
                .reply(
                    Reply::text_chunks(["visible-part可取消", "must-not-appear"])
                        .with_thinking_chunks(["reasoning before visible text"])
                        .fragment_bytes(3)
                        .hold_after_text_chunks(1),
                );
            let user = case.user().new_session().await?;
            let run = user.send("cancel-stream-marker").await?;
            reply.observed().await?;
            run.wait_text_contains("visible-part可取消").await?;
            run.cancel().await?;
            let result = run.wait_settled().await?;
            result.assert_cancelled()?;
            ensure!(
                !result.assistant_text.contains("must-not-appear"),
                "held bytes leaked"
            );
            reply.disconnected().await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_text_gate_releases_a_complete_fragmented_response() -> TestResult {
    fixture()
        .run(|case| async move {
            let reply = case
                .model()
                .expect(RequestMatch::user_text("release-text-gate-marker"))
                .reply(
                    Reply::text_chunks(["可见开头", "，随后完成"])
                        .with_thinking_chunks(["first thought", "second thought"])
                        .hold_after_text_chunks(1)
                        .fragment_bytes(2),
                );
            let user = case.user().new_session().await?;
            let run = user.send("release-text-gate-marker").await?;
            run.wait_text_contains("可见开头").await?;
            ensure!(
                !run.status().await?.phase.is_terminal(),
                "text gate failed to keep the real execution in progress"
            );
            reply.release();
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("可见开头，随后完成")?;
            reply.finished().await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_sessions_match_by_content_instead_of_arrival_order() -> TestResult {
    fixture()
        .run(|case| async move {
            let model = case.model();
            let left = model
                .expect(RequestMatch::user_text("parallel-left-marker"))
                .reply(Reply::text("left-answer").held());
            let right = model
                .expect(RequestMatch::user_text("parallel-right-marker"))
                .reply(Reply::text("right-answer"));
            let left_user = case.user().new_session().await?;
            let right_user = case.user().new_session().await?;
            let (left_run, right_run) = tokio::try_join!(
                left_user.send("parallel-left-marker"),
                right_user.send("parallel-right-marker")
            )?;
            left.observed().await?;
            let right_result = right_run.wait_settled().await?;
            right_result.assert_succeeded()?;
            right_result.assert_assistant_text("right-answer")?;
            ensure!(!left_run.status().await?.phase.is_terminal());
            left.release();
            let left_result = left_run.wait_settled().await?;
            left_result.assert_succeeded()?;
            left_result.assert_assistant_text("left-answer")?;
            right.finished().await?;
            Ok(())
        })
        .await
}
