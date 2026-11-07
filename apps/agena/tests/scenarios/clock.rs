//! Virtual time drives real product Cron state and admission races. Product
//! HTTP and test cleanup retain their real-time watchdogs.

use std::time::Duration;

use agena_testkit::{TestCase, TestResult, UserSession};
use anyhow::{Context, ensure};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};

use super::fixture;

async fn cron_user(case: &TestCase) -> TestResult<UserSession> {
    let user = case.user().new_session().await?;
    user.client()
        .set_session_permission(
            user.id(),
            serde_json::from_value(json!({
                "tools": {"default": "allow"}
            }))?,
        )
        .await?;
    Ok(user)
}

async fn schedule(user: &UserSession, prompt: &str) -> TestResult<String> {
    let created = user
        .call_tool(
            "cron.create",
            json!({
                "expression": "0 0 6 * * *",
                "timezone": "UTC",
                "prompt": prompt,
                "max_age_days": 7
            }),
        )
        .await?;
    created.assert_succeeded()?;
    ensure!(
        created.data()?["next_fire_at"]
            .as_str()
            .is_some_and(|at| at.starts_with("2026-01-01T06:00:00")),
        "Cron did not use the frozen fixture time"
    );
    Ok(created.data()?["id"]
        .as_str()
        .context("Cron job id")?
        .to_owned())
}

async fn history(user: &UserSession, id: &str) -> TestResult<Vec<Value>> {
    let result = user.call_tool("cron.history", json!({"id": id})).await?;
    result.assert_succeeded()?;
    Ok(result.data()?["entries"]
        .as_array()
        .context("Cron history entries")?
        .clone())
}

async fn wait_submitted(user: &UserSession, id: &str) -> TestResult<Vec<Value>> {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let entries = history(user, id).await?;
            if !entries.is_empty() {
                ensure!(
                    entries.len() == 1 && entries[0]["status"] == "submitted",
                    "unexpected durable Cron history: {entries:?}"
                );
                return Ok(entries);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("virtual deadline did not produce durable Cron delivery")?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_clock_advances_six_hours_and_orders_delete_before_same_deadline_admission()
-> TestResult {
    let baseline = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap();
    fixture()
        .test_clock(baseline)
        .expect_plugin("agena.cron")
        .run(|case| async move {
            let clock = case.clock()?;
            let boundary = clock.pause_scheduler().await?;
            ensure!(boundary.scheduler_paused && boundary.active_deliveries == 0);
            let user = cron_user(&case).await?;
            let cancelled = schedule(&user, "delete-before-due-admission").await?;
            let retained = schedule(&user, "deliver-at-exact-six-hour-deadline").await?;

            // Let the real scheduler examine the instant just before both jobs are
            // due. Its acknowledged pass must not deliver either job early.
            clock
                .advance(chrono::Duration::hours(6) - chrono::Duration::milliseconds(1))
                .await?;
            clock.resume_scheduler().await?;
            ensure!(history(&user, &cancelled).await?.is_empty());
            ensure!(history(&user, &retained).await?.is_empty());

            // At the exact shared deadline, control which competing action wins:
            // delete commits while admission is paused, then release the scheduler.
            let boundary = clock.pause_scheduler().await?;
            ensure!(boundary.active_deliveries == 0);
            clock.advance(chrono::Duration::milliseconds(1)).await?;
            let boundary = clock.wait_observed().await?;
            ensure!(
                boundary.scheduler_paused
                    && boundary.now_ms
                        == (baseline + chrono::Duration::hours(6)).timestamp_millis()
            );
            ensure!(history(&user, &retained).await?.is_empty());
            user.call_tool("cron.delete", json!({"id": cancelled}))
                .await?
                .assert_succeeded()?;
            clock.resume_scheduler().await?;
            let entries = wait_submitted(&user, &retained).await?;
            ensure!(
                entries[0]["scheduled_for"]
                    .as_str()
                    .is_some_and(|at| at.starts_with("2026-01-01T06:00:00")),
                "delivery did not retain the exact occurrence: {entries:?}"
            );
            ensure!(history(&user, &cancelled).await?.is_empty());

            // More real scheduler passes do not advance frozen time or replay an
            // already acknowledged occurrence.
            clock.wait_observed().await?;
            ensure!(clock.now().await? == baseline + chrono::Duration::hours(6));
            ensure!(history(&user, &retained).await?.len() == 1);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_clock_keeps_frozen_time_and_admission_gate_across_product_restart() -> TestResult {
    let baseline = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap();
    fixture()
        .test_clock(baseline)
        .expect_plugin("agena.cron")
        .run(|case| async move {
            let clock = case.clock()?;
            let previous = clock.pause_scheduler().await?;
            let user = cron_user(&case).await?;
            let session_id = user.id();
            let job = schedule(&user, "deliver-only-after-restarted-gate-release").await?;
            clock.advance(chrono::Duration::hours(6)).await?;
            clock.wait_observed().await?;
            ensure!(history(&user, &job).await?.is_empty());

            case.restart().await?;
            // Fresh revision is essential: the old process's acknowledgement must
            // not satisfy a restart barrier even though wall time is unchanged.
            let current = clock.wait_observed().await?;
            ensure!(current.process_id != previous.process_id);
            ensure!(current.scheduler_paused && current.active_deliveries == 0);
            ensure!(clock.now().await? == baseline + chrono::Duration::hours(6));
            ensure!(clock.elapsed().await? == Duration::from_secs(6 * 3600));
            let restored = case.user().attach_session(session_id).await?;
            ensure!(history(&restored, &job).await?.is_empty());
            clock.resume_scheduler().await?;
            wait_submitted(&restored, &job).await?;
            Ok(())
        })
        .await
}
