use std::time::Duration;

use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{ModelOptions, Reply, RequestMatch, ScriptedModel, Usage, lock};

mod diagnostics;
mod disconnect;
mod protocols;
mod query_routes;
mod text_gates;

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .http1_only()
        .timeout(Duration::from_secs(5))
        .build()
        .context("build loopback test client")
}

fn request(text: &str, streaming: bool) -> Value {
    json!({
        "model": "fake-model",
        "stream": streaming,
        "input": [{"role": "user", "content": [{"type": "input_text", "text": text}]}],
        "tools": [{"type": "function", "name": "tools_call", "parameters": {"type": "object"}}]
    })
}

fn events(body: &str) -> Result<Vec<Value>> {
    body.split("\n\n")
        .filter(|frame| !frame.trim().is_empty())
        .map(|frame| {
            serde_json::from_str(
                frame
                    .strip_prefix("data: ")
                    .context("missing SSE data prefix")?,
            )
            .context("parse scripted SSE event")
        })
        .collect()
}

async fn post_json(client: &reqwest::Client, url: &str, body: Value) -> Result<Value> {
    Ok(client
        .post(url)
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

#[tokio::test]
async fn independent_requests_and_causal_followups_do_not_use_a_global_queue() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let client = client()?;
    let url = format!("{}/responses", owner.base_url());
    let first_a = model
        .expect(RequestMatch::user_text("a:first"))
        .reply(Reply::text("answer a"));
    let first_b = model
        .expect(RequestMatch::user_text("b:first"))
        .reply(Reply::text("answer b"));
    let next_a = model
        .expect(RequestMatch::user_text("a:second"))
        .after(&first_a)
        .reply(Reply::text("answer a2"));

    let catalog: Value = client
        .get(format!("{}/models", owner.base_url()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(catalog["data"][0]["id"], "fake-model");
    assert!(
        model.requests().is_empty(),
        "model discovery must not consume a scripted step"
    );

    let (b, a) = tokio::join!(
        post_json(&client, &url, request("b:first", false)),
        post_json(&client, &url, request("a:first", false)),
    );
    assert_eq!(a?["output"][0]["content"][0]["text"], "answer a");
    assert_eq!(b?["output"][0]["content"][0]["text"], "answer b");
    first_a.finished().await?;
    first_b.finished().await?;
    let second = post_json(&client, &url, request("a:second", false)).await?;
    assert_eq!(second["output"][0]["content"][0]["text"], "answer a2");
    next_a.finished().await?;
    assert_eq!(first_a.observed().await?.user_text(), "a:first");
    assert_eq!(first_b.observed().await?.user_text(), "b:first");
    model.assert_finished()?;
    owner.shutdown().await
}

#[tokio::test]
async fn streamed_tool_calls_keep_item_and_call_ids_distinct_across_fragments() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let client = client()?;
    let arguments = json!({"tool": "test.echo", "input": {"message": "中文"}}).to_string();
    let split = arguments.find("中文").context("argument marker")?;
    let step = model
        .expect(
            RequestMatch::user_text("wire")
                .streaming(true)
                .model("fake-model")
                .declares_function("tools_call")
                .header("x-test-case", "wire"),
        )
        .reply(
            // Raw wire assertions: this reply must stay one frame set, so it
            // uses the explicit call form instead of the auto-help helper.
            Reply::function_call_raw(
                "call_original",
                "tools_call",
                json!({"tool": "test.echo", "input": {"message": "中文"}}).to_string(),
            )
            .arguments_chunks([arguments[..split].to_owned(), arguments[split..].to_owned()])
            .with_thinking_chunks(["checking"])
            .usage(Usage::tokens(11, 7).cached(3).reasoning(2))
            .end_turn(false)
            .fragment_bytes(7),
        );
    let response = client
        .post(format!("{}/responses", owner.base_url()))
        .header("x-test-case", "wire")
        .json(&request("wire", true))
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let frames = events(&response.text().await?)?;
    let added = frames
        .iter()
        .find(|event| event["type"] == "response.output_item.added")
        .context("function item added")?;
    assert_eq!(added["item"]["call_id"], "call_original");
    assert_ne!(added["item"]["id"], added["item"]["call_id"]);
    let deltas = frames
        .iter()
        .filter(|event| event["type"] == "response.function_call_arguments.delta")
        .collect::<Vec<_>>();
    assert_eq!(deltas.len(), 2);
    for delta in &deltas {
        assert_eq!(delta["item_id"], added["item"]["id"]);
        assert!(delta.get("call_id").is_none());
    }
    let joined: String = deltas
        .iter()
        .map(|event| event["delta"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(joined, arguments);
    let terminal = frames.last().context("terminal response")?;
    assert_eq!(terminal["type"], "response.completed");
    assert_eq!(terminal["response"]["usage"]["input_tokens"], 11);
    assert_eq!(
        terminal["response"]["usage"]["output_tokens_details"]["reasoning_tokens"],
        2
    );
    assert_eq!(terminal["response"]["end_turn"], false);
    assert_eq!(
        terminal["response"]["output"][1]["call_id"],
        "call_original"
    );
    step.finished().await?;
    model.assert_finished()?;
    owner.shutdown().await
}

#[tokio::test]
async fn nonstream_approval_replies_are_real_function_calls() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let client = client()?;
    let approve = model
        .expect(RequestMatch::declares_function(
            RequestMatch::any(),
            "approve_action",
        ))
        .reply(Reply::approve("a safe fixture action").end_turn(true));
    let block = model
        .expect(RequestMatch::any().declares_function("block_action"))
        .reply(Reply::block("fs.read:/denied/**", "denied fixture path"));
    let mut allow_request = request("allow", false);
    allow_request["tools"] = json!([{"type": "function", "name": "approve_action"}]);
    let allowed = post_json(
        &client,
        &format!("{}/responses", owner.base_url()),
        allow_request,
    )
    .await?;
    assert_eq!(allowed["output"][0]["type"], "function_call");
    assert_eq!(allowed["output"][0]["name"], "approve_action");
    let args: Value = serde_json::from_str(
        allowed["output"][0]["arguments"]
            .as_str()
            .context("arguments")?,
    )?;
    assert_eq!(args["reason"], "a safe fixture action");
    assert_eq!(allowed["end_turn"], true);
    assert_ne!(allowed["output"][0]["id"], allowed["output"][0]["call_id"]);

    let mut deny_request = request("deny", false);
    deny_request["tools"] = json!([{"type": "function", "name": "block_action"}]);
    let denied = post_json(
        &client,
        &format!("{}/responses", owner.base_url()),
        deny_request,
    )
    .await?;
    assert_eq!(denied["output"][0]["name"], "block_action");
    let args: Value = serde_json::from_str(
        denied["output"][0]["arguments"]
            .as_str()
            .context("arguments")?,
    )?;
    assert_eq!(args["rule"], "fs.read:/denied/**");
    approve.finished().await?;
    block.finished().await?;
    model.assert_finished()?;
    owner.shutdown().await
}

#[tokio::test]
async fn tool_result_checks_are_correlated_by_call_id_not_prompt_text() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    model
        .expect(RequestMatch::tool_output("call_wanted"))
        .check_tool_output_contains("call_wanted", "expected-value")
        .reply(Reply::text("must not be delivered"));
    let mut body = request("expected-value appears in user text", false);
    body["input"].as_array_mut().context("input")?.extend([
        json!({"type": "function_call_output", "call_id": "another_call", "output": "expected-value"}),
        json!({"type": "function_call_output", "call_id": "call_wanted", "output": "actual-wrong-value"}),
    ]);
    let response = client()?
        .post(format!("{}/responses", owner.base_url()))
        .json(&body)
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 400);
    let failure = tokio::time::timeout(Duration::from_secs(1), model.failed()).await?;
    assert!(failure.to_string().contains("actual-wrong-value"));
    assert!(failure.to_string().contains("call_wanted"));
    assert!(model.assert_finished().is_err());
    assert_eq!(
        model.requests()[0]
            .tool_output_text("call_wanted")
            .as_deref(),
        Some("actual-wrong-value")
    );
    // A later subscriber also immediately sees the persistent failure.
    let repeated = tokio::time::timeout(Duration::from_secs(1), model.failure_notified()).await?;
    assert!(repeated.to_string().contains("check failed"));
    owner.shutdown().await
}

#[tokio::test]
async fn released_stream_and_observed_dependencies_are_causal_without_sleeps() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let first = model
        .expect(RequestMatch::user_text("held"))
        .reply(Reply::text_chunks(["first", "second"]).hold_after_chunks(1));
    let next = model
        .expect(RequestMatch::tool_output("call_result"))
        .after_observed(&first)
        .check_tool_output_contains("call_result", "correct-value")
        .reply(Reply::text("checked").held());
    // Release before request arrival is deliberately supported.
    next.release();
    let client = client()?;
    let mut response = client
        .post(format!("{}/responses", owner.base_url()))
        .json(&request("held", true))
        .send()
        .await?
        .error_for_status()?
        .bytes_stream();
    first.observed().await?;
    let bytes = response.next().await.context("first text frame")??;
    assert!(String::from_utf8_lossy(&bytes).contains("first"));
    assert_eq!(first.chunks_sent(), 1);
    assert!(
        first
            .clone()
            .with_timeout(Duration::from_millis(20))
            .finished()
            .await
            .is_err()
    );

    let mut second = request("result", false);
    second["input"]
        .as_array_mut()
        .context("input")?
        .push(json!({
            "type": "function_call_output", "call_id": "call_result",
            "output": [{"type": "input_text", "text": "correct-value"}]
        }));
    let checked = post_json(&client, &format!("{}/responses", owner.base_url()), second).await?;
    assert_eq!(checked["output"][0]["content"][0]["text"], "checked");
    next.finished().await?;
    first.release();
    let mut remaining = Vec::new();
    while let Some(chunk) = response.next().await {
        remaining.extend_from_slice(&chunk?);
    }
    assert!(String::from_utf8_lossy(&remaining).contains("second"));
    first.finished().await?;
    model.assert_finished()?;
    owner.shutdown().await
}

#[tokio::test]
async fn cancelling_a_held_body_is_observable_and_shutdown_joins_tasks() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let first = model
        .expect(RequestMatch::user_text("cancel"))
        .reply(Reply::text_chunks(["visible", "unreleased"]).hold_after_chunks(1));
    let mut response = client()?
        .post(format!("{}/responses", owner.base_url()))
        .json(&request("cancel", true))
        .send()
        .await?
        .error_for_status()?
        .bytes_stream();
    let first_chunk = response.next().await.context("first delta")??;
    assert!(String::from_utf8_lossy(&first_chunk).contains("visible"));
    drop(response);
    first.disconnected().await?;
    assert_eq!(first.chunks_sent(), 1);
    model.assert_finished()?;
    let address = owner
        .base_url()
        .trim_start_matches("http://")
        .trim_end_matches("/v1")
        .to_owned();
    owner.shutdown().await?;
    assert!(model.state.tasks.is_empty());
    assert!(lock(&model.state.aborts).is_empty());
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    Ok(())
}

#[tokio::test]
async fn unexpected_and_ambiguous_requests_fail_instead_of_returning_canned_success() -> Result<()>
{
    for ambiguous in [false, true] {
        let owner = ScriptedModel::start().await?;
        let model = owner.handle();
        if ambiguous {
            model
                .expect(RequestMatch::user_text("same"))
                .named("one")
                .reply(Reply::text("one"));
            model
                .expect(RequestMatch::user_text("same"))
                .named("two")
                .reply(Reply::text("two"));
        }
        let response = client()?
            .post(format!("{}/responses", owner.base_url()))
            .json(&request("same", false))
            .send()
            .await?;
        assert_eq!(response.status().as_u16(), 400);
        let failure = tokio::time::timeout(Duration::from_secs(1), model.failed()).await?;
        assert!(
            failure
                .to_string()
                .contains(if ambiguous { "ambiguous" } else { "unexpected" })
        );
        assert_eq!(model.requests().len(), 1);
        assert!(model.requests()[0].step_id.is_none());
        owner.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn a_step_cannot_be_consumed_twice_even_by_concurrent_requests() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let step = model
        .expect(RequestMatch::user_text("duplicate"))
        .reply(Reply::text("one response"));
    let client = client()?;
    let url = format!("{}/responses", owner.base_url());
    let send = || client.post(&url).json(&request("duplicate", false)).send();
    let (left, right) = tokio::join!(send(), send());
    let statuses = [left?.status().as_u16(), right?.status().as_u16()];
    assert_eq!(statuses.iter().filter(|&&status| status == 200).count(), 1);
    assert_eq!(statuses.iter().filter(|&&status| status == 400).count(), 1);
    let records = model.requests();
    assert_eq!(
        records
            .iter()
            .filter(|record| record.step_id == Some(step.id()))
            .count(),
        1
    );
    assert!(model.failed().await.to_string().contains("unexpected"));
    owner.shutdown().await
}

#[tokio::test]
async fn after_requires_response_completion_not_just_request_arrival() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let first = model
        .expect(RequestMatch::user_text("first"))
        .reply(Reply::text_chunks(["visible", "still held"]).hold_after_chunks(1));
    model
        .expect(RequestMatch::user_text("premature"))
        .after(&first)
        .reply(Reply::text("not yet"));
    let client = client()?;
    let response = client
        .post(format!("{}/responses", owner.base_url()))
        .json(&request("first", true))
        .send()
        .await?;
    first.observed().await?;
    let next = client
        .post(format!("{}/responses", owner.base_url()))
        .json(&request("premature", false))
        .send()
        .await?;
    assert_eq!(next.status().as_u16(), 400);
    let failure = model.failed().await.to_string();
    assert!(failure.contains("blocked"));
    assert!(failure.contains("waits for"));
    drop(response);
    owner.shutdown().await
}

#[tokio::test]
async fn full_checks_survive_bounded_record_retention() -> Result<()> {
    let owner = ScriptedModel::start_with_options(ModelOptions {
        max_record_bytes: 32,
        max_recorded_requests: 2,
        ..ModelOptions::default()
    })
    .await?;
    let model = owner.handle();
    let client = client()?;
    for index in 0..3 {
        let marker = format!("case-{index}");
        let expected = format!("{marker} {}", "x".repeat(256));
        let expected_check = expected.clone();
        let step = model
            .expect(RequestMatch::user_text(marker))
            .check(move |actual| {
                ensure!(
                    actual.user_text() == expected_check,
                    "checks must see the complete body"
                );
                ensure!(!actual.body_truncated, "live body must not be truncated");
                Ok(())
            })
            .reply(Reply::text("ok"));
        post_json(
            &client,
            &format!("{}/responses", owner.base_url()),
            request(&expected, false),
        )
        .await?;
        step.finished().await?;
        assert!(step.observed().await?.body_truncated);
    }
    let records = model.requests();
    assert_eq!(records.len(), 2);
    assert_eq!(model.dropped_request_records(), 1);
    assert_eq!(records[0].sequence, 2);
    assert!(
        records
            .iter()
            .all(|record| record.body.is_null() && record.body_preview.len() <= 35)
    );
    model.assert_finished()?;
    owner.shutdown().await
}

#[tokio::test]
async fn raw_status_headers_and_unterminated_sse_are_preserved() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let error_step = model.expect(RequestMatch::user_text("retry")).reply(
        Reply::raw_json(429, json!({"error": {"message": "try again"}})).header("retry-after", "0"),
    );
    let stream_step = model
        .expect(RequestMatch::user_text("truncated"))
        .reply(Reply::raw_stream(
            200,
            ["data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n"],
        ));
    let client = client()?;
    let error = client
        .post(format!("{}/responses", owner.base_url()))
        .json(&request("retry", false))
        .send()
        .await?;
    assert_eq!(error.status().as_u16(), 429);
    assert_eq!(error.headers()["retry-after"], "0");
    assert_eq!(
        error.json::<Value>().await?["error"]["message"],
        "try again"
    );
    let truncated = client
        .post(format!("{}/responses", owner.base_url()))
        .json(&request("truncated", true))
        .send()
        .await?
        .text()
        .await?;
    let frames = events(&truncated)?;
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["delta"], "partial");
    assert!(
        !truncated.contains("completed"),
        "raw replies must not gain a synthetic terminal event"
    );
    error_step.finished().await?;
    stream_step.finished().await?;
    model.assert_finished()?;
    owner.shutdown().await
}

#[tokio::test]
async fn shutdown_closes_incomplete_request_uploads_and_releases_listener() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let address = owner
        .base_url()
        .trim_start_matches("http://")
        .trim_end_matches("/v1")
        .to_owned();
    let mut upload = tokio::net::TcpStream::connect(&address).await?;
    upload.write_all(
        b"POST /v1/responses HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 500\r\n\r\n{"
    ).await?;
    // A completed request proves the accept loop is active; no timing sleeps.
    client()?
        .get(format!("{}/models", owner.base_url()))
        .send()
        .await?
        .error_for_status()?;
    owner.shutdown().await?;
    assert!(model.state.tasks.is_empty());
    assert!(lock(&model.state.aborts).is_empty());
    let mut byte = [0_u8; 1];
    match tokio::time::timeout(Duration::from_secs(1), upload.read(&mut byte)).await? {
        Ok(0) | Err(_) => {}
        Ok(_) => anyhow::bail!("shutdown left the unfinished upload connected"),
    }
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    Ok(())
}

#[tokio::test]
async fn user_predicate_and_check_panics_become_fixture_failures() -> Result<()> {
    for predicate_panic in [false, true] {
        let owner = ScriptedModel::start().await?;
        let model = owner.handle();
        let matcher = if predicate_panic {
            RequestMatch::new("panic matcher", |_| panic!("intentional matcher failure"))
        } else {
            RequestMatch::any()
        };
        let expectation = model.expect(matcher);
        let expectation = if predicate_panic {
            expectation
        } else {
            expectation.check(|_| panic!("intentional check failure"))
        };
        expectation.reply(Reply::text("never"));
        let response = client()?
            .post(format!("{}/responses", owner.base_url()))
            .json(&request("panic", false))
            .send()
            .await?;
        assert_eq!(response.status().as_u16(), 400);
        let failure = tokio::time::timeout(Duration::from_secs(1), model.failed())
            .await?
            .to_string();
        assert!(failure.contains("panicked"));
        assert!(failure.contains(if predicate_panic {
            "intentional matcher"
        } else {
            "intentional check"
        }));
        owner.shutdown().await?;
    }
    Ok(())
}
