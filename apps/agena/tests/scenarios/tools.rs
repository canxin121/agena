use super::{fixture, path_permission, workspace_permission};
use agena_domain::AttachmentItem;
use agena_testkit::{Failure, ModelFeedbackKind, Reply, RequestMatch, SessionOptions, TestResult};
use anyhow::{Context, ensure};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_driver_preserves_real_output_and_expected_tool_failures() -> TestResult {
    fixture()
        .seed_file(
            "workspace/helper.txt",
            b"tool-driver-durable-marker".to_vec(),
        )
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let read = user
                .call_tool("fs.read", json!({"file_path": "helper.txt"}))
                .await?;
            read.assert_succeeded()?;
            ensure!(read.failure().is_err(), "success was accepted as a failure");
            read.assert_text_contains("tool-driver-durable-marker")?;
            ensure!(
                read.data()?
                    .to_string()
                    .contains("tool-driver-durable-marker")
            );
            ensure!(read.part.message_id == Some(read.run.status.message_id));
            ensure!(read.attachments()?.is_empty());
            ensure!(
                read.attachment(0).is_err(),
                "missing attachment was invented"
            );

            let missing = user
                .call_tool("fs.read", json!({"file_path": "does-not-exist.txt"}))
                .await?;
            missing.assert_failed()?;
            ensure!(
                missing
                    .assert_failure_kind(ModelFeedbackKind::PermissionDenied)
                    .is_err(),
                "a missing file was mistaken for a permission rejection"
            );
            missing.run.assert_succeeded()?;
            ensure!(missing.call_id != read.call_id);
            ensure!(missing.part.part_id != read.part.part_id);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_workspace_sessions_do_not_change_the_default_workspace() -> TestResult {
    fixture()
        .seed_file("workspace/scope.txt", b"default-workspace-marker".to_vec())
        .seed_file(
            "alternate-workspace/scope.txt",
            b"alternate-workspace-marker".to_vec(),
        )
        .run(|case| async move {
            let default = case.user().new_session().await?;
            let alternate = case
                .user()
                .new_session_in_with(
                    case.paths().root.join("alternate-workspace"),
                    SessionOptions {
                        title: "Explicit workspace session".into(),
                        ..Default::default()
                    },
                )
                .await?;
            let later_default = case.user().new_session().await?;
            let state = alternate.snapshot().await?;
            ensure!(state.session.title == "Explicit workspace session");
            ensure!(state.session.workspace_id != default.snapshot().await?.session.workspace_id);
            ensure!(
                default.snapshot().await?.session.workspace_id
                    == later_default.snapshot().await?.session.workspace_id
            );
            for (user, marker, other_marker) in [
                (
                    &default,
                    "default-workspace-marker",
                    "alternate-workspace-marker",
                ),
                (
                    &alternate,
                    "alternate-workspace-marker",
                    "default-workspace-marker",
                ),
                (
                    &later_default,
                    "default-workspace-marker",
                    "alternate-workspace-marker",
                ),
            ] {
                let result = user
                    .call_tool("fs.read", json!({"file_path":"scope.txt"}))
                    .await?;
                result.assert_succeeded()?;
                result.assert_text_contains(marker)?;
                ensure!(
                    !result.text().contains(other_marker),
                    "tool read another workspace"
                );
            }
            let error = case
                .user()
                .new_session_in("alternate-workspace")
                .await
                .err()
                .context("relative workspace path was silently resolved against a cwd")?;
            ensure!(format!("{error:#}").contains("absolute server-side path"));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attachment_reader_uses_session_scope_and_enforces_transfer_limits() -> TestResult {
    const DEFAULT_LIMIT: usize = 4 * 1024 * 1024;
    const CONTENT: &[u8] = b"session-media-download-marker";
    fixture()
        .seed_file("workspace/download.txt", CONTENT.to_vec())
        .seed_file("workspace/large.bin", vec![b'x'; DEFAULT_LIMIT + 1])
        .seed_file("foreign-workspace/own.txt", b"another workspace".to_vec())
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let foreign = case
                .user()
                .new_session_in(case.paths().root.join("foreign-workspace"))
                .await?;
            let local: AttachmentItem = serde_json::from_value(json!({
                "kind":"file", "mime":"text/plain", "filename":"download.txt",
                "source":{"source":"local_path","path":case.workspace().join("download.txt")},
                // Untrusted presentation metadata cannot weaken the byte limit.
                "size_bytes":1
            }))?;
            let (filename, bytes) = user.download_attachment(&local).await?;
            ensure!(filename == "download.txt" && bytes == CONTENT);
            let too_small = user
                .download_attachment_with_limit(&local, 8)
                .await
                .err()
                .context("attachment transfer ignored its explicit limit")?;
            let diagnostic = format!("{too_small:#}");
            ensure!(diagnostic.contains(&format!("session {}", user.id())));
            ensure!(
                diagnostic.contains("download.txt") && diagnostic.contains("8 byte client limit")
            );
            ensure!(
                user.download_attachment_with_limit(&local, CONTENT.len())
                    .await?
                    .1
                    == CONTENT
            );

            let wrong_owner = foreign
                .download_attachment(&local)
                .await
                .err()
                .context("another workspace's session read an unowned attachment")?;
            let diagnostic = format!("{wrong_owner:#}");
            ensure!(
                diagnostic.contains(&format!("session {}", foreign.id()))
                    && diagnostic.contains("download.txt")
            );

            let large: AttachmentItem = serde_json::from_value(json!({
                "kind":"file", "mime":"application/octet-stream", "filename":"large.bin",
                "source":{"source":"local_path","path":case.workspace().join("large.bin")}
            }))?;
            let default_limit = user
                .download_attachment(&large)
                .await
                .err()
                .context("default attachment limit exceeded 4 MiB")?;
            ensure!(format!("{default_limit:#}").contains("4194304 byte client limit"));
            let (_, bytes) = user
                .download_attachment_with_limit(&large, DEFAULT_LIMIT + 1)
                .await?;
            ensure!(bytes.len() == DEFAULT_LIMIT + 1 && bytes.iter().all(|byte| *byte == b'x'));

            let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let seen = requests.clone();
            let server = case
                .serve_http(axum::Router::new().route(
                    "/external",
                    axum::routing::get(move || {
                        let seen = seen.clone();
                        async move {
                            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            "must not be fetched"
                        }
                    }),
                ))
                .await?;
            let mut external = local.clone();
            external.source = agena_domain::AttachmentSource::Url {
                url: server.url("/external"),
            };
            let error = user
                .download_attachment(&external)
                .await
                .err()
                .context("attachment helper fetched an external URL")?;
            ensure!(format!("{error:#}").contains("external attachment URLs are not fetched"));
            ensure!(requests.load(std::sync::atomic::Ordering::SeqCst) == 0);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_interaction_helpers_preserve_requests_and_explain_missing_approval() -> TestResult {
    fixture().run(|case| async move {
        let user = case.user().new_session().await?;
        let target = case.workspace().join("interactive.txt");
        path_permission(&user, &target, "ask").await?;
        let writing = user.start_tool("fs.write", json!({"path":"interactive.txt","content":"approved through helper"})).await?;
        let permission = writing.wait_permission().await?;
        ensure!(permission.session_id() == user.id() && !target.exists());
        permission.mark_presented().await?;
        permission.allow_once().await?;
        writing.wait_settled().await?.assert_succeeded()?;
        ensure!(tokio::fs::read_to_string(&target).await? == "approved through helper");

        let denied_target = case.workspace().join("denied-interactive.txt");
        path_permission(&user, &denied_target, "deny").await?;
        let denied = user.start_tool("fs.write", json!({
            "path":"denied-interactive.txt","content":"private-invocation-value-not-error-details"
        })).await?;
        let error = denied.wait_permission().await.err()
            .context("policy denial was presented as a pending approval")?;
        let diagnostic = format!("{error:#}");
        ensure!(diagnostic.contains("fs.write") && diagnostic.contains("PermissionDenied"));
        ensure!(diagnostic.contains("permission.denied") && diagnostic.contains("ended before the interaction"));
        ensure!(!diagnostic.contains("private-invocation-value-not-error-details"));
        denied.wait_settled().await?.assert_failure_kind(ModelFeedbackKind::PermissionDenied)?;
        let missing_input = denied.wait_user_input().await.err()
            .context("completed tool was presented as user input")?;
        ensure!(format!("{missing_input:#}").contains("PermissionDenied"));
        ensure!(!denied_target.exists());
        ensure!(tokio::fs::read_to_string(&target).await? == "approved through helper");

        let asking = user.start_tool("interaction.ask", json!({"questions":[{
            "header":"Color", "question":"Which color should be used?",
            "options":[{"label":"Blue","description":"Use blue."},{"label":"Green","description":"Use green."}],
            "multiple":false
        }]})).await?;
        let question = asking.wait_user_input().await?;
        ensure!(question.session_id() == user.id());
        question.mark_presented().await?;
        question.answer(std::collections::BTreeMap::from([("0".into(),vec!["Green".into()])])).await?;
        let answer = asking.wait_settled().await?;
        answer.assert_succeeded()?;
        answer.assert_text_contains("Green")?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_tool_call_executes_real_file_read_and_returns_matching_call_id() -> TestResult {
    fixture()
        .seed_file("workspace/note.txt", b"real-file-content-marker".to_vec())
        .run(|case| async move {
            let model = case.model();
            let call = model
                .expect(RequestMatch::user_text("read-file-marker").declares_function("tools_call"))
                .reply(
                    Reply::tool_call(
                        "call_read_note",
                        "fs.read",
                        json!({"file_path": "note.txt"}),
                    )
                    .arguments_chunks([
                        "{\"tool\":",
                        "\"fs.read\",\"input\":{\"file_path\":\"note.txt\"}}",
                    ])
                    .fragment_bytes(13),
                );
            model
                .expect(RequestMatch::tool_output("call_read_note"))
                .after(&call)
                .check_tool_output_contains("call_read_note", "real-file-content-marker")
                .reply(Reply::text("read-complete"));
            let user = case.user().new_session().await?;
            workspace_permission(&user, "deny").await?;
            let result = user.send("read-file-marker").await?.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("read-complete")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_reply_controls_real_tool_side_effect() -> TestResult {
    fixture().run(|case| async move {
        let target = case.workspace().join("approved.txt");
        let model = case.model();
        let call = model.expect(RequestMatch::user_text("permission-write-marker"))
            .reply(Reply::tool_call("call_write_approved", "fs.write",
                json!({"path": "approved.txt", "content": "permission-approved", "create_parents": true})));
        model.expect(RequestMatch::tool_output("call_write_approved"))
            .after(&call)
            .check(|request| {
                let output = request.tool_output_text("call_write_approved")
                    .expect("matched tool output");
                let output: serde_json::Value = serde_json::from_str(&output)?;
                ensure!(output["phase"] == "succeeded", "approved tool failed: {output}");
                Ok(())
            })
            .reply(Reply::text("write-complete"));
        let user = case.user().new_session().await?;
        path_permission(&user, &target, "ask").await?;
        let run = user.send("permission-write-marker").await?;
        let prompt = run.wait_permission().await?;
        ensure!(!target.exists(), "write occurred before approval");
        prompt.mark_presented().await?;
        prompt.allow_once().await?;
        let result = run.wait_settled().await?;
        result.assert_succeeded()?;
        result.assert_assistant_text("write-complete")?;
        ensure!(tokio::fs::read_to_string(target).await? == "permission-approved");
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn denied_permission_is_visible_to_model_without_writing_file() -> TestResult {
    fixture()
        .run(|case| async move {
            let target = case.workspace().join("denied.txt");
            let model = case.model();
            let call = model
                .expect(RequestMatch::user_text("permission-deny-marker"))
                .reply(Reply::tool_call(
                    "call_write_denied",
                    "fs.write",
                    json!({"path": "denied.txt", "content": "must-not-be-written"}),
                ));
            model
                .expect(RequestMatch::tool_output("call_write_denied"))
                .after(&call)
                .check(|request| {
                    let output = request
                        .tool_output_text("call_write_denied")
                        .expect("matched tool output");
                    let output: serde_json::Value = serde_json::from_str(&output)?;
                    ensure!(output["phase"] == "failed", "missing denial: {output}");
                    let failure: Failure =
                        serde_json::from_value(output["result"]["failure"].clone())?;
                    ensure!(
                        failure.model.as_ref().is_some_and(
                            |feedback| feedback.kind == ModelFeedbackKind::UserDeclined
                        ),
                        "user rejection lost its structured identity: {failure:?}"
                    );
                    Ok(())
                })
                .reply(Reply::text("permission-was-denied"));
            let user = case.user().new_session().await?;
            path_permission(&user, &target, "ask").await?;
            let run = user.send("permission-deny-marker").await?;
            run.wait_permission().await?.deny_once().await?;
            let result = run.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("permission-was-denied")?;
            ensure!(!target.exists(), "denied tool wrote a file");
            Ok(())
        })
        .await
}
