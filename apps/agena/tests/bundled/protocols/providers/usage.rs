use super::peer::*;
use agena_domain::{UsagePeriod, UsageTotals};
use agena_testkit::{TestResult, UserSession};
use anyhow::{Context, ensure};
use axum::http::Method;
use serde_json::json;

async fn usage(user: &UserSession, models: &[&str]) -> anyhow::Result<UsageTotals> {
    let stats = user
        .client()
        .usage_stats(
            UsagePeriod::AllTime,
            None,
            None,
            &["testkit".into()],
            &models.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
            &[],
            true,
            0,
        )
        .await?;
    // Both public accounting surfaces must read the same durable usage. For
    // concurrent aliases, either caller can own the one billable observation;
    // check every attributed session rather than guessing the winning caller.
    let mut sessions = std::collections::BTreeSet::from([user.id()]);
    sessions.extend(stats.by_session.iter().map(|row| row.session_id));
    for session_id in sessions {
        let cost = user.client().session_cost_summary(session_id).await?;
        let empty = UsageTotals::default();
        let expected = stats
            .by_session
            .iter()
            .find(|row| row.session_id == session_id)
            .map(|row| &row.totals)
            .unwrap_or(&empty);
        let mut observed = UsageTotals::default();
        for row in cost
            .by_model
            .iter()
            .filter(|row| row.provider_id == "testkit" && models.contains(&row.model_id.as_str()))
        {
            observed.input_tokens += row.input_tokens;
            observed.output_tokens += row.output_tokens;
            observed.requests += row.requests;
            observed.unpriced_requests += row.unpriced_requests;
            observed.total_cost_usd += row.total_cost_usd;
            observed.recorded_cost_usd += row.recorded_cost_usd;
            observed.estimated_cost_usd += row.estimated_cost_usd;
        }
        ensure!(
            observed.input_tokens == expected.input_tokens
                && observed.output_tokens == expected.output_tokens
                && observed.requests == expected.requests
                && observed.unpriced_requests == expected.unpriced_requests
                && (observed.total_cost_usd - expected.total_cost_usd).abs() < 1e-12
                && (observed.recorded_cost_usd - expected.recorded_cost_usd).abs() < 1e-12
                && (observed.estimated_cost_usd - expected.estimated_cost_usd).abs() < 1e-12,
            "session {session_id} cost differs from persisted usage: {observed:?} vs {expected:?}"
        );
    }
    Ok(stats.totals)
}
fn tokens(actual: &UsageTotals, input: u64, output: u64, requests: u64) -> TestResult {
    ensure!(
        actual.input_tokens == input
            && actual.output_tokens == output
            && actual.requests == requests,
        "wrong persisted billing totals: {actual:?}; wanted {input}/{output}/{requests}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_completion_and_aliases_record_billable_usage_once() -> TestResult {
    let peer = Peer::default();
    scripted(&peer)
        .run(move |case| async move {
            let user = session(&case).await?;
            let completed = json!({"id":"resp_paid","status":"completed","model":"paid-model",
            "usage":{"input_tokens":11,"output_tokens":7}});
            peer.json(
                Method::POST,
                "/v1/responses",
                json!({"id":"resp_paid","status":"queued"}),
            );
            let started = user
                .call_tool(
                    "chatgpt.inference",
                    json!({"prompt":"work","parameters":{"background":true}}),
                )
                .await?;
            started.assert_succeeded()?;
            ensure!(started.data()?["pending"] == true);
            tokens(&usage(&user, &["paid-model"]).await?, 0, 0, 0)?;
            for action in ["wait", "get"] {
                peer.json(Method::GET, "/v1/responses/resp_paid", completed.clone());
                user.call_tool(
                    "chatgpt.responses",
                    json!({"action":action,"id":"resp_paid"}),
                )
                .await?
                .assert_succeeded()?;
                tokens(&usage(&user, &["paid-model"]).await?, 11, 7, 1)?;
            }
            peer.assert_finished()
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incomplete_usage_can_arrive_later_and_concurrent_polls_do_not_double_bill() -> TestResult {
    let peer = Peer::default();
    scripted(&peer)
        .run(move |case| async move {
            let user = session(&case).await?;
            for (id, initial) in [
                ("resp_missing_usage", json!({"id":"resp_missing_usage"})),
                (
                    "resp_completed_no_usage",
                    json!({"id":"resp_completed_no_usage","status":"completed"}),
                ),
            ] {
                peer.json(Method::POST, "/v1/responses", initial);
                user.call_tool("chatgpt.inference", json!({"prompt":"work"}))
                    .await?
                    .assert_succeeded()?;
                peer.json(
                    Method::GET,
                    format!("/v1/responses/{id}"),
                    json!({"id":id,"status":"completed",
                "model":id,"usage":{"input_tokens":11,"output_tokens":7}}),
                );
                user.call_tool("chatgpt.responses", json!({"action":"get","id":id}))
                    .await?
                    .assert_succeeded()?;
                tokens(&usage(&user, &[id]).await?, 11, 7, 1)?;
            }
            let other = session(&case).await?;
            for _ in 0..2 {
                peer.json(
                    Method::GET,
                    "/v1/responses/resp_concurrent",
                    json!({"id":"resp_concurrent","status":"completed",
                "model":"concurrent-model","usage":{"input_tokens":5,"output_tokens":2}}),
                );
            }
            let args = json!({"action":"get","id":"resp_concurrent"});
            let (first, second) = tokio::join!(
                user.call_tool("chatgpt.responses", args.clone()),
                other.call_tool("chatgpt.responses", args),
            );
            first?.assert_succeeded()?;
            second?.assert_succeeded()?;
            tokens(&usage(&user, &["concurrent-model"]).await?, 5, 2, 1)?;
            peer.assert_finished()
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn batch_results_attribute_each_model_once_and_keep_downloads() -> TestResult {
    let peer = Peer::default();
    scripted(&peer).run(move |case| async move {
        let user = session(&case).await?;
        let bytes = [
            json!({"custom_id":"a","result":{"type":"succeeded","message":{"model":"claude-a","usage":{"input_tokens":10,"output_tokens":3}}}}),
            json!({"custom_id":"b","result":{"type":"succeeded","message":{"model":"claude-a","usage":{"input_tokens":20,"output_tokens":4}}}}),
            json!({"custom_id":"c","result":{"type":"succeeded","message":{"model":"claude-b","usage":{"input_tokens":30,"output_tokens":5}}}}),
        ].iter().map(serde_json::Value::to_string).collect::<Vec<_>>().join("\n").into_bytes();
        for _ in 0..2 {
            peer.expect(Method::GET, "/v1/messages/batches/batch1/results",
                ResponseSpec::bytes("application/x-ndjson", bytes.clone()));
            let result = user.call_tool("claude.batches", json!({"action":"results","id":"batch1"})).await?;
            result.assert_succeeded()?;
            ensure!(attachment_bytes(&user, &result, 0, "application/x-ndjson").await? == bytes);
            tokens(&usage(&user, &["claude-a"]).await?, 30, 7, 2)?;
            tokens(&usage(&user, &["claude-b"]).await?, 30, 5, 1)?;
        }
        let completed = json!({"name":"batches/batch1","done":true,"response":{"name":"batches/batch1",
            "model":"models/gemini-batch","output":{"inlinedResponses":{"inlinedResponses":[
                {"response":{"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":3}}},
                {"response":{"usageMetadata":{"promptTokenCount":20,"candidatesTokenCount":4}}}
            ]}}}});
        for id in ["batch1", "batches/batch1"] {
            peer.json(Method::GET, "/v1beta/batches/batch1", completed.clone());
            user.call_tool("gemini.batches", json!({"action":"get","id":id})).await?.assert_succeeded()?;
            // The model is attributed without its API resource prefix.
            tokens(&usage(&user, &["gemini-batch"]).await?, 30, 7, 2)?;
        }
        peer.assert_finished()
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_sse_preserves_cursors_usage_and_managed_session_identity() -> TestResult {
    let peer = Peer::default();
    scripted(&peer).run(move |case| async move {
        let user = session(&case).await?;
        let event = json!({"type":"response.completed","response":{"id":"r1","model":"stream-actual-model",
            "status":"completed","usage":{"input_tokens":9,"output_tokens":2}}});
        peer.expect(Method::POST, "/v1/responses", ResponseSpec::bytes("text/event-stream",
            format!("id: cursor-1\nevent: response.completed\nretry: 1200\ndata: {event}\n\n").into_bytes()));
        let result = user.call_tool("chatgpt.inference",
            json!({"prompt":"work","stream":true,"last_event_id":"cursor-0"})).await?;
        result.assert_succeeded()?;
        let request = peer.last()?;
        ensure!(request.headers["last-event-id"] == "cursor-0" && request.json()?["stream"] == true);
        let response = &result.data()?["response"]["response"];
        ensure!(response["last_event_id"] == "cursor-1", "{response}");
        ensure!(response["sse_events"][0]["event"] == "response.completed");
        ensure!(response["sse_events"][0]["retry"] == 1200);
        tokens(&usage(&user, &["stream-actual-model"]).await?, 9, 2, 1)?;

        peer.expect(Method::GET, "/v1/agents/sessions/session_test/events", ResponseSpec::bytes("text/event-stream",
            b"event: session.message\ndata: {\"type\":\"message\",\"text\":\"hello-session-events\"}\n\ndata: [DONE]\n\n".to_vec()));
        let events = user.call_tool("chatgpt.agents", json!({"action":"events","id":"session_test"})).await?;
        events.assert_succeeded()?;
        let encoded = events.data()?.to_string();
        ensure!(encoded.contains("hello-session-events") && encoded.contains("session_test") && encoded.contains("stream_ended"));
        peer.assert_finished()
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_poll_retains_pending_identity_without_resubmitting_paid_work() -> TestResult {
    let peer = Peer::default();
    scripted(&peer)
        .run(move |case| async move {
            let user = session(&case).await?;
            peer.json(
                Method::GET,
                "/v1/responses/resp_pending",
                json!({"id":"resp_pending","status":"in_progress"}),
            );
            peer.expect(
                Method::GET,
                "/v1/responses/resp_pending",
                ResponseSpec::json(json!({"error":{"message":"temporary poll failure"}}))
                    .status(503),
            );
            let result = user
                .call_tool(
                    "chatgpt.responses",
                    // The production poller pauses for up to two seconds between GETs.
                    // Keep enough deadline for the scripted HTTP failure to be
                    // observed; a deadline warning would exercise another branch.
                    json!({"action":"wait","id":"resp_pending","wait_seconds":20}),
                )
                .await?;
            result.assert_succeeded()?;
            ensure!(result.data()?["pending"] == true, "{}", result.data()?);
            let output = result.data()?;
            ensure!(output["resource_id"] == "resp_pending", "{output}");
            let warning = output["response"]["poll_warning"]
                .as_str()
                .context("pending poll result omitted its warning")?;
            ensure!(
                warning.contains("HTTP 503") && warning.contains("temporary poll failure"),
                "the scripted failed GET was not observed: {warning}"
            );
            let requests = peer.requests();
            ensure!(
                requests.len() == 2
                    && requests.iter().all(|r| {
                        r.method == Method::GET && r.target == "/v1/responses/resp_pending"
                    }),
                "polling changed the pending resource or resubmitted work: {requests:?}",
            );
            peer.assert_finished()
        })
        .await
}
