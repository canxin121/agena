//! Expected failures must never silently disable process ownership checks.

use super::fixture;
use agena_testkit::{FaultEffect, Reply, RequestMatch, TestResult};
use anyhow::{Context, ensure};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_crash_arm_preserves_monitoring_and_a_fresh_arm_can_recover() -> TestResult {
    fixture()
        .kernel_faults()
        .run(|case| async move {
            let original = case.user().new_session().await?;
            let workspace = original
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("ordinary session must have a project")?;
            let faults = case.faults()?;

            // An outstanding unconsumed injection makes a second arm invalid. This
            // must not disable the product monitor or cancel the current readers.
            faults
                .arm("kernel.before_commit", FaultEffect::Fail)
                .await?;
            ensure!(case.arm_expected_kernel_crash().await.is_err());
            ensure!(!case.server_cancellation_token().await.is_cancelled());
            faults.clear().await?;

            case.arm_expected_kernel_crash().await?;
            ensure!(case.arm_expected_kernel_crash().await.is_err());
            ensure!(
                case.restart().await.is_err(),
                "restart accepted an unreaped crash expectation"
            );
            let result = case
                .client()
                .await
                .create_session(workspace, "crash-arm-regression", None)
                .await;
            ensure!(result.is_err());
            ensure!(faults.wait_hit().await?.hits == 1);
            case.reap_expected_kernel_crash().await?;
            ensure!(case.reap_expected_kernel_crash().await.is_err());
            faults.clear().await?;
            case.restart().await?;
            let restored = case.user().attach_session(original.id()).await?;
            ensure!(restored.snapshot().await?.session.id == original.id());
            let fresh = case
                .client()
                .await
                .create_session(workspace, "monitoring-restored", None)
                .await?;
            ensure!(fresh.title == "monitoring-restored");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_owned_process_kill_cannot_impersonate_an_injected_kernel_crash() -> TestResult {
    fixture()
        .kernel_faults()
        .run(|case| async move {
            let faults = case.faults()?;
            ensure!(
                faults.power_cut().await.is_err(),
                "power cut accepted an unobserved transaction"
            );
            faults.kill().await?;
            let error = case
                .reap_expected_kernel_crash()
                .await
                .expect_err("ordinary kill was mistaken for an armed kernel crash");
            ensure!(error.to_string().contains("must precede"));
            faults.restart().await?;
            case.user().new_session().await?;
            Ok(())
        })
        .await
}

async fn recover_disconnected_model(after_visible_text: bool) -> TestResult {
    fixture()
        .kernel_faults()
        .run(move |case| async move {
            let reply = if after_visible_text {
                Reply::text_chunks(["transport-prefix", "-unreached"]).hold_after_text_chunks(1)
            } else {
                Reply::text("never delivered").held()
            };
            let first = case
                .model()
                .expect(RequestMatch::user_text("cut-one-model-connection"))
                .reply(reply);
            let next = if after_visible_text {
                case.model()
                    .expect(RequestMatch::user_text("fresh-request-after-stream-cut"))
                    .after(&first)
                    .reply(Reply::text("healthy-after-model-cut"))
            } else {
                case.model()
                    .expect(RequestMatch::user_text("cut-one-model-connection"))
                    .after(&first)
                    .reply(Reply::text_chunks(["transport-prefix", "-recovered"]))
            };
            let user = case.user().new_session().await?;
            let run = user.send("cut-one-model-connection").await?;
            first.observed().await?;
            if after_visible_text {
                run.wait_text_contains("transport-prefix").await?;
            }
            case.faults()?.disconnect_model(&first).await?;
            first.disconnected().await?;
            let result = run.wait_settled().await?;
            if after_visible_text {
                // This adapter deliberately disables unsafe replay after output.
                // Observe the real failure, then prove the runtime accepts a
                // fresh request without stale connection or execution state.
                result.assert_failed()?;
                ensure!(result.parts.iter().any(|part| part.kind == "error"
                    && part.content["problem"]["code"] == "provider.request_failed"));
                ensure!(!result.assistant_text.contains("-unreached"));
                let recovered = user
                    .send("fresh-request-after-stream-cut")
                    .await?
                    .wait_settled()
                    .await?;
                recovered.assert_succeeded()?;
                recovered.assert_assistant_text("healthy-after-model-cut")?;
            } else {
                result.assert_succeeded()?;
                ensure!(
                    result.assistant_text == "transport-prefix-recovered",
                    "transport retry duplicated or lost text: {:?}",
                    result.assistant_text
                );
            }
            next.finished().await?;
            ensure!(case.model().requests().len() == 2);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn product_recovers_from_a_model_socket_cut_before_response() -> TestResult {
    recover_disconnected_model(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn product_reports_partial_stream_failure_then_accepts_a_fresh_request() -> TestResult {
    recover_disconnected_model(true).await
}
