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
                    "kind":"fact", "content":body
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

            // Damage only the first LZ4 document block, preserving metadata,
            // manifest, segment lengths and footer so the failure is detected
            // when a search reads the stored payload.
            let layout = agena_storage_layout::StorageLayout::new(case.paths().data_dir.clone());
            let mut directories = vec![layout.memory_index_dir(case.workspace())];
            let mut corrupted = false;
            while let Some(directory) = directories.pop() {
                for entry in std::fs::read_dir(directory)? {
                    let entry = entry?;
                    let kind = entry.file_type()?;
                    if kind.is_dir() {
                        directories.push(entry.path());
                    } else if kind.is_file()
                        && entry
                            .path()
                            .extension()
                            .is_some_and(|extension| extension == "store")
                    {
                        let mut bytes = std::fs::read(entry.path())?;
                        ensure!(bytes.len() > 4);
                        ensure!(u32::from_le_bytes(bytes[..4].try_into()?) > 0);
                        bytes[..4].fill(0);
                        std::fs::write(entry.path(), bytes)?;
                        corrupted = true;
                        break;
                    }
                }
                if corrupted {
                    break;
                }
            }
            ensure!(
                corrupted,
                "the actual memory search cache had no stored document block"
            );
            let recovered = call(
                &writer,
                "memory.search",
                json!({"query":"我们之前数据库选型是什么"}),
            )
            .await?;
            ensure!(
                recovered.data()?["results"]
                    .as_array()
                    .context("recovered search results")?
                    .iter()
                    .any(|item| item["name"] == "database-decision"
                        && item["excerpt"]["text"]
                            .as_str()
                            .unwrap_or_default()
                            .contains("late-database-proof")),
                "memory.search did not recover its damaged stored payload"
            );
            let unchanged =
                call(&writer, "memory.get", json!({"name":"database-decision"})).await?;
            assert_eq!(unchanged.data()?["sha256"], saved.data()?["sha256"]);
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
                    "kind":"fact", "content":"数据库以前使用 SQLite，obsolete-db-proof。"
                }),
            )
            .await?;
            let revision = old.data()?["sha256"]
                .as_str()
                .context("old revision")?
                .to_string();
            call(
                &writer,
                "memory.write",
                json!({
                    "name":"database-current", "description":"数据库选型",
                    "kind":"fact", "content":"数据库现在使用 PostgreSQL，current-db-proof。",
                    "supersedes":[{"name":"database-before","sha256":revision}]
                }),
            )
            .await?;
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
                        "name":"stale-correction", "kind":"fact", "content":"must not be committed",
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
                        "name":name, "description":"User preference", "kind":"preference",
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
                    "kind":"task", "scope":"session"
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
                        "name":"private-task", "kind":"task", "content":"foreign overwrite",
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
                        "name":"foreign-replacement", "kind":"fact", "content":"foreign supersession",
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_tool_unicode_names_can_be_managed_through_http() -> TestResult {
    use reqwest::Method;
    use serde_json::Value;

    memory_fixture()
        .run(|case| async move {
            let writer = user(&case).await?;
            let http = case.http().await;
            let name = "中文 偏好";
            let created = call(
                &writer,
                "memory.write",
                json!({"name":name,"kind":"preference","content":"原始偏好"}),
            )
            .await?;
            let path = format!("/api/v1/memories/{name}");
            let opened: Value = http.get(&path).await?.require_success()?.json()?;
            assert_eq!(opened["name"], name);
            assert_eq!(opened["sha256"], created.data()?["sha256"]);
            let updated: Value = http
                .send_json(
                    Method::PUT,
                    &path,
                    &json!({
                        "kind":"preference",
                        "body":"更新后的偏好",
                        "expected_sha256":opened["sha256"]
                    }),
                )
                .await?
                .require_success()?
                .json()?;
            let current = call(&writer, "memory.get", json!({"name":name})).await?;
            assert_eq!(current.data()?["sha256"], updated["sha256"]);
            current.assert_text_contains("更新后的偏好")?;
            let revision = updated["sha256"].as_str().context("updated revision")?;
            let deleted: Value = http
                .send_json(
                    Method::DELETE,
                    &format!("{path}?expected_sha256={revision}"),
                    &json!({}),
                )
                .await?
                .require_success()?
                .json()?;
            assert_eq!(deleted["name"], name);
            assert_eq!(deleted["sha256"], updated["sha256"]);
            assert_eq!(http.get(&path).await?.status.as_u16(), 404);

            let batch_name = "项目 决定";
            let batch_record = call(
                &writer,
                "memory.write",
                json!({"name":batch_name,"kind":"fact","content":"可批量删除"}),
            )
            .await?;
            let deleted: Vec<String> = http
                .send_json(
                    Method::POST,
                    "/api/v1/memories/batch-delete",
                    &json!({"records":[{
                        "name":format!("{batch_name}.md"),
                        "sha256":batch_record.data()?["sha256"]
                    }]}),
                )
                .await?
                .require_success()?
                .json()?;
            assert_eq!(deleted, [batch_name]);
            let remaining: Vec<Value> = http
                .get("/api/v1/memories")
                .await?
                .require_success()?
                .json()?;
            ensure!(remaining.is_empty());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_http_requires_canonical_kind_and_each_mutation_revision() -> TestResult {
    use reqwest::Method;
    use serde_json::Value;

    memory_fixture()
        .run(|case| async move {
            let http = case.http().await;
            let first_path = "/api/v1/memories/http-first";
            let first: Value = http
                .send_json(
                    Method::PUT,
                    first_path,
                    &json!({"kind":"fact","body":"original"}),
                )
                .await?
                .require_success()?
                .json()?;
            assert_eq!(first["metadata"]["kind"], "fact");
            ensure!(first["sha256"].is_string());
            ensure!(first.get("memory_type").is_none());
            for invalid in [
                json!({"body":"missing kind"}),
                json!({"body":"old type","memory_type":"project"}),
                json!({"body":"mixed schema","kind":"fact","memory_type":"project"}),
            ] {
                let response = http.send_json(Method::PUT, first_path, &invalid).await?;
                assert_eq!(response.status.as_u16(), 422);
            }
            let unversioned = http
                .send_json(
                    Method::PUT,
                    first_path,
                    &json!({"kind":"fact","body":"unversioned"}),
                )
                .await?;
            assert_eq!(unversioned.status.as_u16(), 409);
            let missing_delete_revision = http
                .send_json(Method::DELETE, first_path, &json!({}))
                .await?;
            assert_eq!(missing_delete_revision.status.as_u16(), 400);
            let second_path = "/api/v1/memories/http-second";
            let second: Value = http
                .send_json(
                    Method::PUT,
                    second_path,
                    &json!({"kind":"preference","body":"second"}),
                )
                .await?
                .require_success()?
                .json()?;
            let batch_path = "/api/v1/memories/batch-delete";
            for invalid in [
                json!({"names":["http-first"],"expected_sha256":{"http-first":first["sha256"]}}),
                json!({"records":[{"name":"http-first"}]}),
            ] {
                assert_eq!(
                    http.send_json(Method::POST, batch_path, &invalid)
                        .await?
                        .status
                        .as_u16(),
                    422
                );
            }
            let stale_batch = http
                .send_json(
                    Method::POST,
                    batch_path,
                    &json!({
                        "records": [
                            {"name":"http-first","sha256":first["sha256"]},
                            {"name":"http-second","sha256":"stale"}
                        ]
                    }),
                )
                .await?;
            assert_eq!(stale_batch.status.as_u16(), 409);
            for (path, expected) in [(first_path, &first), (second_path, &second)] {
                let current: Value = http.get(path).await?.require_success()?.json()?;
                assert_eq!(current["sha256"], expected["sha256"]);
                assert_eq!(current["body"], expected["body"]);
            }
            let deleted: Vec<String> = http
                .send_json(
                    Method::POST,
                    batch_path,
                    &json!({
                        "records": [
                            {"name":"http-first","sha256":first["sha256"]},
                            {"name":"http-second","sha256":second["sha256"]}
                        ]
                    }),
                )
                .await?
                .require_success()?
                .json()?;
            assert_eq!(deleted, ["http-first", "http-second"]);
            let remaining: Vec<Value> = http
                .get("/api/v1/memories")
                .await?
                .require_success()?
                .json()?;
            ensure!(remaining.is_empty());
            Ok(())
        })
        .await
}
