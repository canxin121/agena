use super::*;
use agena_domain::{RawOutput, ToolResultState};
use agena_runtime_contracts::part_content::ToolCallContent;
use agena_storage::store::{NewPart, PartRole, PartState};

#[tokio::test]
async fn session_file_changes_reads_full_durable_membership_through_http_client() {
    let (provider_url, _requests, provider) = spawn_fake_responses_provider(Vec::new()).await;
    let server = start_test_server(&provider_url).await;
    let client = AgenaClient::new(&server.url).unwrap();
    let source = client
        .create_session(server.workspace_id, "recorded edits", None)
        .await
        .unwrap();
    let store = application_for_test(&server.runtime)
        .session_store_facade()
        .unwrap();
    // More than the transcript raw scan page. The earliest edit must not
    // vanish simply because the visible transcript contains a recent tail.
    let mut parts = vec![NewPart {
        state: PartState::Completed,
        ..NewPart::pending("tool_call", PartRole::Assistant, ToolCallContent {
            name: "fs.write".into(), input: serde_json::json!({"path":"early.txt","content":"hello"}),
            state: ToolResultState::Completed,
            output: Some(RawOutput { payload: Some(serde_json::json!({"path":"early.txt","kind":"created","sha256":"abc","diff":"+hello"})), ..Default::default() }),
            ..Default::default()
        }.as_value())
    }];
    parts.extend((0..250).map(|n| NewPart {
        state: PartState::Completed,
        ..NewPart::pending(
            "text",
            PartRole::Assistant,
            serde_json::json!({"text":format!("later {n}")}),
        )
    }));
    store.submit_user_run(source.id, parts, None).await.unwrap();
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
    assert!(!detail.recording_incomplete);
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
    assert!(!store.load(fork.session.id).await.unwrap().parts.is_empty());
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
