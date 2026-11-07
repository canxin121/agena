//! Behavioral coverage through the real server, provider HTTP and plugin host.
//! Model scripts choose actions; persisted output, session state and external
//! effects establish what the product actually did.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use agena_api::{live::ToolDetailSection, resource::UserInputReplyKind};
use agena_domain::{AgentStatus, RawOutput};
use agena_testkit::{ModelRequest, Reply, RequestMatch, TestResult, UserSession};
use anyhow::{Context, Result, ensure};
use chrono::{Datelike, Timelike, Utc};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_tungstenite::tungstenite::Message;

use super::{allow_permissions, fixture};

pub(super) const CASES: &[(&str, fn() -> TestResult)] = &[
    (
        "agena.agent",
        agent_runs_messages_follows_up_and_cancels_a_real_child,
    ),
    (
        "agena.cron",
        cron_persists_edits_and_delivers_a_real_scheduled_wake,
    ),
    (
        "agena.monitor",
        monitor_delivers_websocket_events_and_stops_the_owned_connection,
    ),
    (
        "agena.plan",
        plan_requires_review_and_enforces_the_execution_gate,
    ),
    (
        "agena.session",
        session_tools_observe_and_mutate_the_actual_session,
    ),
    (
        "agena.interaction",
        interaction_persists_notifications_and_resolves_user_choices,
    ),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_tools_observe_and_mutate_the_actual_session() -> TestResult {
    fixture()
        .expect_plugin("agena.session")
        .run(|case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let initial = user.call_tool("session.get", json!({})).await?;
            initial.assert_succeeded()?;
            ensure!(initial.data()?["session"]["id"] == user.id());
            ensure!(initial.data()?["session"]["is_subagent"] == false);

            let renamed = user
                .call_tool("session.rename", json!({"title": "runtime session proof"}))
                .await?;
            renamed.assert_succeeded()?;
            ensure!(renamed.data()?["session"]["title"] == "runtime session proof");
            ensure!(user.snapshot().await?.session.title == "runtime session proof");
            let reread = user.call_tool("session.get", json!({})).await?;
            reread.assert_succeeded()?;
            ensure!(reread.data()?["session"]["title"] == "runtime session proof");

            let environment = user.call_tool("session.environment", json!({})).await?;
            environment.assert_succeeded()?;
            ensure!(
                environment.data()?["workspace_root"]
                    == case.workspace().to_string_lossy().as_ref()
            );
            ensure!(
                environment.data()?["os"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty())
            );
            ensure!(
                environment.data()?["arch"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty())
            );
            ensure!(
                environment.data()?["tool_catalog"]["count"]
                    .as_u64()
                    .is_some_and(|count| count > 0),
                "environment omitted the real tool catalog: {}",
                environment.data()?
            );

            let executables = user
                .call_tool(
                    "session.executables",
                    json!({
                        "names": ["git"], "refresh": true, "probe_versions": false
                    }),
                )
                .await?;
            executables.assert_succeeded()?;
            let tools = executables.data()?["tools"]
                .as_array()
                .context("executable inventory")?;
            ensure!(tools.len() == 1 && tools[0]["name"] == "git");
            ensure!(tools[0]["available"].is_boolean());
            if tools[0]["available"] == true {
                ensure!(
                    tools[0]["executable"]
                        .as_str()
                        .is_some_and(|path| !path.is_empty())
                );
            }
            let unknown = user
                .call_tool(
                    "session.executables",
                    json!({"names": ["agena-not-a-catalog-cli"]}),
                )
                .await?;
            unknown.assert_failed()?;

            let selection = user.call_tool("session.model", json!({})).await?;
            selection.assert_succeeded()?;
            ensure!(selection.data()?["session_id"] == user.id());
            ensure!(selection.data()?["model_provider_id"] == "testkit");
            ensure!(selection.data()?["model_adapter_id"] == "openai_responses");
            ensure!(selection.data()?["model_id"] == "fake-model");

            let tokens = user.call_tool("session.tokens", json!({})).await?;
            tokens.assert_succeeded()?;
            ensure!(tokens.data()?["session_id"] == user.id());
            ensure!(tokens.data()?["current_tokens"].as_u64().is_some());
            ensure!(tokens.data()?["reserved_tokens"].as_u64().is_some());
            if let (Some(remaining), Some(limit)) = (
                tokens.data()?["remaining_tokens"].as_u64(),
                tokens.data()?["limit_tokens"].as_u64(),
            ) {
                ensure!(remaining <= limit);
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interaction_persists_notifications_and_resolves_user_choices() -> TestResult {
    fixture()
        .expect_plugin("agena.interaction")
        .run(|case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let notice = user
                .call_tool(
                    "interaction.notify",
                    json!({
                        "title": "Fixture notification",
                        "body_markdown": "**runtime-notice-proof**",
                        "level": "warning"
                    }),
                )
                .await?;
            notice.assert_succeeded()?;
            ensure!(notice.data()?["title"] == "Fixture notification");
            ensure!(notice.data()?["body_markdown"] == "**runtime-notice-proof**");
            ensure!(notice.data()?["level"] == "warning");
            ensure!(notice.data()?["metadata"]["agena.effect"] == "notification");
            ensure!(
                user.snapshot()
                    .await?
                    .session
                    .state
                    .pending_interactive_requests()
                    .is_empty()
            );

            let input = json!({
                "title": "Choose the fixture value",
                "questions": [{
                    "header": "Color",
                    "question": "Which color should the fixture record?",
                    "options": [
                        {"label": "Blue", "description": "Record blue"},
                        {"label": "Green", "description": "Record green"}
                    ],
                    "multiple": false
                }]
            });
            let asking = user.start_tool("interaction.ask", input.clone()).await?;
            let pending = asking.run().wait_user_input().await?;
            ensure!(serde_json::to_string(&pending.request)?.contains("Which color"));
            ensure!(pending.session_id() == user.id());
            pending.mark_presented().await?;
            pending
                .answer(BTreeMap::from([("0".to_owned(), vec!["Green".to_owned()])]))
                .await?;
            let answered = asking.wait_settled().await?;
            answered.assert_succeeded()?;
            ensure!(answered.data()?["answers"]["0"] == json!(["Green"]));
            ensure!(
                user.snapshot()
                    .await?
                    .session
                    .state
                    .pending_interactive_requests()
                    .is_empty()
            );

            let asking = user.start_tool("interaction.ask", input).await?;
            let pending = asking.run().wait_user_input().await?;
            pending
                .reply_user_input(
                    UserInputReplyKind::Cancel,
                    BTreeMap::new(),
                    Some("fixture declined".to_owned()),
                )
                .await?;
            let cancelled = asking.wait_settled().await?;
            cancelled.assert_failed()?;
            ensure!(
                user.snapshot()
                    .await?
                    .session
                    .state
                    .pending_interactive_requests()
                    .is_empty()
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plan_requires_review_and_enforces_the_execution_gate() -> TestResult {
    fixture().expect_plugin("agena.plan").run(|case| async move {
        let user = case.user().new_session().await?;
        allow_permissions(&user).await?;
        let empty = user.call_tool("plan.get", json!({"view": "full"})).await?;
        empty.assert_succeeded()?;
        ensure!(empty.data()?["plan"].is_null());

        // The default trusted setting must reject bypassing review even when
        // ordinary tool/path permission has been explicitly granted.
        let bypass = user.call_tool("plan.set", json!({
            "objective": "unreviewed plan", "request_approval": false, "autorun": false
        })).await?;
        bypass.assert_failed()?;
        let saved = user.call_tool("plan.set", json!({
            "title": "Runtime plan",
            "objective": "Write and verify one local fixture",
            "document_markdown": "Create plan-proof.txt after approval.",
            "autorun": false,
            "steps": [{"title": "Write fixture", "checks": [{"text": "File content matches"}]}]
        })).await?;
        saved.assert_succeeded()?;
        let plan = &saved.data()?["plan"];
        ensure!(plan["phase"] == "planning" && plan["autorun"] == false);
        let revision = plan["revision"].as_str().context("saved plan revision")?.to_owned();

        let edited = user.call_tool("plan.edit", json!({
            "expected_revision": revision, "step": 1, "status": "in_progress", "note": "Ready to execute"
        })).await?;
        edited.assert_succeeded()?;
        ensure!(edited.data()?["plan"]["steps"][0]["status"] == "in_progress");
        let conflict = user.call_tool("plan.edit", json!({
            "expected_revision": revision, "step": 1, "status": "completed"
        })).await?;
        conflict.assert_failed()?;
        let persisted = user.call_tool("plan.get", json!({"view": "full"})).await?;
        persisted.assert_succeeded()?;
        ensure!(persisted.data()?["plan"]["revision"] == edited.data()?["plan"]["revision"]);
        ensure!(persisted.data()?["plan"]["steps"][0]["status"] == "in_progress");

        let write = json!({"path": "plan-proof.txt", "content": "approved-plan-side-effect"});
        let blocked = user.call_tool("fs.write", write.clone()).await?;
        blocked.assert_failed()?;
        ensure!(!case.workspace().join("plan-proof.txt").exists(), "planning gate allowed a write");

        let review = user.start_tool("plan.review", json!({})).await?;
        let pending = review.run().wait_user_input().await?;
        ensure!(serde_json::to_string(&pending.request)?.contains("Approve"));
        pending.answer(BTreeMap::from([("0".to_owned(), vec!["Approve with autorun off".to_owned()])])).await?;
        let approved = review.wait_settled().await?;
        approved.assert_succeeded()?;
        ensure!(approved.data()?["phase"] == "active");
        let projection = user.client().session_plan(user.id(), true).await?;
        let plan = projection.payload.context("public session plan projection")?;
        ensure!(plan["plan"]["phase"] == "active" && plan["plan"]["autorun"] == false,
            "review was not projected to public session state: {plan}");

        user.call_tool("fs.write", write).await?.assert_succeeded()?;
        ensure!(tokio::fs::read_to_string(case.workspace().join("plan-proof.txt")).await? == "approved-plan-side-effect");
        let checked = user.call_tool("plan.edit", json!({"step": 1, "check": 1, "status": "completed"})).await?;
        checked.assert_succeeded()?;
        ensure!(checked.data()?["plan"]["steps"][0]["checks"][0]["status"] == "completed");
        let completed = user.call_tool("plan.phase", json!({
            "phase": "completed", "summary": "Verified the actual file"
        })).await?;
        completed.assert_succeeded()?;
        ensure!(completed.data()?["plan"]["phase"] == "completed");

        let cleared = user.call_tool("plan.clear", json!({})).await?;
        cleared.assert_succeeded()?;
        ensure!(cleared.data()?["cleared"] == true);
        let empty = user.call_tool("plan.get", json!({"view": "full"})).await?;
        empty.assert_succeeded()?;
        ensure!(empty.data()?["plan"].is_null());
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cron_persists_edits_and_delivers_a_real_scheduled_wake() -> TestResult {
    fixture().expect_plugin("agena.cron").run(|case| async move {
        let user = case.user().new_session().await?;
        allow_permissions(&user).await?;
        let created = user.call_tool("cron.create", json!({
            "expression": "0 0 0 1 1 *", "timezone": "UTC",
            "prompt": "dormant cron fixture", "max_age_days": 7
        })).await?;
        created.assert_succeeded()?;
        let id = created.data()?["id"].as_str().context("created cron id")?.to_owned();
        let listed = user.call_tool("cron.list", json!({})).await?;
        listed.assert_succeeded()?;
        let jobs = listed.data()?["jobs"].as_array().context("cron jobs")?;
        ensure!(jobs.len() == 1 && jobs[0]["id"] == id && jobs[0]["timezone"] == "UTC");

        let paused = user.call_tool("cron.pause", json!({"id": id})).await?;
        paused.assert_succeeded()?;
        ensure!(paused.data()?["job"]["paused"] == true);
        let updated = user.call_tool("cron.update", json!({
            "id": id, "prompt": "runtime-cron-delivery-proof",
            "misfire_policy": "run_once_now",
            "retry_policy": {"max_attempts": 2, "initial_delay_seconds": 1, "max_delay_seconds": 2, "multiplier": 2}
        })).await?;
        updated.assert_succeeded()?;
        ensure!(updated.data()?["job"]["prompt"] == "runtime-cron-delivery-proof");
        ensure!(updated.data()?["job"]["retry_max_attempts"] == 2);
        ensure!(updated.data()?["job"]["paused"] == true);
        let history = user.call_tool("cron.history", json!({"id": id, "limit": 10})).await?;
        history.assert_succeeded()?;
        ensure!(history.data()?["entries"].as_array().context("cron history")?.is_empty());

        let other = case.user().new_session().await?;
        allow_permissions(&other).await?;
        let isolated = other.call_tool("cron.list", json!({})).await?;
        isolated.assert_succeeded()?;
        ensure!(isolated.data()?["jobs"].as_array().context("isolated cron jobs")?.is_empty());

        let resumed = user.call_tool("cron.resume", json!({"id": id})).await?;
        resumed.assert_succeeded()?;
        ensure!(resumed.data()?["job"]["paused"] == false);

        // A real deadline, awaited via the provider request, not a fixed sleep.
        // Restrict this matcher to delivered system messages: update arguments
        // and tool output also contain the marker and must not impersonate a wake.
        let wake = case.model().expect(RequestMatch::new("cron delivery reaches the model", |request| {
            input_role_contains(request, "runtime-cron-delivery-proof")
        })).reply(Reply::text("runtime-cron-fired-complete")).with_timeout(Duration::from_secs(45));
        let at = Utc::now() + chrono::Duration::seconds(10);
        let expression = format!("{} {} {} {} {} *", at.second(), at.minute(), at.hour(), at.day(), at.month());
        let armed = user.call_tool("cron.update", json!({"id": id, "expression": expression})).await?;
        armed.assert_succeeded()?;
        ensure!(armed.data()?["job"]["next_fire_at"].as_str().is_some());
        wake.observed().await?;
        wake.finished().await?;
        user.wait_text_contains("runtime-cron-fired-complete").await?;

        let history = user.call_tool("cron.history", json!({"id": id, "limit": 10})).await?;
        history.assert_succeeded()?;
        let entries = history.data()?["entries"].as_array().context("fired cron history")?;
        ensure!(entries.len() == 1, "expected one real scheduled delivery: {entries:?}");
        ensure!(entries[0]["job_id"] == id && entries[0]["status"] == "submitted");
        ensure!(entries[0]["session_id"] == user.id());
        ensure!(entries[0]["delivery_key"].as_str().is_some_and(|key| !key.is_empty()));
        ensure!(user.parts().await?.iter().any(|part| part.kind == "system_notification"
            && part.content.to_string().contains("runtime-cron-delivery-proof")));

        user.call_tool("cron.pause", json!({"id": id})).await?.assert_succeeded()?;
        let deleted = user.call_tool("cron.delete", json!({"id": id})).await?;
        deleted.assert_succeeded()?;
        ensure!(deleted.data()?["removed"] == true);
        let empty = user.call_tool("cron.list", json!({})).await?;
        empty.assert_succeeded()?;
        ensure!(empty.data()?["jobs"].as_array().context("remaining cron jobs")?.is_empty());
        Ok(())
    }).await
}

fn input_role_contains(request: &ModelRequest, marker: &str) -> bool {
    request
        .body
        .get("input")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                matches!(
                    item.get("role").and_then(Value::as_str),
                    Some("system" | "developer")
                ) && item.to_string().contains(marker)
            })
        })
}

fn model_tool_succeeded(request: &ModelRequest, call_id: &str) -> Result<()> {
    let text = request
        .tool_output_text(call_id)
        .with_context(|| format!("missing tool output {call_id}"))?;
    let envelope: Value = serde_json::from_str(&text)?;
    ensure!(
        envelope["phase"] == "succeeded",
        "tool {call_id} failed: {envelope}"
    );
    Ok(())
}

// Read the product's structured payload, never parse a model preview as raw
// data. These fixtures intentionally have compact payloads with no field refs.
async fn raw_tool_payload(user: &UserSession, tool: &str) -> Result<Value> {
    let part = user
        .parts()
        .await?
        .into_iter()
        .rev()
        .find(|part| {
            part.kind == "tool_call"
                && part
                    .content
                    .get("tool_api_call")
                    .and_then(|call| {
                        serde_json::from_value::<agena_domain::ToolApiCall>(call.clone()).ok()
                    })
                    .map(|call| Value::from(call.arguments))
                    .is_some_and(|arguments| {
                        arguments.get("tool").and_then(Value::as_str) == Some(tool)
                    })
        })
        .with_context(|| format!("missing persisted {tool} part"))?;
    let detail = user
        .client()
        .session_tool_detail(user.id(), part.part_id, ToolDetailSection::Output)
        .await?;
    let raw: Option<RawOutput> = serde_json::from_value(detail.value)?;
    let raw = raw.with_context(|| format!("missing {tool} raw output"))?;
    ensure!(
        raw.fields.is_empty(),
        "compact {tool} fixture unexpectedly has referenced fields"
    );
    raw.payload
        .with_context(|| format!("missing {tool} payload"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_runs_messages_follows_up_and_cancels_a_real_child() -> TestResult {
    fixture()
        .expect_plugin("agena.agent")
        .run(|case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let model = case.model();
            let first = model
                .expect(RequestMatch::user_text("runtime-agent-parent-start"))
                .reply(Reply::tool_call(
                    "call_agent_spawn",
                    "agent.spawn",
                    json!({
                        "name": "worker", "prompt": "runtime-agent-child-start", "timeout_ms": 60000
                    }),
                ));
            let child_first = model
                .expect(RequestMatch::user_text("runtime-agent-child-start"))
                .reply(Reply::tool_call(
                    "call_child_wait",
                    "agent.wait",
                    json!({"timeout_ms": 30000}),
                ));
            let send_message = model
                .expect(RequestMatch::tool_output("call_agent_spawn"))
                .after(&first)
                .check(|request| model_tool_succeeded(request, "call_agent_spawn"))
                .reply(
                    Reply::tool_call(
                        "call_agent_message",
                        "agent.message",
                        json!({
                            "target": "worker", "message": "runtime-agent-message-proof"
                        }),
                    )
                    .held(),
                );
            let child_complete = model
                .expect(RequestMatch::tool_output("call_child_wait"))
                .after(&child_first)
                .check(|request| {
                    model_tool_succeeded(request, "call_child_wait")?;
                    ensure!(
                        request
                            .body
                            .to_string()
                            .contains("runtime-agent-message-proof")
                    );
                    Ok(())
                })
                .reply(Reply::text("runtime-agent-child-result").held());
            let parent_wait = model
                .expect(RequestMatch::tool_output("call_agent_message"))
                .after(&send_message)
                .check(|request| model_tool_succeeded(request, "call_agent_message"))
                .reply(Reply::tool_call(
                    "call_parent_wait",
                    "agent.wait",
                    json!({"timeout_ms": 30000}),
                ));
            let followup = model
                .expect(RequestMatch::tool_output("call_parent_wait"))
                .after(&parent_wait)
                .check(|request| {
                    model_tool_succeeded(request, "call_parent_wait")?;
                    ensure!(
                        request
                            .body
                            .to_string()
                            .contains("runtime-agent-child-result")
                    );
                    Ok(())
                })
                .reply(Reply::tool_call(
                    "call_agent_followup",
                    "agent.followup",
                    json!({
                        "target": "worker", "message": "runtime-agent-followup-start"
                    }),
                ));
            let child_followup = model
                .expect(RequestMatch::user_text("runtime-agent-followup-start"))
                .reply(Reply::text("cancelled-child-must-not-complete").held());
            let cancel = model
                .expect(RequestMatch::tool_output("call_agent_followup"))
                .after(&followup)
                .check(|request| model_tool_succeeded(request, "call_agent_followup"))
                .reply(
                    Reply::tool_call(
                        "call_agent_cancel",
                        "agent.cancel",
                        json!({"target": "worker"}),
                    )
                    .held(),
                );
            let drain = model
                .expect(RequestMatch::tool_output("call_agent_cancel"))
                .after(&cancel)
                .check(|request| model_tool_succeeded(request, "call_agent_cancel"))
                .reply(
                    Reply::tool_call("call_agent_drain", "agent.wait", json!({"timeout_ms": 1}))
                        .held(),
                );
            model
                .expect(RequestMatch::tool_output("call_agent_drain"))
                .after(&drain)
                .check(|request| model_tool_succeeded(request, "call_agent_drain"))
                .reply(Reply::text("runtime-agent-parent-complete"));

            let run = user.send("runtime-agent-parent-start").await?;
            send_message.observed().await?;
            let spawned = raw_tool_payload(&user, "agent.spawn").await?;
            ensure!(spawned["name"] == "worker" && spawned["status"] == "running");
            let child_id = spawned["agent_id"]
                .as_i64()
                .context("spawned child session id")?;
            let child = case.user().attach_session(child_id).await?;
            child_first.observed().await?;
            send_message.release();
            child_complete.observed().await?;
            ensure!(raw_tool_payload(&child, "agent.wait").await?["reason"] == "message");
            parent_wait.finished().await?;
            ensure!(raw_tool_payload(&user, "agent.message").await?["accepted"] == true);
            child_complete.release();
            child_followup.observed().await?;
            cancel.observed().await?;
            ensure!(raw_tool_payload(&user, "agent.wait").await?["reason"] == "message");
            ensure!(raw_tool_payload(&user, "agent.followup").await?["status"] == "running");
            cancel.release();
            drain.observed().await?;
            let cancelled = raw_tool_payload(&user, "agent.cancel").await?;
            ensure!(cancelled["accepted"] == true && cancelled["state"] == "cancelled");
            child_followup.disconnected().await?;
            let state = child
                .wait_state("cancelled child is durably published", |state| {
                    state.session.agent_status == Some(AgentStatus::Cancelled)
                })
                .await?;
            ensure!(state.session.parent_id == Some(user.id()) && state.session.is_subagent);
            ensure!(state.session.agent_name.as_deref() == Some("worker"));
            // finish_agent commits status and parent delivery together. Consume the
            // committed cancellation notice before the parent chooses to finish.
            drain.release();
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("runtime-agent-parent-complete")?;
            let drained = raw_tool_payload(&user, "agent.wait").await?;
            ensure!(matches!(
                drained["reason"].as_str(),
                Some("message" | "timeout")
            ));
            let child_text = child.assistant_text().await?;
            ensure!(child_text.contains("runtime-agent-child-result"));
            ensure!(!child_text.contains("cancelled-child-must-not-complete"));
            let parent_parts = user.parts().await?;
            // Operation ids in persisted system notifications are strings,
            // even when the originating session id is numeric. Compare the
            // wire value's actual type instead of comparing JSON String to
            // an i64 (which would silently return false for valid notices).
            let child_operation_id = child_id.to_string();
            for status in ["completed", "cancelled"] {
                ensure!(
                    parent_parts
                        .iter()
                        .any(|part| part.kind == "system_notification"
                            && part.content["operation_kind"] == "task"
                            && part.content["operation_id"].as_str()
                                == Some(child_operation_id.as_str())
                            && part.content["status"] == status),
                    "parent did not durably consume the child's {status} notification"
                );
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn monitor_delivers_websocket_events_and_stops_the_owned_connection() -> TestResult {
    fixture()
        .expect_plugin("agena.monitor")
        .run(|case| async move {
            // Own the WS transport explicitly: its accepted connection is joined by
            // the case, and cancellation closes it even when an assertion fails.
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
            let address = listener.local_addr()?;
            let connected = Arc::new(Semaphore::new(0));
            let emit = Arc::new(Semaphore::new(0));
            let closed = Arc::new(Semaphore::new(0));
            let connected_service = connected.clone();
            let emit_service = emit.clone();
            let closed_service = closed.clone();
            let stop = case.service_cancellation_token();
            case.spawn_service(async move {
                let connection = async {
                    let (stream, _) = listener.accept().await?;
                    let mut socket = tokio_tungstenite::accept_async(stream).await?;
                    connected_service.add_permits(1);
                    emit_service.acquire().await?.forget();
                    socket
                        .send(Message::Text("runtime-monitor-event-proof".into()))
                        .await?;
                    // Stopping the real subscription must close its actual socket.
                    while let Some(message) = socket.next().await {
                        match message {
                            Ok(Message::Close(_)) | Err(_) => break,
                            Ok(_) => {}
                        }
                    }
                    closed_service.add_permits(1);
                    Ok::<(), anyhow::Error>(())
                };
                tokio::select! {
                    result = connection => result,
                    () = stop.cancelled() => Ok(()),
                }
            });

            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let first = case
                .model()
                .expect(RequestMatch::user_text("runtime-monitor-start"))
                .reply(Reply::tool_call(
                    "call_monitor_start",
                    "monitor.start",
                    json!({
                        "ws": {"url": format!("ws://{address}/feed")},
                        "timeout_ms": 60000, "description": "case-owned monitor feed"
                    }),
                ));
            let started = case
                .model()
                .expect(RequestMatch::tool_output("call_monitor_start"))
                .after(&first)
                .reply(
                    Reply::tool_call(
                        "call_monitor_wait_event",
                        "agent.wait",
                        json!({"timeout_ms": 30000}),
                    )
                    .held(),
                );
            let run = user.send("runtime-monitor-start").await?;
            started.observed().await?;
            let payload = raw_tool_payload(&user, "monitor.start").await?;
            let monitor_id = payload["monitor_id"]
                .as_str()
                .context("started monitor id")?
                .to_owned();
            ensure!(
                payload["status"] == "running",
                "monitor did not start: {payload}"
            );

            let stopping = case
                .model()
                .expect(RequestMatch::tool_output("call_monitor_wait_event"))
                .after(&started)
                .check(|request| {
                    model_tool_succeeded(request, "call_monitor_wait_event")?;
                    ensure!(
                        request
                            .body
                            .to_string()
                            .contains("runtime-monitor-event-proof")
                    );
                    Ok(())
                })
                .reply(Reply::tool_call(
                    "call_monitor_stop",
                    "monitor.stop",
                    json!({"monitor_id": monitor_id}),
                ));
            let draining = case
                .model()
                .expect(RequestMatch::tool_output("call_monitor_stop"))
                .after(&stopping)
                .check(|request| {
                    let output: Value = serde_json::from_str(
                        &request
                            .tool_output_text("call_monitor_stop")
                            .context("monitor stop output")?,
                    )?;
                    ensure!(
                        output["phase"] == "succeeded",
                        "monitor stop failed: {output}"
                    );
                    Ok(())
                })
                .reply(
                    Reply::tool_call(
                        "call_monitor_wait_finish",
                        "agent.wait",
                        json!({"timeout_ms": 5000}),
                    )
                    .held(),
                );
            let finishing = case
                .model()
                .expect(RequestMatch::tool_output("call_monitor_wait_finish"))
                .after(&draining)
                .reply(Reply::text("runtime-monitor-complete").held());

            tokio::time::timeout(Duration::from_secs(10), connected.acquire())
                .await
                .context("monitor did not establish the real WebSocket")??
                .forget();
            emit.add_permits(1);
            started.release();
            draining.observed().await?;
            tokio::time::timeout(Duration::from_secs(10), closed.acquire())
                .await
                .context("monitor.stop did not close the real WebSocket")??
                .forget();
            let stopped = raw_tool_payload(&user, "monitor.stop").await?;
            ensure!(
                stopped["status"] == "stopped",
                "monitor stop was not persisted: {stopped}"
            );
            draining.release();
            finishing.observed().await?;
            let parts = user.parts().await?;
            ensure!(
                parts.iter().any(|part| part.kind == "system_notification"
                    && part.content["operation_kind"] == "monitor"
                    && part.content["operation_id"] == monitor_id
                    && part.content["body"]
                        .as_str()
                        .is_some_and(|text| text.contains("runtime-monitor-event-proof"))),
                "monitor event was not durably delivered"
            );
            ensure!(
                parts.iter().any(|part| part.kind == "system_notification"
                    && part.content["operation_kind"] == "monitor"
                    && part.content["operation_id"] == monitor_id
                    && part.content["status"] == "stopped"),
                "monitor final notification was not consumed before finishing"
            );
            finishing.release();
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("runtime-monitor-complete")?;
            ensure!(
                user.snapshot().await?.background_activities.is_empty(),
                "stopped monitor remains an active background member"
            );
            Ok(())
        })
        .await
}
