//! Usage reporting spans every project and global conversation once a server
//! hosts more than one workspace.
use std::time::Duration;

use agena_domain::UsagePeriod;
use agena_testkit::TestResult;
use anyhow::{Context, ensure};
use serde_json::json;

use super::fixture;

/// One tool round trip records provider usage without depending on a streamed
/// reply: the scripted tool call is its own model turn.
async fn record_usage(user: &agena_testkit::UserSession) -> TestResult {
    user.call_tool("session.environment", json!({}))
        .await?
        .assert_succeeded()?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn usage_reports_sessions_from_every_workspace_and_global_bots() -> TestResult {
    fixture()
        .case_timeout(Duration::from_secs(120))
        .run(|case| async move {
            let default_project = case.user().new_session().await?;
            record_usage(&default_project).await?;

            // Creating a session in a sibling directory registers that project
            // but deliberately leaves the server's own workspace alone.
            let other_project = case.workspace().join("another-project");
            tokio::fs::create_dir(&other_project).await?;
            let other = case.user().new_session_in(&other_project).await?;
            record_usage(&other).await?;

            let client = case.client().await;
            let bot_id = client.ensure_bot_session().await?.session_id;
            record_usage(&case.user().attach_session(bot_id).await?).await?;

            let stats = client
                .usage_stats(UsagePeriod::AllTime, None, None, &[], &[], &[], true, 0)
                .await
                .context("read usage statistics")?;
            let seen = |session_id: i64| {
                stats
                    .by_session
                    .iter()
                    .any(|row| row.session_id == session_id && row.totals.requests > 0)
            };
            ensure!(
                seen(default_project.id()),
                "the server workspace conversation is missing from usage: {:?}",
                stats.by_session
            );
            ensure!(
                seen(other.id()),
                "a sibling project conversation is missing from usage: {:?}",
                stats.by_session
            );
            ensure!(
                seen(bot_id),
                "a global Bot conversation is missing from usage: {:?}",
                stats.by_session
            );
            Ok(())
        })
        .await
}
