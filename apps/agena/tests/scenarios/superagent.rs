use std::time::Duration;

use agena_testkit::{Reply, RequestMatch, TestHttpClient, TestResult};
use anyhow::{Context, Result, ensure};
use reqwest::Method;
use serde_json::{Value, json};

use super::fixture;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_cancel_from_a_client_cannot_stop_a_newer_task_attempt() -> TestResult {
    fixture().run(|case| async move {
        let reply = case.model().expect(RequestMatch::user_text("stale-cancel-ui-marker"))
            .reply(Reply::text("still running until a current cancellation").held());
        let source = case.user().new_session().await?;
        let http = case.http().await;
        let task = post(&http, "/api/v1/tasks", json!({
            "source_session_id":source.id(),"request_key":"stale-cancel-ui","goal":"stale-cancel-ui-marker"
        })).await?;
        let id = task["id"].as_str().context("task id")?;
        reply.observed().await?;
        let running = wait_task(&http, id, "running").await?;
        ensure!(running["revision"] != task["revision"]);
        let stale = http.send_json(Method::POST, &format!("/api/v1/tasks/{id}/cancel"), &json!({
            "expected_revision":task["revision"]
        })).await?;
        ensure!(stale.status.as_u16() == 409, "stale cancellation was accepted");
        let current: Value = http.get(&format!("/api/v1/tasks/{id}")).await?.require_success()?.json()?;
        ensure!(current["state"] == "running");
        post(&http, &format!("/api/v1/tasks/{id}/cancel"), json!({"expected_revision":current["revision"]})).await?;
        wait_task(&http, id, "cancelled").await?;
        reply.disconnected().await?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_routes_keep_the_existing_operator_authentication_boundary() -> TestResult {
    fixture()
        .password("isolated-task-fixture-password")
        .run(|case| async move {
            let http = case.http().await;
            let response = http.anonymous().get("/api/v1/tasks").await?;
            ensure!(response.status.as_u16() == 401);
            let response = http
                .anonymous()
                .send_json(
                    Method::POST,
                    "/api/v1/chatbot/messages",
                    &json!({
                        "session_id":1,"message_id":"unauthorized","text":"/task must-not-run"
                    }),
                )
                .await?;
            ensure!(response.status.as_u16() == 401);
            let tasks: Value = http.get("/api/v1/tasks").await?.require_success()?.json()?;
            ensure!(tasks.as_array().context("tasks")?.is_empty());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_worker_preserves_permission_denials_and_does_not_copy_chat_history() -> TestResult {
    fixture().run(|case| async move {
        let model = case.model();
        model.expect(RequestMatch::user_text("source-private-marker"))
            .reply(Reply::text("private-source-reply"));
        let call = model.expect(RequestMatch::user_text("task-permission-marker"))
            .check(|request| {
                let body = request.body.to_string();
                ensure!(!body.contains("source-private-marker") && !body.contains("private-source-reply"), "worker inherited private chat history");
                Ok(())
            })
            .reply(Reply::tool_call("denied-task-write", "fs.write", json!({"path":"protected.txt","content":"forbidden"})));
        let result = model.expect(RequestMatch::tool_output("denied-task-write"))
            .after(&call)
            .check(|request| {
                let output: Value = serde_json::from_str(&request.tool_output_text("denied-task-write").context("tool result")?)?;
                ensure!(output["phase"] == "failed", "worker bypassed source denial: {output}");
                Ok(())
            }).reply(Reply::text("write denied; requires operator review"));
        let source = case.user().new_session().await?;
        source.send("source-private-marker").await?.wait_settled().await?.assert_succeeded()?;
        let target = case.workspace().join("protected.txt");
        super::path_permission(&source, &target, "deny").await?;
        let http = case.http().await;
        let task = post(&http, "/api/v1/tasks", json!({"source_session_id":source.id(),"request_key":"permission-task","goal":"task-permission-marker"})).await?;
        wait_task(&http, task["id"].as_str().context("task id")?, "review").await?;
        result.finished().await?;
        ensure!(!target.exists(), "denied tool modified the filesystem");
        Ok(())
    }).await
}

async fn post(http: &TestHttpClient, path: &str, body: Value) -> Result<Value> {
    http.send_json(Method::POST, path, &body)
        .await?
        .require_success()?
        .json()
}

async fn wait_task(http: &TestHttpClient, id: &str, expected: &str) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let task: Value = http
                .get(&format!("/api/v1/tasks/{id}"))
                .await?
                .require_success()?
                .json()?;
            if task["state"] == expected {
                return Ok(task);
            }
            ensure!(
                !matches!(task["state"].as_str(), Some("failed" | "interrupted"))
                    || matches!(expected, "failed" | "interrupted"),
                "unexpected task state: {task}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .with_context(|| format!("task {id} did not reach {expected}"))?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_chat_task_is_idempotent_scoped_and_requires_acceptance() -> TestResult {
    fixture().run(|case| async move {
        let reply = case.model().expect(RequestMatch::user_text("task-delivery-marker"))
            .reply(Reply::text("implementation and verification evidence"));
        let source = case.user().new_session().await?;
        let other = case.user().new_session().await?;
        let http = case.http().await;
        let body = json!({"session_id":source.id(),"message_id":"event-1","text":"/task task-delivery-marker"});
        let response = post(&http, "/api/v1/chatbot/messages", body.clone()).await?;
        let duplicate = post(&http, "/api/v1/chatbot/messages", body).await?;
        ensure!(response["task"]["id"] == duplicate["task"]["id"]);
        let id = response["task"]["id"].as_str().context("task id")?;
        let task = wait_task(&http, id, "review").await?;
        reply.finished().await?;
        let worker_id = task["worker_session_id"].as_i64().context("worker id")?;
        ensure!(worker_id != source.id(), "worker reused the interactive conversation");
        let worker = case.user().attach_session(worker_id).await?;
        ensure!(worker.snapshot().await?.session.workspace_id == source.snapshot().await?.session.workspace_id);
        let hidden = http.send_json(Method::POST, "/api/v1/chatbot/messages", &json!({
            "session_id":other.id(),"message_id":"probe","text":format!("/status {id}")
        })).await?;
        ensure!(hidden.status.as_u16() == 404, "cross-conversation task leaked: {:?}", hidden.status);
        let changed = http.send_json(Method::POST, "/api/v1/tasks", &json!({
            "source_session_id":source.id(),"request_key":"event-1","goal":"different task"
        })).await?;
        ensure!(changed.status.as_u16() == 409);
        let listed = post(&http, "/api/v1/chatbot/messages", json!({
            "session_id":source.id(),"message_id":"list","text":"/tasks"
        })).await?;
        ensure!(listed["items"].as_array().context("items")?.len() == 1);
        let accepted = post(&http, &format!("/api/v1/tasks/{id}/accept"), json!({"expected_revision":task["revision"]})).await?;
        ensure!(accepted["state"] == "completed");
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_chat_does_not_create_a_task_and_replay_does_not_repeat_the_model() -> TestResult {
    fixture().run(|case| async move {
        let reply = case.model().expect(RequestMatch::user_text("ordinary-chat-marker"))
            .reply(Reply::text("a conversational response"));
        let source = case.user().new_session().await?;
        let http = case.http().await;
        let body = json!({"session_id":source.id(),"message_id":"chat-1","text":"ordinary-chat-marker"});
        let first = post(&http, "/api/v1/chatbot/messages", body.clone()).await?;
        reply.finished().await?;
        let replay = post(&http, "/api/v1/chatbot/messages", body).await?;
        let changed = http.send_json(Method::POST, "/api/v1/chatbot/messages", &json!({
            "session_id":source.id(),"message_id":"chat-1","text":"/task changed-event-must-not-run"
        })).await?;
        ensure!(changed.status.as_u16() == 409, "chat event changed its meaning during replay");
        ensure!(first["kind"] == "conversation");
        ensure!(first["execution_id"] == replay["execution_id"]);
        let tasks: Value = http.get("/api/v1/tasks").await?.require_success()?.json()?;
        ensure!(tasks.as_array().context("tasks")?.is_empty());
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_and_running_tasks_can_be_cancelled_without_launching_queued_work() -> TestResult {
    fixture().run(|case| async move {
        let reply = case.model().expect(RequestMatch::user_text("cancel-task-marker"))
            .reply(Reply::text("must not be completed").held());
        let source = case.user().new_session().await?;
        let http = case.http().await;
        let running = post(&http, "/api/v1/tasks", json!({
            "source_session_id":source.id(),"request_key":"cancel-running","goal":"cancel-task-marker"
        })).await?;
        reply.observed().await?;
        let queued = post(&http, "/api/v1/tasks", json!({
            "source_session_id":source.id(),"request_key":"cancel-queued","goal":"must-never-reach-model"
        })).await?;
        let queued_id = queued["id"].as_str().context("queued id")?;
        post(&http, &format!("/api/v1/tasks/{queued_id}/cancel"), json!({})).await?;
        let queued = wait_task(&http, queued_id, "cancelled").await?;
        ensure!(queued["worker_session_id"].is_null());
        let running_id = running["id"].as_str().context("running id")?;
        post(&http, &format!("/api/v1/tasks/{running_id}/cancel"), json!({})).await?;
        wait_task(&http, running_id, "cancelled").await?;
        reply.disconnected().await?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn crash_keeps_task_identity_and_requires_explicit_idempotent_resume() -> TestResult {
    fixture().run(|case| async move {
        let interrupted = case.model().expect(RequestMatch::user_text("crash-task-marker"))
            .reply(Reply::text("unconfirmed-result").held());
        let resumed = case.model().expect(RequestMatch::user_text("resume-safe-marker"))
            .after(&interrupted).reply(Reply::text("verified after recovery"));
        let source = case.user().new_session().await?;
        let http = case.http().await;
        let task = post(&http, "/api/v1/tasks", json!({
            "source_session_id":source.id(),"request_key":"restart-event","goal":"crash-task-marker"
        })).await?;
        let id = task["id"].as_str().context("task id")?.to_owned();
        interrupted.observed().await?;
        let before = wait_task(&http, &id, "running").await?;
        case.crash().await?;
        interrupted.disconnected().await?;
        case.restart().await?;
        let http = case.http().await;
        let recovered = wait_task(&http, &id, "interrupted").await?;
        ensure!(recovered["worker_session_id"] == before["worker_session_id"]);
        let request = json!({"expected_revision":recovered["revision"],"request_key":"explicit-resume","instruction":"resume-safe-marker: inspect previous work first"});
        let started = post(&http, &format!("/api/v1/tasks/{id}/resume"), request.clone()).await?;
        let replay = post(&http, &format!("/api/v1/tasks/{id}/resume"), request).await?;
        ensure!(started["attempt"] == 2 && replay["attempt"] == 2);
        let result = wait_task(&http, &id, "review").await?;
        ensure!(result["worker_session_id"] == before["worker_session_id"]);
        ensure!(result["execution_id"] != before["execution_id"]);
        resumed.finished().await?;
        Ok(())
    }).await
}
