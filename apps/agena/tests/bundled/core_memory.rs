//! Behavioral memory regressions through the real Agena process and model input.
use agena_testkit::{Reply, RequestMatch, TestAgenaBuilder, TestResult};
use anyhow::{Context, ensure};
use serde_json::json;

use super::{call, fixture, user};

fn memory_fixture() -> TestAgenaBuilder {
    fixture()
        .expect_plugin("agena.memory")
        .configure_global(|config| {
            config["plugins"]["list"]["agena.memory"] = json!({
                "package":{"kind":"static"},
                "settings":{"maintenance":{"enabled":false}}
            });
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_chinese_natural_query_recalls_late_evidence_after_restart() -> TestResult {
    memory_fixture()
        .run(|case| async move {
            let writer = user(&case).await?;
            let body = format!(
                "{}\n数据库选型：PostgreSQL，late-database-proof。\n{}",
                "早期调研背景材料。\n".repeat(180),
                "补充说明。\n".repeat(80)
            );
            let saved = call(
                &writer,
                "memory.write",
                json!({
                    "name":"database-decision", "description":"数据库选型",
                    "memory_type":"project", "content":body
                }),
            )
            .await?;
            assert_eq!(
                saved.data()?["metadata"]["source"]["session_id"],
                writer.id()
            );
            assert_eq!(saved.data()?["metadata"]["provenance_verified"], true);
            let found = call(
                &writer,
                "memory.search",
                json!({"query":"我们之前数据库选型是什么"}),
            )
            .await?;
            let hit = found.data()?["results"]
                .as_array()
                .context("search results")?
                .iter()
                .find(|hit| hit["name"] == "database-decision")
                .context("Chinese recall")?;
            ensure!(
                hit["excerpt"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("late-database-proof")
            );
            ensure!(hit["excerpt"]["start_line"].as_u64().unwrap_or_default() > 100);
            ensure!(hit.get("searchable_text").is_none());
            case.restart().await?;
            let reader = user(&case).await?;
            case.model()
                .expect(RequestMatch::user_text("我们之前数据库选型是什么"))
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(
                        body.contains("late-database-proof"),
                        "matched evidence missing from actual provider request"
                    );
                    ensure!(body.contains("PostgreSQL"));
                    ensure!(!body.contains(&"早期调研背景材料。".repeat(30)));
                    Ok(())
                })
                .reply(Reply::text("memory-evidence-restored"));
            reader
                .send("我们之前数据库选型是什么")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("memory-evidence-restored")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_correction_removes_old_fact_from_default_recall_but_keeps_history() -> TestResult {
    memory_fixture()
        .run(|case| async move {
            let writer = user(&case).await?;
            let old = call(
                &writer,
                "memory.write",
                json!({
                    "name":"database-before", "description":"数据库选型",
                    "memory_type":"project", "content":"数据库以前使用 SQLite，obsolete-db-proof。"
                }),
            )
            .await?;
            let revision = old.data()?["sha256"]
                .as_str()
                .context("old revision")?
                .to_string();
            call(&writer, "memory.write", json!({
            "name":"database-current", "description":"数据库选型",
            "memory_type":"project", "content":"数据库现在使用 PostgreSQL，current-db-proof。",
            "supersedes":[{"name":"database-before","sha256":revision}]
        })).await?;
            let previous = call(&writer, "memory.get", json!({"name":"database-before"})).await?;
            assert_eq!(previous.data()?["metadata"]["status"], "superseded");
            let active = call(&writer, "memory.search", json!({"query":"数据库"})).await?;
            let active_hits = active.data()?["results"]
                .as_array()
                .context("active results")?;
            ensure!(
                active_hits
                    .iter()
                    .any(|hit| hit["name"] == "database-current")
            );
            ensure!(
                active_hits
                    .iter()
                    .all(|hit| hit["name"] != "database-before")
            );
            let history = call(
                &writer,
                "memory.search",
                json!({"query":"数据库","include_history":true}),
            )
            .await?;
            ensure!(
                history.data()?["results"]
                    .as_array()
                    .context("history results")?
                    .iter()
                    .any(|hit| hit["name"] == "database-before")
            );
            writer
                .call_tool(
                    "memory.write",
                    json!({
                        "name":"stale-correction", "content":"must not be committed",
                        "supersedes":[{"name":"database-before","sha256":revision}]
                    }),
                )
                .await?
                .assert_failed()?;
            writer
                .call_tool("memory.get", json!({"name":"stale-correction"}))
                .await?
                .assert_failed()?;
            let reader = user(&case).await?;
            case.model()
                .expect(RequestMatch::user_text("我们之前数据库选型是什么"))
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(body.contains("current-db-proof"));
                    ensure!(
                        !body.contains("obsolete-db-proof"),
                        "superseded content entered automatic recall"
                    );
                    Ok(())
                })
                .reply(Reply::text("current-fact-only"));
            reader
                .send("我们之前数据库选型是什么")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("current-fact-only")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_core_profile_covers_four_preferences_and_task_scope_does_not_leak() -> TestResult {
    memory_fixture()
        .run(|case| async move {
            let writer = user(&case).await?;
            for (name, marker) in [
                ("a-style", "core-preference-alpha"),
                ("b-tests", "core-preference-beta"),
                ("c-format", "core-preference-gamma"),
                ("z-important", "core-preference-delta"),
            ] {
                call(
                    &writer,
                    "memory.write",
                    json!({
                        "name":name, "description":"User preference", "memory_type":"user",
                        "content":marker, "importance":75
                    }),
                )
                .await?;
            }
            let task = call(
                &writer,
                "memory.write",
                json!({
                    "name":"private-task", "content":"session-task-proof",
                    "kind":"task", "scope":"session", "memory_type":"project"
                }),
            )
            .await?;
            assert_eq!(task.data()?["metadata"]["scope"]["session_id"], writer.id());
            ensure!(task.data()?["metadata"]["scope"]["instance_id"].is_string());
            let own = call(
                &writer,
                "memory.search",
                json!({"query":"session-task-proof"}),
            )
            .await?;
            ensure!(
                own.data()?["results"]
                    .as_array()
                    .context("own task")?
                    .iter()
                    .any(|hit| hit["name"] == "private-task")
            );
            let reader = user(&case).await?;
            let foreign = call(
                &reader,
                "memory.search",
                json!({"query":"session-task-proof"}),
            )
            .await?;
            ensure!(
                foreign.data()?["results"]
                    .as_array()
                    .context("foreign task")?
                    .is_empty()
            );
            reader
                .call_tool("memory.get", json!({"name":"private-task"}))
                .await?
                .assert_failed()?;
            reader
                .call_tool(
                    "memory.write",
                    json!({
                        "name":"private-task", "content":"foreign overwrite",
                        "expected_sha256":task.data()?["sha256"]
                    }),
                )
                .await?
                .assert_failed()?;
            reader
                .call_tool(
                    "memory.delete",
                    json!({
                        "name":"private-task", "expected_sha256":task.data()?["sha256"]
                    }),
                )
                .await?
                .assert_failed()?;
            reader
                .call_tool(
                    "memory.write",
                    json!({
                        "name":"foreign-replacement", "content":"foreign supersession",
                        "supersedes":[{"name":"private-task","sha256":task.data()?["sha256"]}]
                    }),
                )
                .await?
                .assert_failed()?;
            let untouched = call(&writer, "memory.get", json!({"name":"private-task"})).await?;
            ensure!(
                untouched.data()?["body"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("session-task-proof")
            );
            let fresh = user(&case).await?;
            case.model()
                .expect(RequestMatch::user_text("请规划今天的工作安排"))
                .check(|request| {
                    let body = request.body.to_string();
                    for marker in ["alpha", "beta", "gamma", "delta"] {
                        ensure!(
                            body.contains(&format!("core-preference-{marker}")),
                            "missing core preference {marker}"
                        );
                    }
                    ensure!(
                        !body.contains("session-task-proof"),
                        "session-scoped task leaked"
                    );
                    Ok(())
                })
                .reply(Reply::text("all-four-preferences"));
            fresh
                .send("请规划今天的工作安排")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("all-four-preferences")?;
            Ok(())
        })
        .await
}
