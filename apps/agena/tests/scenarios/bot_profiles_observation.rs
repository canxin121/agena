//! Looking at ordinary conversations must not create hidden Bot conversations.
use super::fixture;
use agena_testkit::{Reply, RequestMatch, TestCase, TestResult};
use anyhow::{Context, Result, ensure};
use reqwest::Method;
use serde_json::{Value, json};

async fn ids(case: &TestCase) -> Result<Vec<i64>> {
    let result: Value = case
        .http()
        .await
        .get("/api/v1/sessions?limit=50")
        .await?
        .require_success()?
        .json()?;
    result["items"]
        .as_array()
        .context("session list")?
        .iter()
        .map(|item| item["id"].as_i64().context("session ID"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_profiles_ordinary_profile_and_model_hooks_do_not_create_a_bot() -> TestResult {
    fixture()
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let before = ids(&case).await?;
            ensure!(before == vec![ordinary.id()]);
            ordinary.client().assistant_snapshot(ordinary.id()).await?;
            ensure!(
                ids(&case).await? == before,
                "reading ordinary assistant settings created a Bot"
            );
            case.model()
                .expect(RequestMatch::user_text("ordinary-without-bot-entry"))
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(
                        !body.contains("## Instance operator Bot")
                            && !body.contains("Current registered Bot identity"),
                        "ordinary conversation acquired Bot role from an observational hook"
                    );
                    Ok(())
                })
                .reply(Reply::text("ordinary native conversation"));
            ordinary
                .send("ordinary-without-bot-entry")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            ensure!(
                ids(&case).await? == before,
                "ordinary model hooks created a Bot conversation"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_profiles_unknown_explicit_identity_never_seeds_a_default() -> TestResult {
    fixture()
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let before = ids(&case).await?;
            let http = case.http().await;
            let unknown = uuid::Uuid::new_v4();
            for (method, path, body) in [
                (Method::GET, format!("/api/v1/bots/{unknown}"), None),
                (
                    Method::POST,
                    format!("/api/v1/bots/{unknown}/session"),
                    Some(json!({})),
                ),
            ] {
                let response = if let Some(body) = body {
                    http.send_json(method, &path, &body).await?
                } else {
                    http.get(&path).await?
                };
                ensure!(
                    response.status.as_u16() == 404,
                    "unknown explicit Bot must be not-found: {}",
                    response.status
                );
                ensure!(
                    ids(&case).await? == before,
                    "invalid explicit Bot lookup silently created a different default Bot"
                );
            }
            let profile = ordinary.client().assistant_snapshot(ordinary.id()).await?;
            ensure!(profile.source_session_id == ordinary.id());
            Ok(())
        })
        .await
}
