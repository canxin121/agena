use super::{client, post_json, request};
use crate::model::{ModelOptions, Reply, RequestMatch, ScriptedModel};
use anyhow::{Context, Result};
use serde_json::{Value, json};

#[tokio::test]
async fn long_requests_report_nearby_routing_conditions_without_header_secrets() -> Result<()> {
    let owner = ScriptedModel::start_with_options(ModelOptions {
        max_record_bytes: 1024,
        ..ModelOptions::default()
    })
    .await?;
    let model = owner.handle();
    for index in 0..12 {
        model
            .expect(RequestMatch::user_text(format!("unrelated-{index}")))
            .reply(Reply::text("not this request"));
    }
    model
        .expect(
            RequestMatch::user_text("late-user-marker")
                .streaming(true)
                .header("authorization", "expected-credential-canary"),
        )
        .named("current session authorization")
        .reply(Reply::text("must not be returned"));

    // Put instructions before input on the actual wire. The retained raw prefix
    // cannot contain the user marker and must not be the only routing evidence.
    let body = request("late-user-marker", true).to_string();
    let raw = format!(
        "{{\"instructions\":{},{}",
        json!("large-instruction-canary ".repeat(4096)),
        &body[1..]
    );
    let response = client()?
        .post(format!("{}/responses", owner.base_url()))
        .header("authorization", "actual-credential-canary")
        .header("content-type", "application/json")
        .body(raw)
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 400);
    let error: Value = response.json().await?;
    let message = error["error"]["message"]
        .as_str()
        .context("error message")?;
    assert!(
        message.contains("current session authorization"),
        "{message}"
    );
    assert!(message.contains("header \"authorization\""), "{message}");
    assert!(message.contains("late-user-marker"), "{message}");
    assert!(message.contains("more omitted"), "{message}");
    assert!(!message.contains("expected-credential-canary"), "{message}");
    assert!(!message.contains("actual-credential-canary"), "{message}");
    assert!(!message.contains("large-instruction-canary"), "{message}");
    assert!(message.len() <= 8195, "diagnostic exceeded its bound");
    let recorded = model.requests();
    assert!(recorded[0].body_truncated);
    assert!(recorded[0].body.is_null());
    assert!(!recorded[0].body_preview.contains("late-user-marker"));
    assert!(model.assert_finished().is_err());
    owner.shutdown().await
}

#[tokio::test]
async fn mismatches_name_model_stream_json_pointer_and_custom_conditions() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    for (name, matcher) in [
        (
            "model route",
            RequestMatch::user_text("route-marker").model("expected-model"),
        ),
        (
            "stream route",
            RequestMatch::user_text("route-marker").streaming(false),
        ),
        (
            "agent route",
            RequestMatch::user_text("route-marker")
                .json_pointer("/metadata/agent", json!("expected-agent")),
        ),
        (
            "custom route",
            RequestMatch::user_text("route-marker").and_named("child execution", |request| {
                request.body.pointer("/metadata/child") == Some(&json!(true))
            }),
        ),
    ] {
        model
            .expect(matcher)
            .named(name)
            .reply(Reply::text("never"));
    }
    let mut body = request("route-marker", true);
    body["metadata"] = json!({"agent":"actual-agent","child":false});
    let response = client()?
        .post(format!("{}/responses", owner.base_url()))
        .json(&body)
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 400);
    let failure = model.failed().await.to_string();
    for evidence in [
        "model route",
        "expected-model",
        "fake-model",
        "stream route",
        "expected false, actual true",
        "/metadata/agent",
        "expected-agent",
        "actual-agent",
        "child execution",
    ] {
        assert!(
            failure.contains(evidence),
            "missing {evidence:?}: {failure}"
        );
    }
    owner.shutdown().await
}

#[tokio::test]
async fn strict_tool_output_checks_reject_duplicate_items_before_reading_payloads() -> Result<()> {
    for duplicate in [
        json!({"type":"function_call_output","call_id":"duplicate-call","output":"wrong"}),
        json!({"type":"function_call_output","call_id":"duplicate-call"}),
    ] {
        for missing_first in [false, true] {
            let owner = ScriptedModel::start().await?;
            let model = owner.handle();
            model
                .expect(RequestMatch::tool_output("duplicate-call"))
                .check_tool_output_contains("duplicate-call", "expected-value")
                .reply(Reply::text("incorrectly accepted duplicate"));
            let valid = json!({
                "type":"function_call_output","call_id":"duplicate-call","output":"expected-value"
            });
            let mut body = request("duplicate result", false);
            let items = body["input"].as_array_mut().context("input items")?;
            if missing_first {
                // Reverse the items as well: neither first-match ordering nor a
                // missing output field changes the required cardinality.
                items.extend([duplicate.clone(), valid]);
            } else {
                items.extend([valid, duplicate.clone()]);
            }
            let response = client()?
                .post(format!("{}/responses", owner.base_url()))
                .json(&body)
                .send()
                .await?;
            assert_eq!(response.status().as_u16(), 400);
            let failure = model.failed().await.to_string();
            assert!(
                failure.contains("2 function_call_output items"),
                "{failure}"
            );
            assert!(failure.contains("duplicate-call"), "{failure}");
            assert!(failure.contains("expected exactly one"), "{failure}");
            assert!(model.assert_finished().is_err());
            owner.shutdown().await?;
        }
    }
    Ok(())
}

#[tokio::test]
async fn strict_tool_output_reads_keep_multimodal_text_and_reject_missing_output() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let accepted = model
        .expect(RequestMatch::tool_output("unique-call"))
        .check(|request| {
            assert!(request.require_tool_output("unique-call")?.is_array());
            request.assert_tool_output_contains("unique-call", "visible-result")
        })
        .reply(Reply::text("accepted multimodal output"));
    let mut body = request("one result", false);
    body["input"].as_array_mut().context("input")?.extend([
        json!({
            "type":"function_call_output","call_id":"another-call","output":"unrelated"
        }),
        json!({
            "type":"function_call_output","call_id":"unique-call",
            "output":[
                {"type":"input_text","text":"visible-result"},
                {"type":"input_image","image_url":"data:image/png;base64,AA=="}
            ]
        }),
    ]);
    let response = post_json(&client()?, &format!("{}/responses", owner.base_url()), body).await?;
    assert_eq!(
        response["output"][0]["content"][0]["text"],
        "accepted multimodal output"
    );
    accepted.finished().await?;
    model.assert_finished()?;
    owner.shutdown().await?;

    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    model
        .expect(RequestMatch::any())
        .check(|request| {
            request.require_tool_output("missing-output")?;
            Ok(())
        })
        .reply(Reply::text("must not be returned"));
    let mut body = request("missing result field", false);
    body["input"]
        .as_array_mut()
        .context("input")?
        .push(json!({"type":"function_call_output","call_id":"missing-output"}));
    let response = client()?
        .post(format!("{}/responses", owner.base_url()))
        .json(&body)
        .send()
        .await?;
    assert_eq!(response.status().as_u16(), 400);
    let failure = model.failed().await.to_string();
    assert!(failure.contains("has no output field"), "{failure}");
    owner.shutdown().await
}
