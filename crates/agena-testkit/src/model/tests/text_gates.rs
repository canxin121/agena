use super::{client, events, request};
use crate::model::{Reply, RequestMatch, ScriptedModel};
use anyhow::{Context, Result};
use bytes::Bytes;
use futures_core::Stream;
use futures_util::StreamExt;
use serde_json::json;
use std::time::Duration;

async fn read_event(
    stream: &mut (impl Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Unpin),
    event_type: &str,
    delta: &str,
) -> Result<Vec<u8>> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut body = Vec::new();
        loop {
            body.extend_from_slice(&stream.next().await.context("stream ended before event")??);
            if body.ends_with(b"\n\n") {
                let frames = events(std::str::from_utf8(&body)?)?;
                if frames
                    .iter()
                    .any(|frame| frame["type"] == event_type && frame["delta"] == delta)
                {
                    return Ok(body);
                }
            }
        }
    })
    .await
    .context("text gate stopped before the complete expected event")?
}

#[tokio::test]
async fn text_gate_survives_thinking_and_byte_fragments_before_release() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let step = model
        .expect(RequestMatch::user_text("semantic-gate"))
        .reply(
            Reply::text_chunks(["可见第一段", "must-wait"])
                .with_thinking_chunks(["分析一", "analysis-two"])
                .with_tool_call("after-text", "fs.read", json!({"path":"note.txt"}))
                .fragment_bytes(3)
                .hold_after_text_chunks(1),
        );
    let mut response = client()?
        .post(format!("{}/responses", owner.base_url()))
        .json(&request("semantic-gate", true))
        .send()
        .await?
        .error_for_status()?
        .bytes_stream();
    let mut body = read_event(&mut response, "response.output_text.delta", "可见第一段").await?;
    let frames = events(std::str::from_utf8(&body)?)?;
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame["type"] == "response.reasoning_summary_text.delta")
            .count(),
        2
    );
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame["type"] == "response.output_text.delta")
            .count(),
        1
    );
    assert!(!std::str::from_utf8(&body)?.contains("must-wait"));
    assert!(
        step.chunks_sent() > frames.len(),
        "byte fragmentation was not exercised"
    );
    assert!(
        step.clone()
            .with_timeout(Duration::from_millis(20))
            .finished()
            .await
            .is_err(),
        "text gate completed before release"
    );
    step.release();
    while let Some(chunk) = response.next().await {
        body.extend_from_slice(&chunk?);
    }
    let frames = events(std::str::from_utf8(&body)?)?;
    let terminal = frames.last().context("terminal event")?;
    assert_eq!(terminal["type"], "response.completed");
    assert_eq!(
        terminal["response"]["output"][1]["content"][0]["text"],
        "可见第一段must-wait"
    );
    assert_eq!(terminal["response"]["output"][2]["call_id"], "after-text");
    step.finished().await?;
    model.assert_finished()?;
    owner.shutdown().await
}

#[tokio::test]
async fn text_gate_disconnect_retains_a_complete_visible_utf8_delta() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let step = model
        .expect(RequestMatch::user_text("cancel-semantic-gate"))
        .reply(
            Reply::text_chunks(["可取消", "hidden"])
                .with_thinking_chunks(["thought"])
                .fragment_bytes(1)
                .hold_after_text_chunks(1),
        );
    let mut response = client()?
        .post(format!("{}/responses", owner.base_url()))
        .json(&request("cancel-semantic-gate", true))
        .send()
        .await?
        .error_for_status()?
        .bytes_stream();
    let body = read_event(&mut response, "response.output_text.delta", "可取消").await?;
    assert!(!std::str::from_utf8(&body)?.contains("hidden"));
    drop(response);
    step.disconnected().await?;
    model.assert_finished()?;
    owner.shutdown().await
}

#[tokio::test]
async fn zero_and_last_text_gates_preserve_reasoning_and_terminal_boundaries() -> Result<()> {
    for count in [0, 1] {
        let owner = ScriptedModel::start().await?;
        let model = owner.handle();
        let step = model.expect(RequestMatch::user_text("boundary")).reply(
            Reply::text("only text")
                .with_thinking_chunks(["reasoning first"])
                .hold_after_text_chunks(count)
                .fragment_bytes(5),
        );
        let mut response = client()?
            .post(format!("{}/responses", owner.base_url()))
            .json(&request("boundary", true))
            .send()
            .await?
            .error_for_status()?
            .bytes_stream();
        let (kind, delta) = if count == 0 {
            ("response.reasoning_summary_text.delta", "reasoning first")
        } else {
            ("response.output_text.delta", "only text")
        };
        let mut body = read_event(&mut response, kind, delta).await?;
        let frames = events(std::str::from_utf8(&body)?)?;
        assert_eq!(
            frames
                .iter()
                .filter(|frame| frame["type"] == "response.output_text.delta")
                .count(),
            count
        );
        assert!(
            frames
                .iter()
                .all(|frame| frame["type"] != "response.completed"),
            "terminal snapshot passed the text gate"
        );
        step.release();
        while let Some(chunk) = response.next().await {
            body.extend_from_slice(&chunk?);
        }
        assert_eq!(
            events(std::str::from_utf8(&body)?)?
                .last()
                .context("terminal event")?["type"],
            "response.completed"
        );
        step.finished().await?;
        model.assert_finished()?;
        owner.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn invalid_text_gate_combinations_fail_on_the_real_http_request() -> Result<()> {
    for (reply, streaming, reason) in [
        (
            Reply::raw_json(200, json!({})).hold_after_text_chunks(0),
            true,
            "requires a completion reply",
        ),
        (
            Reply::raw_stream(200, ["data: {}\n\n"]).hold_after_text_chunks(1),
            true,
            "requires a completion reply",
        ),
        (
            Reply::text("one").hold_after_text_chunks(1),
            false,
            "requires a streaming request",
        ),
        (
            Reply::tool_call("no-text", "fs.read", json!({"path":"a"})).hold_after_text_chunks(0),
            true,
            "requires at least one text chunk",
        ),
        (
            Reply::text("one").hold_after_text_chunks(2),
            true,
            "exceeds the 1 text chunks",
        ),
        (
            Reply::text("one").held().hold_after_text_chunks(1),
            true,
            "cannot be combined",
        ),
        (
            Reply::text("one").hold_after_text_chunks(1).held(),
            true,
            "cannot be combined",
        ),
        (
            Reply::text("one").hold_after_text_chunks(1).status(204),
            true,
            "bodyless HTTP response",
        ),
    ] {
        let owner = ScriptedModel::start().await?;
        let model = owner.handle();
        model.expect(RequestMatch::any()).reply(reply);
        let response = client()?
            .post(format!("{}/responses", owner.base_url()))
            .json(&request("invalid text gate", streaming))
            .send()
            .await?;
        assert_eq!(response.status().as_u16(), 500);
        let failure = model.failed().await.to_string();
        assert!(failure.contains(reason), "{failure}");
        assert!(model.assert_finished().is_err());
        owner.shutdown().await?;
    }
    Ok(())
}
