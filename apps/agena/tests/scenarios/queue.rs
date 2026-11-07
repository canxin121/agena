//! Durable queue behavior through the same HTTP commands used by real clients.
use super::fixture;
use agena_domain::session_kernel::{ExecutionAttemptSettlement, ExecutionPhase};
use agena_testkit::{Reply, RequestMatch, TestResult};
use anyhow::ensure;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_without_queued_input_allows_the_next_send_to_start() -> TestResult {
    fixture()
        .run(|case| async move {
            for exact in [false, true] {
                let marker = format!("stop-without-queue-{exact}-marker");
                let next_marker = format!("next-after-stop-{exact}-marker");
                let first = case
                    .model()
                    .expect(RequestMatch::user_text(marker.clone()))
                    .reply(
                        Reply::text_chunks(["visible-before-stop", "unreleased-suffix"])
                            .hold_after_chunks(1),
                    );
                case.model()
                    .expect(RequestMatch::user_text(next_marker.clone()))
                    .reply(Reply::text("next-send-reached-the-provider"));
                let user = case.user().new_session().await?;
                let run = user.send(marker).await?;
                run.wait_text_contains("visible-before-stop").await?;
                let execution = if exact {
                    Some(run.receipt().execution_id.into())
                } else {
                    None
                };
                user.client().cancel_run(user.id(), execution).await?;
                run.wait_settled().await?.assert_cancelled()?;
                first.disconnected().await?;
                ensure!(
                    !user.snapshot().await?.queue_paused,
                    "stopping an empty queue paused future sends"
                );
                user.send(next_marker)
                    .await?
                    .wait_settled()
                    .await?
                    .assert_assistant_text("next-send-reached-the-provider")?;
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_with_queued_input_keeps_it_paused_and_intact() -> TestResult {
    fixture()
        .run(|case| async move {
            let first = case
                .model()
                .expect(RequestMatch::user_text("queue-active-marker"))
                .reply(
                    Reply::text_chunks(["active-before-stop", "unreleased-suffix"])
                        .hold_after_chunks(1),
                );
            let next = case
                .model()
                .expect(RequestMatch::user_text("queue-second-marker"))
                .reply(Reply::text("queue-second-answer").held());
            case.model()
                .expect(RequestMatch::user_text("queue-third-marker"))
                .after(&next)
                .check(|request| {
                    ensure!(
                        request.body.to_string().contains("queue-second-answer"),
                        "FIFO successor did not inherit the completed predecessor"
                    );
                    Ok(())
                })
                .reply(Reply::text("queue-third-answer"));
            let user = case.user().new_session().await?;
            let active = user.send("queue-active-marker").await?;
            active.wait_text_contains("active-before-stop").await?;
            let second = user.send("queue-second-marker").await?;
            let third = user.send("queue-third-marker").await?;
            ensure!(second.status().await?.phase == ExecutionPhase::Queued);
            ensure!(third.status().await?.phase == ExecutionPhase::Queued);
            user.client().cancel_run(user.id(), None).await?;
            active.wait_settled().await?.assert_cancelled()?;
            first.disconnected().await?;
            ensure!(
                user.snapshot().await?.queue_paused,
                "queued work must remain explicitly paused after stop"
            );
            for queued in [&second, &third] {
                let status = queued.status().await?;
                ensure!(status.phase == ExecutionPhase::Queued, "{status:?}");
                ensure!(
                    status.settlement == ExecutionAttemptSettlement::Pending,
                    "{status:?}"
                );
            }
            ensure!(
                case.model().requests().len() == 1,
                "a paused successor reached the provider"
            );
            user.client()
                .set_message_queue_paused(user.id(), false)
                .await?;
            next.observed().await?;
            ensure!(third.status().await?.phase == ExecutionPhase::Queued);
            next.release();
            second
                .wait_settled()
                .await?
                .assert_assistant_text("queue-second-answer")?;
            third
                .wait_settled()
                .await?
                .assert_assistant_text("queue-third-answer")?;
            ensure!(
                case.model().requests().len() == 3,
                "a queued message was lost or duplicated"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_an_empty_session_preserves_a_manual_pause() -> TestResult {
    fixture()
        .run(|case| async move {
            let response = case
                .model()
                .expect(RequestMatch::user_text("manual-pause-marker"))
                .reply(Reply::text("resumed-manual-pause-answer").held());
            let user = case.user().new_session().await?;
            user.client()
                .set_message_queue_paused(user.id(), true)
                .await?;
            user.client().cancel_run(user.id(), None).await?;
            ensure!(user.snapshot().await?.queue_paused);
            let run = user.send("manual-pause-marker").await?;
            let queued = run.status().await?;
            ensure!(queued.phase == ExecutionPhase::Queued, "{queued:?}");
            ensure!(
                queued.settlement == ExecutionAttemptSettlement::Pending,
                "{queued:?}"
            );
            ensure!(
                case.model().requests().is_empty(),
                "cancel silently removed the manual pause"
            );
            user.client()
                .set_message_queue_paused(user.id(), false)
                .await?;
            response.observed().await?;
            response.release();
            run.wait_settled()
                .await?
                .assert_assistant_text("resumed-manual-pause-answer")?;
            Ok(())
        })
        .await
}
