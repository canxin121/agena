//! Explicit cloud media tests. Only synthetic input files, isolated process
//! environment and loopback servers are used; no real clipboard or API key.
#[path = "support/hosted_fixture.rs"]
mod fixture;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use fixture::{Fixture, PNG, isolate};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
const PROVIDERS: [&str; 3] = ["chatgpt", "claude", "gemini"];
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn local(path: &str) -> Value {
    json!({"source":"local","path":path})
}
fn name(provider: &str, action: &str) -> String {
    format!("{provider}.cloud_{action}")
}
fn assert_actual_image(provider: &str, body: &Value) {
    let encoded = match provider {
        "chatgpt" => {
            body["input"][0]["content"][0]["image_url"]
                .as_str()
                .unwrap()
                .split_once(',')
                .unwrap()
                .1
        }
        "claude" => body["messages"][0]["content"][0]["source"]["data"]
            .as_str()
            .unwrap(),
        _ => body["contents"][0]["parts"][0]["inlineData"]["data"]
            .as_str()
            .unwrap(),
    };
    assert_eq!(
        STANDARD.decode(encoded).unwrap(),
        STANDARD.decode(PNG).unwrap()
    );
    assert!(
        body.get("tools").is_none(),
        "image understanding is multimodal inference, not an invented vendor tool type"
    );
}
#[tokio::test]
async fn every_vendor_receives_validated_image_bytes_and_returns_a_send_receipt() {
    if isolate("every_vendor_receives_validated_image_bytes_and_returns_a_send_receipt") {
        return;
    }
    let f = Fixture::new().await;
    for provider in PROVIDERS {
        let mut request = json!({"inputs":[local("fixture.png")],"prompt":"What is visible?"});
        if provider == "chatgpt" {
            request["detail"] = json!("high");
        }
        let (output, text) = f
            .call(&name(provider, "image_understanding"), request)
            .await
            .unwrap();
        assert_eq!(output["input_sent"], true);
        assert_eq!(output["remote_file_created_by_analysis"], false);
        assert_eq!(
            output["media_inputs"][0]["sha256"],
            hash(&STANDARD.decode(PNG).unwrap())
        );
        assert_eq!(output["media_inputs"][0]["transport"], "inline");
        assert_eq!(output["outcome"], "completed");
        assert!(text.contains("explicit input"));
        assert!(text.contains(provider));
        let request = f.requests().pop().unwrap();
        assert_eq!(request.method, "POST");
        assert!(request.beta.is_none());
        assert_actual_image(provider, &request.body);
        if provider == "chatgpt" {
            assert_eq!(request.body["input"][0]["content"][0]["detail"], "high");
        }
    }
    assert_eq!(f.requests().len(), 3);
}
#[tokio::test]
async fn document_content_is_sent_inline_not_as_a_local_path() {
    if isolate("document_content_is_sent_inline_not_as_a_local_path") {
        return;
    }
    let f = Fixture::new().await;
    let pdf = b"%PDF-1.7\nsynthetic-document-fixture\n%%EOF";
    std::fs::write(f.dir.path().join("report.pdf"), pdf).unwrap();
    std::fs::write(f.dir.path().join("notes.txt"), "UNIQUE_USER_DOCUMENT_TEXT").unwrap();
    for provider in PROVIDERS {
        let (output,_)=f.call(&name(provider,"document_understanding"),json!({"inputs":[local("report.pdf"),local("notes.txt")],"prompt":"Summarize the supplied inputs"})).await.unwrap();
        assert_eq!(output["media_inputs"].as_array().unwrap().len(), 2);
        let request = f.requests().pop().unwrap();
        let text = request.body.to_string();
        assert!(text.contains(&STANDARD.encode(pdf)));
        assert!(text.contains("UNIQUE_USER_DOCUMENT_TEXT"));
        assert!(!text.contains(f.dir.path().to_str().unwrap()));
        assert!(request.body.get("tools").is_none());
    }
}
#[tokio::test]
async fn invalid_media_and_unsupported_settings_never_send_any_network_request() {
    if isolate("invalid_media_and_unsupported_settings_never_send_any_network_request") {
        return;
    }
    let f = Fixture::new().await;
    std::fs::write(f.dir.path().join("fake.png"), "not an image").unwrap();
    let mut corrupt = STANDARD.decode(PNG).unwrap();
    corrupt.truncate(40);
    std::fs::write(f.dir.path().join("truncated.png"), corrupt).unwrap();
    let large = f.dir.path().join("large.bin");
    std::fs::File::create(&large)
        .unwrap()
        .set_len(21 * 1024 * 1024)
        .unwrap();
    for provider in PROVIDERS {
        for inputs in [
            json!([]),
            json!([local("fake.png")]),
            json!([local("truncated.png")]),
            json!([local("large.bin")]),
            json!([{"source":"local","path":"fixture.png","expected_sha256":"stale"}]),
        ] {
            assert!(
                f.call(
                    &name(provider, "image_understanding"),
                    json!({"inputs":inputs,"prompt":"fixture"})
                )
                .await
                .is_err()
            );
        }
        assert!(
            f.call(
                &name(provider, "document_understanding"),
                json!({"inputs":[local("fixture.png")],"prompt":"fixture"})
            )
            .await
            .is_err()
        );
        if provider != "chatgpt" {
            assert!(
                f.call(
                    &name(provider, "image_understanding"),
                    json!({"inputs":[local("fixture.png")],"prompt":"fixture","detail":"high"})
                )
                .await
                .is_err()
            );
        }
    }
    assert!(f.requests().is_empty());
}
#[tokio::test]
async fn cloud_file_upload_status_analyze_and_delete_form_a_complete_owned_lifecycle() {
    if isolate("cloud_file_upload_status_analyze_and_delete_form_a_complete_owned_lifecycle") {
        return;
    }
    let f = Fixture::new().await;
    let expected = STANDARD.decode(PNG).unwrap();
    for (index, provider) in PROVIDERS.into_iter().enumerate() {
        let (uploaded, _) = f
            .call_as(
                &name(provider, "file_upload"),
                json!({"path":"fixture.png","expected_sha256":hash(&expected)}),
                41,
                10 + index as i64,
            )
            .await
            .unwrap();
        assert_eq!(uploaded["state"], "ready");
        let handle = uploaded["handle"].as_str().unwrap();
        let capture = f.requests().pop().unwrap();
        assert!(
            capture
                .raw
                .windows(expected.len())
                .any(|window| window == expected.as_slice())
        );
        if provider == "gemini" {
            assert!(!capture.authenticated);
            assert!(capture.path.ends_with("/upload/session"));
        } else {
            assert!(capture.authenticated);
            assert!(String::from_utf8_lossy(&capture.raw).contains("expires"));
        }
        let record = f.dir.path().join(format!(
            ".agena/artifacts/provider-tools/media/{handle}.json"
        ));
        let serialized = std::fs::read_to_string(&record).unwrap();
        assert!(!serialized.contains(PNG));
        assert!(!serialized.contains("synthetic-provider-test-only"));
        let count = f.requests().len();
        let (replayed, _) = f
            .call_as(
                &name(provider, "file_upload"),
                json!({"path":"fixture.png","expected_sha256":hash(&expected)}),
                41,
                10 + index as i64,
            )
            .await
            .unwrap();
        assert_eq!(replayed["handle"], handle);
        assert_eq!(
            f.requests().len(),
            count,
            "same upload must not be repeated"
        );
        let (status, _) = f
            .call(&name(provider, "file_status"), json!({"handle":handle}))
            .await
            .unwrap();
        assert_eq!(status["state"], "ready");
        let (analysis,_)=f.call(&name(provider,"image_understanding"),json!({"inputs":[{"source":"cloud","handle":handle}],"prompt":"Analyze uploaded image"})).await.unwrap();
        assert_eq!(analysis["media_inputs"][0]["transport"], "provider_file");
        let body = f.requests().pop().unwrap().body.to_string();
        assert!(!body.contains(PNG));
        assert!(body.contains("file_fixture"));
        let (deleted, _) = f
            .call(&name(provider, "file_delete"), json!({"handle":handle}))
            .await
            .unwrap();
        assert_eq!(deleted["state"], "deleted");
        assert_eq!(deleted["physical_erasure_guaranteed"], false);
        let count = f.requests().len();
        assert!(
            f.call(
                &name(provider, "image_understanding"),
                json!({"inputs":[{"source":"cloud","handle":handle}],"prompt":"must not send"})
            )
            .await
            .is_err()
        );
        assert_eq!(f.requests().len(), count);
    }
}
#[tokio::test]
async fn cloud_handles_reject_cross_session_provider_and_tampered_manifest_before_network() {
    if isolate("cloud_handles_reject_cross_session_provider_and_tampered_manifest_before_network") {
        return;
    }
    let f = Fixture::new().await;
    let (uploaded, _) = f
        .call_as(
            "chatgpt.cloud_file_upload",
            json!({"path":"fixture.png"}),
            41,
            20,
        )
        .await
        .unwrap();
    let handle = uploaded["handle"].as_str().unwrap();
    let count = f.requests().len();
    assert!(
        f.call_as(
            "chatgpt.cloud_file_delete",
            json!({"handle":handle}),
            42,
            21
        )
        .await
        .is_err()
    );
    assert!(
        f.call("claude.cloud_file_status", json!({"handle":handle}))
            .await
            .is_err()
    );
    assert!(
        f.call(
            "chatgpt.cloud_file_status",
            json!({"handle":"../../secret"})
        )
        .await
        .is_err()
    );
    let path = f.dir.path().join(format!(
        ".agena/artifacts/provider-tools/media/{handle}.json"
    ));
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["remote_id"] = json!("file_other");
    std::fs::write(path, record.to_string()).unwrap();
    assert!(
        f.call("chatgpt.cloud_file_delete", json!({"handle":handle}))
            .await
            .is_err()
    );
    assert_eq!(f.requests().len(), count);
}
#[tokio::test]
async fn uncertain_upload_is_recorded_once_and_not_automatically_repeated() {
    if isolate("uncertain_upload_is_recorded_once_and_not_automatically_repeated") {
        return;
    }
    let f = Fixture::new().await;
    *f.state.status.lock().unwrap() = Some(503);
    let (output, text) = f
        .call_as(
            "chatgpt.cloud_file_upload",
            json!({"path":"fixture.png"}),
            41,
            30,
        )
        .await
        .unwrap();
    assert_eq!(output["state"], "submission_unknown");
    assert!(text.contains("do not automatically repeat"));
    let count = f.requests().len();
    *f.state.status.lock().unwrap() = None;
    let (replayed, _) = f
        .call_as(
            "chatgpt.cloud_file_upload",
            json!({"path":"fixture.png"}),
            41,
            30,
        )
        .await
        .unwrap();
    assert_eq!(replayed["handle"], output["handle"]);
    assert_eq!(f.requests().len(), count);
    let (status, _) = f
        .call(
            "chatgpt.cloud_file_status",
            json!({"handle":output["handle"]}),
        )
        .await
        .unwrap();
    assert_eq!(status["state"], "submission_unknown");
    assert_eq!(f.requests().len(), count);
}
#[tokio::test]
async fn cloud_expiry_and_delete_confirmation_are_not_invented() {
    if isolate("cloud_expiry_and_delete_confirmation_are_not_invented") {
        return;
    }
    let f = Fixture::new().await;
    assert!(
        f.call(
            "gemini.cloud_file_upload",
            json!({"path":"fixture.png","expires_in_seconds":86400})
        )
        .await
        .is_err()
    );
    assert!(f.requests().is_empty());
    *f.state.response.lock().unwrap() =
        Some(json!({"id":"file_fixture","status":"processed","expires_at":1}));
    let (expired, _) = f
        .call_as(
            "chatgpt.cloud_file_upload",
            json!({"path":"fixture.png"}),
            41,
            40,
        )
        .await
        .unwrap();
    assert_eq!(expired["state"], "expired");
    let count = f.requests().len();
    assert!(f.call("chatgpt.cloud_image_understanding",json!({"inputs":[{"source":"cloud","handle":expired["handle"]}],"prompt":"must not send"})).await.is_err());
    assert_eq!(count, f.requests().len());
    *f.state.response.lock().unwrap() = None;
    let (ready, _) = f
        .call_as(
            "chatgpt.cloud_file_upload",
            json!({"path":"fixture.png"}),
            41,
            41,
        )
        .await
        .unwrap();
    *f.state.response.lock().unwrap() = Some(json!({"id":"file_wrong","deleted":true}));
    let (deleted, _) = f
        .call(
            "chatgpt.cloud_file_delete",
            json!({"handle":ready["handle"]}),
        )
        .await
        .unwrap();
    assert_eq!(deleted["state"], "deletion_unconfirmed");
    assert_ne!(deleted["deletion_acknowledged"], true);
}
#[tokio::test]
async fn resumable_upload_origin_change_does_not_forward_input_bytes_or_credentials() {
    if isolate("resumable_upload_origin_change_does_not_forward_input_bytes_or_credentials") {
        return;
    }
    let f = Fixture::new().await;
    *f.state.upload_target.lock().unwrap() = Some("https://example.invalid/do-not-contact".into());
    let (output, _) = f
        .call("gemini.cloud_file_upload", json!({"path":"fixture.png"}))
        .await
        .unwrap();
    assert_eq!(output["state"], "submission_unknown");
    let requests = f.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].path.ends_with("/upload/v1beta/files"));
    assert!(
        !requests[0]
            .raw
            .windows(8)
            .any(|part| part == b"\x89PNG\r\n\x1a\n")
    );
}

#[tokio::test]
async fn cancelling_upload_leaves_a_recoverable_unknown_receipt_without_automatic_retry() {
    if isolate("cancelling_upload_leaves_a_recoverable_unknown_receipt_without_automatic_retry") {
        return;
    }
    let f = std::sync::Arc::new(Fixture::new().await);
    f.state
        .block_upload
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let worker = f.clone();
    let task = tokio::spawn(async move {
        worker
            .call_as(
                "chatgpt.cloud_file_upload",
                json!({"path":"fixture.png"}),
                41,
                70,
            )
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        f.state.upload_entered.notified(),
    )
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    f.state
        .block_upload
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let (recovered, _) = f
        .call_as(
            "chatgpt.cloud_file_upload",
            json!({"path":"fixture.png"}),
            41,
            70,
        )
        .await
        .unwrap();
    assert_eq!(recovered["state"], "submission_unknown");
    assert_eq!(f.requests().len(), 1);
    assert!(recovered["handle"].as_str().unwrap().starts_with("media_"));
}
