use super::*;
use agena_domain::RawOutput;
use agena_storage::kernel::{
    ExecutionPhase, MetadataChange, RequestId, ToolPhase, ToolResult, ToolStart,
};

#[tokio::test]
async fn session_file_changes_reads_full_durable_membership_through_http_client() {
    let (provider_url, _requests, provider) = spawn_fake_responses_provider(Vec::new()).await;
    let server = start_test_server(&provider_url).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let source = client
        .create_session(server.workspace_id, "recorded edits", None)
        .await
        .unwrap();
    let application = application_for_test(&server.runtime);
    let (attempt, round, tool, entry) = kernel_fixture::tool(
        &application,
        source.id,
        "fs.write",
        serde_json::json!({"path":"early.txt","content":"hello"}),
    )
    .await;
    let output = RawOutput {
        payload: Some(
            serde_json::json!({"path":"early.txt","kind":"created","sha256":"abc","diff":"+hello"}),
        ),
        ..Default::default()
    };
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!({"raw_output":output}),
        },
    )
    .await
    .unwrap();
    let second = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "second-owned-edit".into(),
                name: "fs.write".into(),
                input: serde_json::json!({"path":"early.txt","content":"world",
                    "expected_sha256":"abc"}),
            },
        )
        .await
        .unwrap()
        .value;
    second
        .complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"raw_output": RawOutput {
                    payload: Some(serde_json::json!({"path":"early.txt", "kind":"updated",
                        "sha256":"def", "diff":"+world"})),
                    ..Default::default()
                }}),
            },
        )
        .await
        .unwrap();
    let second_entry = application
        .session_kernel()
        .unwrap()
        .reader
        .tool(source.id.try_into().unwrap(), second.id())
        .await
        .unwrap()
        .entry_id;
    // The earliest real tool entry sits outside the latest bounded transcript page.
    for n in 0..250 {
        let text = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        text.append(&format!("later {n}")).await.unwrap();
        text.finish(RequestId::new()).await.unwrap();
    }
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    assert!(
        kernel_fixture::entries(&application, source.id)
            .await
            .iter()
            .any(|fact| fact.id == entry)
    );
    // Application continuations traverse only indexed owned tools, preserve
    // canonical entry IDs/order, and never expose presentation-only data.
    let first_page = application
        .read_owned_tool_parts(source.id, None, 1)
        .await
        .unwrap();
    assert_eq!(first_page.parts.len(), 1);
    assert_eq!(first_page.parts[0].part_id, entry.get());
    assert!(first_page.parts[0].presentation.is_none());
    assert!(first_page.has_more);
    let cursor = first_page.next_cursor.unwrap();
    let second_page = application
        .read_owned_tool_parts(source.id, Some(&cursor), 1)
        .await
        .unwrap();
    assert_eq!(second_page.parts.len(), 1);
    assert_eq!(second_page.parts[0].part_id, second_entry.get());
    assert_eq!(second_page.control_version, first_page.control_version);
    assert!(!second_page.has_more);
    assert!(second_page.next_cursor.is_none());
    let summary = client
        .session_file_changes(source.id, 0, true, None, 256 * 1024)
        .await
        .unwrap();
    assert_eq!(summary.total_files, 1);
    assert!(summary.files.is_empty());
    let detail = client
        .session_file_changes(source.id, 0, false, Some("early.txt"), 256 * 1024)
        .await
        .unwrap();
    assert_eq!(
        detail.files[0].operations[0].diff.as_deref(),
        Some("+hello")
    );
    assert_eq!(detail.files[0].operation_count, 2);
    assert_eq!(
        detail.files[0].operations[1].diff.as_deref(),
        Some("+world")
    );
    assert!(!detail.recording_incomplete);
    application
        .session_kernel()
        .unwrap()
        .sessions
        .update_metadata(
            RequestId::new(),
            source.id.try_into().unwrap(),
            MetadataChange {
                title: Some("metadata changed after first tool page".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let stale = application
        .read_owned_tool_parts(source.id, Some(&cursor), 1)
        .await
        .unwrap_err();
    assert_eq!(
        stale.failure.category,
        agena_failure::FailureCategory::Conflict
    );
    let CommandResult::Execution(fork) = client
        .command(Command::ForkSession(ForkSessionParams {
            conversation_mode: None,
            session_id: source.id,
            at_message_id: None,
            title: Some("inherited edits".into()),
        }))
        .await
        .unwrap()
    else {
        panic!("fork returns execution");
    };
    let inherited = kernel_fixture::entries(&application, fork.session.id).await;
    assert!(!inherited.is_empty());
    assert!(
        inherited
            .iter()
            .all(|entry| entry.origin_session_id.get() == source.id)
    );
    assert_eq!(
        client
            .session_file_changes(fork.session.id, 0, false, None, 256 * 1024)
            .await
            .unwrap()
            .total_files,
        0
    );
    // A cached empty fork acquires only its own new edit; withdrawing that
    // complete owned message invalidates the cache without counting inheritance.
    let (owned_attempt, owned_round, owned_tool, _) = kernel_fixture::tool(
        &application,
        fork.session.id,
        "fs.write",
        serde_json::json!({"path":"owned.txt","content":"owned"}),
    )
    .await;
    owned_tool
        .complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"raw_output":RawOutput {
                    payload: Some(serde_json::json!({"path":"owned.txt", "kind":"created",
                        "sha256":"owned", "diff":"+owned"})),
                    ..Default::default()
                }}),
            },
        )
        .await
        .unwrap();
    owned_round
        .finish(RequestId::new(), None, true)
        .await
        .unwrap();
    owned_attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let own = client
        .session_file_changes(fork.session.id, 0, false, Some("owned.txt"), 256 * 1024)
        .await
        .unwrap();
    assert_eq!(own.total_files, 1);
    assert_eq!(own.files[0].path, "owned.txt");
    assert_eq!(own.files[0].operations[0].diff.as_deref(), Some("+owned"));
    let message = application
        .session_kernel()
        .unwrap()
        .reader
        .execution(
            fork.session.id.try_into().unwrap(),
            owned_attempt.execution_id(),
        )
        .await
        .unwrap()
        .message_id;
    application
        .session_kernel()
        .unwrap()
        .sessions
        .withdraw(
            RequestId::new(),
            fork.session.id.try_into().unwrap(),
            message,
        )
        .await
        .unwrap();
    assert_eq!(
        client
            .session_file_changes(fork.session.id, 0, false, None, 256 * 1024)
            .await
            .unwrap()
            .total_files,
        0
    );
    let empty = client
        .create_session(server.workspace_id, "other session", None)
        .await
        .unwrap();
    let wrong_scope = application
        .read_owned_tool_parts(empty.id, Some(&cursor), 1)
        .await
        .unwrap_err();
    assert_eq!(
        wrong_scope.failure.category,
        agena_failure::FailureCategory::InvalidInput
    );
    assert_eq!(
        client
            .session_file_changes(empty.id, 0, false, None, 256 * 1024)
            .await
            .unwrap()
            .total_files,
        0
    );
    let missing = client
        .session_file_changes(i64::MAX, 0, false, None, 256 * 1024)
        .await
        .unwrap_err();
    assert!(missing.problem().is_some());
    provider.await.unwrap();
}
