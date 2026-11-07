mod recovery;

use std::{collections::BTreeMap, time::Duration};

use agena_api::resource::PluginSurfaceCatalogResponse;
use agena_client::AgenaClient;
use agena_plugin_sdk::{AttentionKind, PluginDisplayContent};
use agena_testkit::{EditOptions, Reply, RequestMatch, SettingsLayer, TestCase, TestResult};
use anyhow::{Context, ensure};
use serde_json::json;

use super::{allow_permissions, fixture};

pub(super) const CASES: &[(&str, fn() -> TestResult)] = &[(
    "agena.notifications",
    notifications_follow_real_runs_questions_reload_and_restart,
)];

fn activity(catalog: &PluginSurfaceCatalogResponse) -> Option<&str> {
    catalog.catalog.terminal.display.iter().find_map(|item| {
        if item.plugin_id.to_string() != "agena.notifications" {
            return None;
        }
        match &item.contribution.content {
            PluginDisplayContent::TerminalActivity { value } => Some(value.as_str()),
            _ => None,
        }
    })
}

async fn wait_catalog(
    client: &AgenaClient,
    description: &str,
    predicate: impl Fn(&PluginSurfaceCatalogResponse) -> bool,
) -> TestResult<PluginSurfaceCatalogResponse> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let catalog = client.plugin_surface_catalog().await?;
            if predicate(&catalog) {
                return Ok(catalog);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .with_context(|| format!("timed out waiting for {description}"))?
}

async fn reload_snapshot(case: &TestCase) -> TestResult {
    case.settings()
        .with_timeout(Duration::from_secs(20))
        .reload_and_wait()
        .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notifications_follow_real_runs_questions_reload_and_restart() -> TestResult {
    fixture().builtin_plugin("agena.notifications", serde_json::Value::Null)
    .expect_plugin("agena.notifications").run(|case| async move {
        let user = case.user().new_session().await?;
        allow_permissions(&user).await?;
        let client = case.client().await;
        wait_catalog(&client, "initial idle activity", |surface| activity(surface) == Some("idle")).await?;
        let step = case.model().expect(RequestMatch::user_text("attention-held-run"))
            .reply(Reply::text("attention-completed-marker").held());
        let run = user.send("attention-held-run").await?;
        step.observed().await?;
        wait_catalog(&client, "running activity", |surface| activity(surface) == Some("running")).await?;
        // The old host retains this running execution. The new host must
        // independently reconstruct Running before the old run can finish.
        reload_snapshot(&case).await?;
        wait_catalog(&client, "running activity in the reloaded snapshot", |surface| activity(surface) == Some("running")).await?;
        let other_user = case.user().new_session().await?;
        let other_step = case.model().expect(RequestMatch::user_text("attention-concurrent-run"))
            .reply(Reply::text("other-attention-completed").held());
        let other_run = other_user.send("attention-concurrent-run").await?;
        other_step.observed().await?;
        step.release();
        run.wait_settled().await?.assert_succeeded()?;
        wait_catalog(&client, "one completed session cannot clear another running session", |surface| activity(surface) == Some("running")).await?;
        other_step.release();
        other_run.wait_settled().await?.assert_succeeded()?;
        let completed_id = format!("execution:{}:{}", run.receipt().execution_id, run.receipt().attempt_generation);
        let completed = wait_catalog(&client, "real completion notification", |surface| {
            activity(surface) == Some("idle") && surface.notifications.iter().any(|item| {
                item.plugin_id == "agena.notifications" && item.session_id == Some(user.id())
                    && item.attention.as_ref().is_some_and(|attention| attention.id == completed_id && attention.kind == AttentionKind::Completed)
            })
        }).await?;
        ensure!(completed.notifications.iter().filter(|item| item.attention.as_ref().is_some_and(|attention| attention.id == completed_id)).count() == 1);
        // No lifecycle callback follows this reload: init/recovery alone must
        // restore the settled display in the fresh plugin host.
        reload_snapshot(&case).await?;
        wait_catalog(&client, "idle activity restored after an explicit settled reload", |surface| activity(surface) == Some("idle")).await?;

        let ask = user.start_tool("interaction.ask", json!({"questions": [{
            "header": "Color", "question": "Which fixture color?",
            "options": [{"label": "Blue", "description": "Use blue"}, {"label": "Green", "description": "Use green"}],
            "multiple": false
        }]})).await?;
        let pending = ask.run().wait_user_input().await?;
        let questions = wait_catalog(&client, "blocked activity and unresolved question notification", |surface| {
            activity(surface) == Some("blocked") && surface.notifications.iter().any(|item| item.session_id == Some(user.id())
                && item.attention.as_ref().is_some_and(|attention| attention.kind == AttentionKind::Question && !attention.resolved))
        }).await?;
        let question_id = questions.notifications.iter().find_map(|item| {
            item.attention.as_ref().filter(|attention| item.session_id == Some(user.id())
                && attention.kind == AttentionKind::Question && !attention.resolved).map(|attention| attention.id.clone())
        }).context("question identity")?;
        ensure!(questions.notifications.iter().filter(|item| item.attention.as_ref().is_some_and(|attention| attention.id == question_id)).count() == 1,
            "the canonical pending question was duplicated");
        pending.answer(BTreeMap::from([("0".to_owned(), vec!["Green".to_owned()])])).await?;
        ask.wait_settled().await?.assert_text_contains("Green")?;
        wait_catalog(&client, "resolved question replaces pending notification", |surface| {
            surface.notifications.iter().any(|item| item.attention.as_ref().is_some_and(|attention| attention.id == question_id && attention.resolved))
                && !surface.notifications.iter().any(|item| item.attention.as_ref().is_some_and(|attention| attention.id == question_id && !attention.resolved))
        }).await?;

        case.settings().set(SettingsLayer::Global, "plugins.list.\"agena.notifications\".enabled", json!(false), EditOptions::default())
            .await?.require_success()?;
        wait_catalog(&client, "disabled plugin removes display", |surface| activity(surface).is_none()).await?;
        let muted_user = case.user().new_session().await?;
        case.model().expect(RequestMatch::user_text("attention-disabled-run"))
            .reply(Reply::text("disabled-run-completed"));
        let muted_run = muted_user.send("attention-disabled-run").await?;
        muted_run.wait_settled().await?.assert_succeeded()?;
        let muted_id = format!("execution:{}:{}", muted_run.receipt().execution_id, muted_run.receipt().attempt_generation);
        let disabled = client.plugin_surface_catalog().await?;
        ensure!(disabled.notifications.iter().all(|item| !item.attention.as_ref().is_some_and(|attention| attention.id == muted_id)));

        case.settings().set(SettingsLayer::Global, "plugins.list.\"agena.notifications\".enabled", json!(true), EditOptions::default())
            .await?.require_success()?;
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if client.plugin_statuses().await?.items.iter().any(|item|
                    item.status.plugin_id.to_string() == "agena.notifications"
                        && item.status.state.as_ref() == "running") {
                    break Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }).await.context("notifications plugin did not reactivate")??;
        case.model().expect(RequestMatch::user_text("attention-reenabled-run"))
            .reply(Reply::text("reenabled-run-completed"));
        muted_user.send("attention-reenabled-run").await?.wait_settled().await?.assert_succeeded()?;
        let reenabled = wait_catalog(&client, "reenabled plugin display", |surface| activity(surface) == Some("idle")).await?;
        ensure!(reenabled.notifications.iter().all(|item| !item.attention.as_ref().is_some_and(|attention| attention.id == muted_id)),
            "re-enabling the observer replayed a completion it never observed");

        // Failure remains attention-worthy across a new host even though no
        // failed run.post callback can ever run in that new snapshot.
        case.model().expect(RequestMatch::user_text("attention-failed-run"))
            .reply(Reply::raw_json(400, json!({"error": {
                "message": "notification failure fixture", "type": "invalid_request_error"
            }})));
        muted_user.send("attention-failed-run").await?.wait_settled().await?.assert_failed()?;
        wait_catalog(&client, "failed execution activity", |surface| activity(surface) == Some("blocked")).await?;
        reload_snapshot(&case).await?;
        wait_catalog(&client, "failed activity restored after reload", |surface| activity(surface) == Some("blocked")).await?;
        case.model().expect(RequestMatch::user_text("attention-recovered-run"))
            .reply(Reply::text("recovered-run-completed"));
        muted_user.send("attention-recovered-run").await?.wait_settled().await?.assert_succeeded()?;
        wait_catalog(&client, "new completion clears the previous failure", |surface| activity(surface) == Some("idle")).await?;
        case.restart().await?;
        let restarted = case.client().await;
        let recovered = wait_catalog(&restarted, "persisted completion and question state", |surface| {
            surface.notifications.iter().any(|item| item.attention.as_ref().is_some_and(|attention| attention.id == completed_id))
                && surface.notifications.iter().any(|item| item.attention.as_ref().is_some_and(|attention| attention.id == question_id && attention.resolved))
        }).await?;
        ensure!(recovered.notifications.iter().filter(|item| item.attention.as_ref().is_some_and(|attention| attention.id == completed_id)).count() == 1);
        ensure!(recovered.notifications.iter().all(|item| !item.attention.as_ref().is_some_and(|attention| attention.id == question_id && !attention.resolved)));
        Ok(())
    }).await
}
