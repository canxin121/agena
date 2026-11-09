//! Opt-in virtual scheduler clock and exact SQLite transaction fault gates.
//! Product HTTP, kernel transactions and scheduler deliveries remain real.

use std::time::Duration;

use super::fixture;
use agena_api::queries::{ListSessionsParams, Query, QueryResult};
use agena_testkit::{FaultEffect, FaultSyscall, TestCase, TestResult};
use anyhow::{Context, ensure};
use chrono::{TimeZone, Utc};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduler_virtual_clock_fires_a_real_cron_occurrence_without_waiting_for_wall_time()
-> TestResult {
    // A real fixed historical instant, chosen so next scheduled second is exact.
    let baseline = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap();
    fixture()
        .scheduler_clock(baseline)
        .expect_plugin("agena.cron")
        .run(|case| async move {
            let clock = case.clock()?;
            ensure!(clock.now().await? == baseline);
            let user = case.user().new_session().await?;
            user.client()
                .set_session_permission(
                    user.id(),
                    serde_json::from_value(json!({
                        "tools": {"default": "allow"}
                    }))?,
                )
                .await?;

            let created = user
                .call_tool(
                    "cron.create",
                    json!({
                        "expression": "15 * * * * *",
                        "timezone": "UTC",
                        "prompt": "scheduler-virtual-clock-delivery",
                        "max_age_days": 7
                    }),
                )
                .await?;
            created.assert_succeeded()?;
            let job_id = created.data()?["id"]
                .as_str()
                .context("scheduled job id")?
                .to_owned();
            let deadline = created.data()?["next_fire_at"]
                .as_str()
                .context("next scheduler fire timestamp")?;
            ensure!(
                deadline.starts_with("2026-01-01T00:00:15"),
                "unexpected virtual next fire: {deadline}"
            );

            // Time jumps in constant wall time; no waiting for 15 real seconds.
            clock.advance(chrono::Duration::seconds(14)).await?;
            ensure!(clock.now().await? == baseline + chrono::Duration::seconds(14));
            let previous = user
                .call_tool("cron.history", json!({"id": job_id}))
                .await?;
            previous.assert_succeeded()?;
            ensure!(
                previous.data()?["entries"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
            );

            clock.advance(chrono::Duration::seconds(1)).await?;
            // Scheduler delivery is a durable enqueue fact, not a promise
            // that the session has already started a new model turn. Observe
            // the real scheduling ledger rather than expecting an unsolicited
            // model request whose dispatch is controlled by session activity.
            let entries = tokio::time::timeout(Duration::from_secs(8), async {
                loop {
                    let history = user
                        .call_tool("cron.history", json!({"id": job_id}))
                        .await?;
                    history.assert_succeeded()?;
                    let entries = history.data()?["entries"]
                        .as_array()
                        .context("delivered cron history")?
                        .clone();
                    if !entries.is_empty() {
                        break Ok::<_, anyhow::Error>(entries);
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
            .context("virtual UTC clock did not produce durable scheduling history")??;
            ensure!(
                entries.len() == 1 && entries[0]["status"] == "submitted",
                "virtual clock did not durably submit exactly one real job: {entries:?}"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_kernel_commit_rolls_back_and_allows_a_fresh_successful_write() -> TestResult {
    fixture()
        .kernel_faults()
        .run(|case| async move {
            let original = case.user().new_session().await?;
            let workspace_id = original
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("ordinary session must have a project")?;
            let control = case.faults()?;
            control
                .arm("kernel.before_commit", FaultEffect::Fail)
                .await?;
            let failed = case
                .client()
                .await
                .create_session(workspace_id, "should-never-commit", None)
                .await;
            ensure!(
                failed.is_err(),
                "faulted production transaction unexpectedly succeeded"
            );
            ensure!(control.wait_hit().await?.hits == 1);
            control.clear().await?;
            assert_session_title_count(&case, workspace_id, "should-never-commit", 0).await?;

            // The SQLite writer must be healthy after rollback. A successful new
            // session is proof this was not just an HTTP rejection before the DB.
            let created = case
                .client()
                .await
                .create_session(workspace_id, "after-rollback", None)
                .await?;
            ensure!(created.title == "after-rollback");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn held_kernel_commit_remains_invisible_until_exact_release() -> TestResult {
    fixture()
        .kernel_faults()
        .run(|case| async move {
            let anchor = case.user().new_session().await?;
            let workspace_id = anchor
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("ordinary session must have a project")?;
            let control = case.faults()?;
            control
                .arm("kernel.before_commit", FaultEffect::Hold)
                .await?;

            let client = case.client().await;
            let creating = tokio::spawn(async move {
                client
                    .create_session(workspace_id, "held-commit", None)
                    .await
            });
            control.wait_hit().await?;
            ensure!(
                !creating.is_finished(),
                "SQLite write returned before the fault gate was released"
            );
            assert_session_title_count(&case, workspace_id, "held-commit", 0).await?;
            control.release().await?;
            let created = tokio::time::timeout(Duration::from_secs(8), creating).await??;
            ensure!(created?.title == "held-commit");
            assert_session_title_count(&case, workspace_id, "held-commit", 1).await?;
            control.clear().await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_during_uncommitted_kernel_write_recovers_without_phantom_session() -> TestResult {
    fixture()
        .kernel_faults()
        .run(|case| async move {
            let existing = case.user().new_session().await?;
            let workspace_id = existing
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("ordinary session must have a project")?;
            let original_id = existing.id();
            let control = case.faults()?;
            control
                .arm("kernel.before_commit", FaultEffect::Hold)
                .await?;
            let client = case.client().await;
            let creating = tokio::spawn(async move {
                client
                    .create_session(workspace_id, "not-durable", None)
                    .await
            });
            control.wait_hit().await?;
            ensure!(!creating.is_finished());

            // This kills the *actual* product while SQLite owns an uncommitted
            // write transaction. Neither a fake database nor a synthetic answer
            // can satisfy the recovery assertions.
            control.power_cut().await?;
            let interrupted = tokio::time::timeout(Duration::from_secs(4), creating).await??;
            ensure!(
                interrupted.is_err(),
                "power-cut transaction returned a successful receipt"
            );
            control.clear().await?;
            control.restart().await?;
            let restored = case.user().attach_session(original_id).await?;
            ensure!(restored.snapshot().await?.session.id == original_id);
            assert_session_title_count(&case, workspace_id, "not-durable", 0).await?;
            let after = case
                .client()
                .await
                .create_session(workspace_id, "durable-after-crash", None)
                .await?;
            ensure!(after.title == "durable-after-crash");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn injected_kernel_crash_exits_the_real_product_and_restart_recovers() -> TestResult {
    fixture()
        .kernel_faults()
        .run(|case| async move {
            let original = case.user().new_session().await?;
            let workspace_id = original
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("ordinary session must have a project")?;
            let original_id = original.id();
            let fault = case.faults()?;

            // Register this as an intentional process crash before launching the
            // failing request. There is no race against the unexpected-exit monitor.
            case.arm_expected_kernel_crash().await?;
            let outcome = case
                .client()
                .await
                .create_session(workspace_id, "transaction-interrupted-by-abort", None)
                .await;
            ensure!(
                outcome.is_err(),
                "kernel.before_commit abort returned a successful HTTP receipt"
            );
            ensure!(
                fault.wait_hit().await?.hits == 1,
                "SQLite kernel was not the crash source"
            );
            let exit = case.reap_expected_kernel_crash().await?;
            ensure!(!exit.status.is_empty(), "crash exit was not reaped");

            fault.clear().await?;
            case.restart().await?;
            let restored = case.user().attach_session(original_id).await?;
            ensure!(restored.snapshot().await?.session.id == original_id);
            assert_session_title_count(&case, workspace_id, "transaction-interrupted-by-abort", 0)
                .await?;
            let fresh = case
                .client()
                .await
                .create_session(workspace_id, "healthy-after-abort", None)
                .await?;
            ensure!(fresh.title == "healthy-after-abort");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hold_release_and_clear_allow_immediate_rearm_without_losing_the_new_fault() -> TestResult {
    fixture()
        .kernel_faults()
        .run(|case| async move {
            let anchor = case.user().new_session().await?;
            let workspace_id = anchor
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("ordinary session must have a project")?;
            let control = case.faults()?;
            control
                .arm("kernel.before_commit", FaultEffect::Hold)
                .await?;
            for (index, title) in ["release-then-rearm", "clear-then-rearm"]
                .into_iter()
                .enumerate()
            {
                let client = case.client().await;
                let creating =
                    tokio::spawn(
                        async move { client.create_session(workspace_id, title, None).await },
                    );
                control.wait_hit().await?;
                ensure!(!creating.is_finished());
                if index == 0 {
                    control.release().await?;
                } else {
                    control.clear().await?;
                }
                control
                    .arm("kernel.before_commit", FaultEffect::Hold)
                    .await?;
                let new_arm = control.state().await?.arm_id;
                let created = tokio::time::timeout(Duration::from_secs(8), creating).await???;
                ensure!(created.title == title);
                let state = control.state().await?;
                ensure!(
                    state.arm_id == new_arm && state.remaining == 1 && state.hits == 0,
                    "old held transaction consumed or replaced the next fault arm"
                );
                assert_session_title_count(&case, workspace_id, title, 1).await?;
            }
            control.clear().await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_disk_full_fails_the_real_write_without_a_phantom_after_restart() -> TestResult {
    recover_from_sqlite_fault(FaultSyscall::Write, FaultEffect::DiskFull, 13, true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_write_io_error_fails_the_real_write_and_restart_recovers() -> TestResult {
    recover_from_sqlite_fault(FaultSyscall::Write, FaultEffect::IoError, 778, true).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_sync_io_error_never_returns_a_successful_receipt_and_restart_recovers() -> TestResult
{
    recover_from_sqlite_fault(FaultSyscall::Sync, FaultEffect::IoError, 1034, false).await
}

async fn recover_from_sqlite_fault(
    operation: FaultSyscall,
    effect: FaultEffect,
    expected_code: i32,
    expect_rollback: bool,
) -> TestResult {
    fixture()
        .kernel_faults()
        .run(move |case| async move {
            let original = case.user().new_session().await?;
            let workspace_id = original
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("ordinary session must have a project")?;
            let original_id = original.id();
            let fault = case.faults()?;
            if effect == FaultEffect::DiskFull {
                fault.disk_full().await?;
            } else {
                fault.fail_syscall(operation).await?;
            }
            let result = case
                .client()
                .await
                .create_session(workspace_id, "interrupted-by-file-io", None)
                .await;
            ensure!(
                result.is_err(),
                "injected SQLite I/O error returned a successful receipt"
            );
            let observed = fault.wait_hit().await?;
            ensure!(observed.point == operation.point());
            ensure!(observed.sqlite_code == Some(expected_code));
            ensure!(
                observed
                    .file
                    .as_deref()
                    .is_some_and(|file| !file.is_empty()),
                "fault did not record the actual SQLite file operation"
            );
            fault.clear().await?;

            // A storage commit error closes the kernel's cache because the
            // durable outcome can be uncertain. Its shutdown correctly reports
            // that unhealthy state, so explicitly reboot the owned product and
            // recover from disk before permitting new writes.
            fault.kill().await?;
            fault.restart().await?;
            let restored = case.user().attach_session(original_id).await?;
            ensure!(restored.snapshot().await?.session.id == original_id);
            if expect_rollback {
                assert_session_title_count(&case, workspace_id, "interrupted-by-file-io", 0)
                    .await?;
            }
            // fsync can fail after bytes were written; its test intentionally
            // makes no claim that the interrupted row is absent after reopen.
            let fresh = case
                .client()
                .await
                .create_session(workspace_id, "healthy-after-file-io", None)
                .await?;
            ensure!(fresh.title == "healthy-after-file-io");
            assert_session_title_count(&case, workspace_id, "healthy-after-file-io", 1).await?;
            Ok(())
        })
        .await
}

async fn assert_session_title_count(
    case: &TestCase,
    workspace_id: i64,
    title: &str,
    expected: usize,
) -> TestResult {
    let client = case.client().await;
    let query = Query::ListSessions(ListSessionsParams {
        workspace_id: Some(workspace_id),
        search: Some(title.to_owned()),
        limit: Some(200),
        ..Default::default()
    });
    let result = tokio::time::timeout(Duration::from_secs(8), client.query_revalidated(query))
        .await
        .context("session visibility read blocked behind the held writer")??;
    let QueryResult::Sessions(page) = result else {
        anyhow::bail!("ListSessions returned a different query result");
    };
    ensure!(
        !page.page.has_more,
        "session title assertion did not read every matching row"
    );
    let actual = page
        .items
        .iter()
        .filter(|session| session.title == title)
        .count();
    ensure!(
        actual == expected,
        "expected {expected} session(s) with title {title:?}, got {actual}"
    );
    Ok(())
}
