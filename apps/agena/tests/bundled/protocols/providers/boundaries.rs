use agena_failure::{FailureCategory, FailureImpact, RetryDirective};
use agena_testkit::{EditOptions, ModelFeedbackKind, SettingsLayer, TestResult};
use anyhow::{Context, ensure};
use axum::http::Method;
use serde_json::{Value, json};
use std::time::Duration;

use super::{API_KEY, Vendor, peer::*};

async fn permissions(user: &agena_testkit::UserSession, rules: Value, network: &str) -> TestResult {
    user.client().set_session_permission(user.id(), serde_json::from_value(json!({
        "path": {"workspace":{"read":"allow","write":"allow"},
            "external":{"read":"allow","write":"allow"}, "rules":rules},
        "network":{"loopback":network,"private":"deny","internet":"deny"},
        "tools":{"default":"allow","names":{"chatgpt.files":"allow","agena.chatgpt.files":"allow"}}
    }))?).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_provider_actions_never_reappear_as_authoritative_text() -> TestResult {
    let peer = Peer::default();
    scripted(&peer).run(move |case| async move {
        let user = session(&case).await?;
        let marker = "blocked-provider-text-canary";
        for (vendor, response) in [
            (Vendor::ChatGpt, json!({"id":"resp_blocked","status":"completed","output":[
                {"type":"message","role":"assistant","content":[{"type":"output_text","text":marker}]},
                {"type":"function_call","name":"local_action","call_id":"call_blocked","arguments":"{}"}]})),
            (Vendor::Claude, json!({"id":"msg_blocked","stop_reason":"tool_use","content":[
                {"type":"text","text":marker},{"type":"tool_use","id":"call_blocked","name":"local_action","input":{}}]})),
            (Vendor::Gemini, json!({"id":"interaction_blocked","status":"completed","outputs":[
                {"type":"text","text":marker},{"type":"function_call","name":"local_action","id":"call_blocked","arguments":{}}]})),
        ] {
            peer.json(Method::POST, vendor.path(), response);
            let result = user.call_tool(vendor.tool(), json!({"prompt":"boundary-check"})).await?;
            result.assert_succeeded()?;
            let data = result.data()?;
            ensure!(data["outcome"] == "blocked_client_execution" && data["local_actions_executed"] == false);
            ensure!(data["text"].as_str().context("durable text")?.contains("Agena blocked that execution boundary"));
            ensure!(!result.output.to_string().contains(marker), "blocked text reached the AI");
            ensure!(!serde_json::to_string(&result.raw_output)?.contains(marker), "blocked text reached durable output");
            let receipt = tokio::fs::read_to_string(data["response_receipt"]["path"].as_str().context("receipt")?).await?;
            ensure!(receipt.contains(marker), "receipt lost original boundary evidence");
        }
        peer.assert_finished()
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unicode_provider_text_is_bounded_but_receipt_bytes_remain_complete() -> TestResult {
    let peer = Peer::default();
    scripted(&peer).run(move |case| async move {
        let user = session(&case).await?;
        let tail = "provider-text-beyond-preview-limit";
        let text = format!("provider-text-prefix:{}:{tail}", "正文".repeat(20_000));
        for vendor in [Vendor::ChatGpt, Vendor::Gemini] {
            let response = if matches!(vendor, Vendor::ChatGpt) {
                json!({"id":"resp_long","status":"completed","output":[{"type":"message","role":"assistant",
                    "content":[{"type":"output_text","text":text}]}]})
            } else {
                json!({"id":"interaction_long","status":"completed","outputs":[{"type":"text","text":text}]})
            };
            peer.json(Method::POST, vendor.path(), response);
            let result = user.call_tool(vendor.tool(), json!({"prompt":"bounded-text"})).await?;
            result.assert_succeeded()?;
            let data = result.data()?;
            let projected = data["text"].as_str().context("durable text")?;
            ensure!(projected.starts_with("provider-text-prefix:"));
            ensure!(projected.len() <= 32 * 1024 + "\n…[truncated]".len());
            ensure!(projected.ends_with("\n…[truncated]") && !projected.contains(tail));
            ensure!(!result.output.to_string().contains(tail), "unbounded provider text reached the AI");
            let receipt = tokio::fs::read_to_string(data["response_receipt"]["path"].as_str().context("receipt")?).await?;
            ensure!(receipt.contains(tail) && receipt.contains(&text), "receipt truncated original provider evidence");
        }
        peer.assert_finished()
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_and_unrecognized_http_errors_keep_typed_public_failures() -> TestResult {
    let peer = Peer::default();
    scripted(&peer)
        .run(move |case| async move {
            let user = session(&case).await?;
            for (index, response) in [
                ResponseSpec::json(json!({"error":"private-body-canary"})).status(400),
                ResponseSpec::json(json!({"error":{"diagnostic":"private-body-canary"}}))
                    .status(400),
                ResponseSpec::json(json!(["private-body-canary"])).status(400),
                ResponseSpec::bytes("text/html", b"<html>private-body-canary</html>".to_vec())
                    .status(502),
            ]
            .into_iter()
            .enumerate()
            {
                let status = response.status.as_u16();
                peer.expect(
                    Method::POST,
                    "/v1/responses",
                    response.header("x-request-id", format!("req_bad-{index}")),
                );
                let result = user
                    .call_tool("chatgpt.web_search", json!({"prompt":"fail-safely"}))
                    .await?;
                let problem = result.failure()?;
                ensure!(problem.user.fallback.contains(&format!("HTTP {status}")));
                ensure!(problem.user.fallback.contains(&format!("req_bad-{index}")));
                let encoded = result.output.to_string();
                ensure!(!encoded.contains("private-body-canary") && !encoded.contains(API_KEY));
            }
            peer.expect(
                Method::POST,
                "/v1/responses/input_tokens",
                ResponseSpec::bytes("text/html", b"<html>private-count-canary</html>".to_vec())
                    .status(404)
                    .header("x-request-id", "req_count-fixture"),
            );
            let count = user
                .call_tool("chatgpt.count_tokens", json!({"prompt":"count only"}))
                .await?;
            let problem = count.failure()?;
            ensure!(problem.impact != FailureImpact::PartialSuccess);
            ensure!(
                problem.user.fallback.contains("HTTP 404")
                    && problem.user.fallback.contains("req_count-fixture")
            );
            ensure!(problem.user.fallback.contains("Response is not valid JSON"));
            ensure!(!count.output.to_string().contains("private-count-canary"));
            peer.assert_finished()
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_timeouts_keep_read_and_mutation_failure_policies_distinct() -> TestResult {
    let peer = Peer::default();
    scripted(&peer)
        .configure_global(|config| {
            config["providers"]["testkit"]["network"] =
                json!({"request_timeout_secs":1,"connect_timeout_secs":1});
        })
        .run(move |case| async move {
            let user = session(&case).await?;
            for (tool, input, method, path, mutation) in [
                (
                    "chatgpt.files",
                    json!({"action":"get","id":"file_stalled"}),
                    Method::GET,
                    "/v1/files/file_stalled",
                    false,
                ),
                (
                    "chatgpt.web_search",
                    json!({"prompt":"stalled paid request"}),
                    Method::POST,
                    "/v1/responses",
                    true,
                ),
            ] {
                peer.expect(
                    method,
                    path,
                    ResponseSpec::json(json!({"id":"too-late"})).delay(Duration::from_secs(2)),
                );
                let result = user.call_tool(tool, input).await?;
                let problem = result.failure()?;
                ensure!(
                    problem.user.fallback.contains("request timed out"),
                    "{}",
                    problem.user.fallback
                );
                ensure!(
                    problem.user.fallback.contains("1 s"),
                    "{}",
                    problem.user.fallback
                );
                ensure!(!result.output.to_string().contains(API_KEY));
                if mutation {
                    ensure!(
                        problem.impact == FailureImpact::PartialSuccess
                            && problem.retry == RetryDirective::Never
                    );
                    ensure!(
                        problem
                            .user
                            .fallback
                            .matches("do not repeat automatically")
                            .count()
                            == 1
                    );
                } else {
                    ensure!(problem.impact != FailureImpact::PartialSuccess);
                }
            }
            ensure!(
                peer.requests().len() == 2,
                "a timed out paid request was repeated"
            );
            peer.assert_finished()
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_vendor_connection_does_not_leak_query_or_credentials() -> TestResult {
    let peer = Peer::default();
    scripted(&peer).run(move |case| async move {
        let user = session(&case).await?;
        let reserved = std::net::TcpListener::bind("127.0.0.1:0")?;
        let address = reserved.local_addr()?;
        drop(reserved);
        case.settings().set(SettingsLayer::Global, "providers.testkit.adapters.anthropic",
            json!({"enabled":true,"model_discovery":"configured_only","models":{},
                "messages_url":format!("http://{address}/private-route-canary?key=query-secret-canary")}),
            EditOptions::default()).await?.require_success()?;
        let result = user.call_tool("claude.inference", json!({"prompt":"connection-failure"})).await?;
        let problem = result.failure()?;
        ensure!(problem.user.fallback.contains("connection refused"), "{}", problem.user.fallback);
        ensure!(problem.impact == FailureImpact::PartialSuccess && problem.retry == RetryDirective::Never);
        ensure!(problem.user.fallback.chars().count() <= 240);
        for secret in [API_KEY, "query-secret-canary", "private-route-canary"] {
            ensure!(!result.output.to_string().contains(secret), "public failure leaked {secret}");
        }
        ensure!(peer.requests().is_empty(), "refused request fell back to a different endpoint");
        peer.assert_finished()
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_and_network_denials_stop_before_io_and_hide_query_secrets() -> TestResult {
    let peer = Peer::default();
    scripted(&peer)
        .run(move |case| async move {
            let user = session(&case).await?;
            let input = case.paths().workspace.join("denied-input");
            tokio::fs::write(&input, b"must-never-leave-workspace").await?;
            permissions(
                &user,
                json!({(input.to_string_lossy().to_string()):{"read":"deny","write":"deny"}}),
                "allow",
            )
            .await?;
            let denied = user
                .call_tool(
                    "chatgpt.files",
                    json!({"action":"upload","path":"denied-input"}),
                )
                .await?;
            denied.assert_failure_kind(ModelFeedbackKind::PermissionDenied)?;
            ensure!(denied.failure()?.category == FailureCategory::PermissionDenied);
            ensure!(tokio::fs::read(&input).await? == b"must-never-leave-workspace");
            ensure!(peer.requests().is_empty());

            permissions(&user, json!({}), "deny").await?;
            let denied = user
                .call_tool("chatgpt.files", json!({"action":"list"}))
                .await?;
            denied.assert_failure_kind(ModelFeedbackKind::PermissionDenied)?;
            ensure!(denied.failure()?.category == FailureCategory::PermissionDenied);
            ensure!(peer.requests().is_empty());

            permissions(&user, json!({}), "ask").await?;
            // Credentials are supplied by the selected provider, so a query
            // key override is rejected before the network permission stage.
            let invalid = user
                .call_tool(
                    "chatgpt.files",
                    json!({"action":"list","query":{"key":"query-secret-canary"}}),
                )
                .await?;
            invalid.assert_failure_kind(ModelFeedbackKind::InvalidInput)?;
            ensure!(peer.requests().is_empty());

            // An otherwise valid opaque pagination value may also be sensitive.
            // It reaches the real network ask, but must not enter its target.
            let pending = user
                .start_tool(
                    "chatgpt.files",
                    json!({"action":"list","query":{"after":"query-secret-canary"}}),
                )
                .await?;
            let permission = pending.run().wait_permission().await?;
            ensure!(peer.requests().is_empty(), "vendor I/O preceded approval");
            let request = serde_json::to_string(&permission.request)?;
            ensure!(
                request.contains("/v1/files"),
                "approval omitted vendor target: {request}"
            );
            ensure!(
                !request.contains("query-secret-canary"),
                "approval leaked query secret"
            );
            permission.mark_presented().await?;
            permission.deny_once().await?;
            let declined = pending.wait_settled().await?;
            declined.assert_failure_kind(ModelFeedbackKind::UserDeclined)?;
            let problem = declined.failure()?;
            ensure!(problem.category == FailureCategory::PermissionDenied);
            ensure!(
                problem.code.as_str().contains("declin"),
                "denial identity was lost: {problem:?}"
            );
            ensure!(
                peer.requests().is_empty(),
                "permission denial still sent vendor traffic"
            );
            peer.assert_finished()
        })
        .await
}
