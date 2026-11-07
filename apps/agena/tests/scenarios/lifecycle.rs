mod ownership;

use super::fixture;
use agena_api::resource::RunOptions;
use agena_domain::session_kernel::{ExecutionAttemptSettlement, ExecutionPhase};
use agena_testkit::{Reply, RequestMatch, TestResult};
use anyhow::ensure;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_restart_preserves_history_and_uses_a_new_server_identity() -> TestResult {
    fixture()
        .run(|case| async move {
            let model = case.model();
            let first = model
                .expect(RequestMatch::user_text("before-restart-marker"))
                .reply(Reply::text("persisted-answer"));
            model
                .expect(RequestMatch::user_text("after-restart-marker"))
                .after(&first)
                .check(|request| {
                    ensure!(
                        request.body.to_string().contains("persisted-answer"),
                        "history did not survive process restart"
                    );
                    Ok(())
                })
                .reply(Reply::text("restored-answer"));
            let user = case.user().new_session().await?;
            let session_id = user.id();
            let result = user
                .send("before-restart-marker")
                .await?
                .wait_settled()
                .await?;
            result.assert_succeeded()?;
            let before = case.client().await.server_identity().await?;
            case.restart().await?;
            let after = case.client().await.server_identity().await?;
            ensure!(before.instance_id == after.instance_id && before.id != after.id);
            let user = case.user().attach_session(session_id).await?;
            ensure!(user.assistant_text().await?.contains("persisted-answer"));
            let result = user
                .send("after-restart-marker")
                .await?
                .wait_settled()
                .await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("restored-answer")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_recovery_is_not_reported_as_successful_settlement() -> TestResult {
    fixture()
        .run(|case| async move {
            let step = case
                .model()
                .expect(RequestMatch::user_text("crash-recovery-marker"))
                .reply(
                    Reply::text_chunks(["saved-prefix", "unreleased-suffix"]).hold_after_chunks(1),
                );
            let user = case.user().new_session().await?;
            let run = user.send("crash-recovery-marker").await?;
            run.wait_text_contains("saved-prefix").await?;
            let receipt = *run.receipt();
            let session_id = user.id();
            case.crash().await?;
            step.disconnected().await?;
            case.restart().await?;
            let user = case.user().attach_session(session_id).await?;
            let recovered = user.attach_run(receipt);
            let terminal = recovered.wait_terminal().await?;
            ensure!(
                terminal.status.phase == ExecutionPhase::Interrupted,
                "{:?}",
                terminal.status
            );
            ensure!(
                terminal.status.settlement == ExecutionAttemptSettlement::Recovered,
                "{:?}",
                terminal.status
            );
            let error = recovered
                .wait_settled()
                .await
                .expect_err("recovered attempt cannot prove old in-memory hooks settled");
            ensure!(
                format!("{error:#}").contains("cannot normally settle: Recovered"),
                "{error:#}"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn continuation_has_a_new_attempt_and_cannot_match_old_terminal_output() -> TestResult {
    fixture()
        .run(|case| async move {
            let model = case.model();
            let first = model
                .expect(RequestMatch::user_text("continue-marker"))
                .reply(
                    Reply::text_chunks(["old-attempt-prefix", "held-old-suffix"])
                        .hold_after_chunks(1),
                );
            let second = model
                .expect(RequestMatch::user_text("continue-marker"))
                .after(&first)
                .reply(Reply::text("new-attempt-answer").held());
            let user = case.user().new_session().await?;
            let original = user.send("continue-marker").await?;
            original.wait_text_contains("old-attempt-prefix").await?;
            original.cancel().await?;
            original.wait_settled().await?.assert_cancelled()?;
            first.disconnected().await?;
            let continued = user.continue_run(RunOptions::default()).await?;
            second.observed().await?;
            ensure!(continued.receipt().execution_id == original.receipt().execution_id);
            ensure!(continued.receipt().attempt_generation > original.receipt().attempt_generation);
            ensure!(
                !continued.status().await?.phase.is_terminal(),
                "new attempt mistaken for old terminal"
            );
            // A stale handle must never cancel this newer attempt.
            original.cancel().await?;
            ensure!(
                !continued.status().await?.phase.is_terminal(),
                "stale cancellation touched a newer attempt"
            );
            second.release();
            let result = continued.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("new-attempt-answer")?;
            Ok(())
        })
        .await
}

/// Returning from a case without waiting for its accepted model response must
/// not make fixture teardown look like a successful completed model turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abandoned_held_model_reply_is_a_case_cleanup_failure() -> TestResult {
    let outcome = fixture()
        .run(|case| async move {
            let step = case
                .model()
                .expect(RequestMatch::user_text("abandoned-model-reply"))
                .reply(Reply::text("never-finished").held());
            let user = case.user().new_session().await?;
            let _run = user.send("abandoned-model-reply").await?;
            step.observed().await?;
            // Intentionally no release, wait_settled or explicit disconnect.
            Ok(())
        })
        .await;
    let error = outcome.expect_err("a half-finished model reply was silently accepted");
    ensure!(
        error.to_string().contains("model responses still active")
            || error
                .to_string()
                .contains("model replies disconnected without"),
        "case failed for an unrelated reason instead of detecting the abandoned response: {error:#}"
    );
    Ok(())
}
