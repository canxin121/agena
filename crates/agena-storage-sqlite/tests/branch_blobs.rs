use agena_domain::{BlobId, ComposerDocument, ComposerNode};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};
use serde_json::{Value, json};

async fn assert_late_blob_is_hidden(
    runtime: &SqliteKernel,
    session: SessionId,
    tool: ToolId,
    blob: BlobId,
) {
    let view = runtime.ports.reader.tool(session, tool).await.unwrap();
    assert_eq!(view.phase, ToolPhase::Running);
    assert!(view.result.is_none());
    assert!(view.finished_at_ms.is_none());
    assert!(view.blobs.is_empty());
    assert!(
        runtime
            .ports
            .reader
            .entry_blobs(session, view.entry_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        runtime.ports.reader.open_blob(session, blob).await,
        Err(KernelError::Ownership)
    ));
}

fn archive_records(bytes: &[u8]) -> Vec<Value> {
    let records: Vec<Value> = std::str::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.last().unwrap()["kind"], "footer");
    records
}

#[tokio::test]
async fn late_tool_blobs_remain_hidden_across_nested_forks_and_archive_round_trips() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/branch-blob-tests")
        .await
        .unwrap();
    let parent = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Parent".into())
        .await
        .unwrap()
        .value
        .id;
    let execution = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            parent,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Produce an attachment".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), parent, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "attachment".into(),
                name: "produce".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let tool_entry = runtime
        .ports
        .reader
        .tool(parent, tool.id())
        .await
        .unwrap()
        .entry_id;
    let branch = runtime
        .ports
        .sessions
        .fork(RequestId::new(), parent, tool_entry)
        .await
        .unwrap()
        .value
        .id;

    let bytes = b"An attachment published after the fork";
    let mut source = bytes.as_slice();
    let blob = runtime
        .ports
        .sessions
        .prepare_blob(RequestId::new(), parent, bytes.len() as u64, &mut source)
        .await
        .unwrap();
    let blob_id = blob.descriptor().id;
    tool.complete_with_blobs(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"blob_id": blob_id}),
        },
        &[blob],
    )
    .await
    .unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let parent_blob = runtime
        .ports
        .reader
        .open_blob(parent, blob_id)
        .await
        .unwrap();
    assert_eq!(parent_blob.range(0, 1024).await.unwrap().bytes, bytes);
    drop(parent_blob);

    // This fork happens after the root publishes the blob. It must inherit
    // the first branch's earlier cut, including its empty attachment set.
    let nested = runtime
        .ports
        .sessions
        .fork(RequestId::new(), branch, tool_entry)
        .await
        .unwrap()
        .value
        .id;
    for session in [branch, nested] {
        assert_late_blob_is_hidden(&runtime, session, tool.id(), blob_id).await;
    }

    // A completed control execution has a Notice anchor. Its provider round
    // and captured inputs must be exported just like those of a user anchor.
    let continued = runtime
        .ports
        .sessions
        .start_control_execution(
            RequestId::new(),
            nested,
            ControlExecutionPurpose::Continuation,
        )
        .await
        .unwrap()
        .value;
    let continued_attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), nested, continued.id)
        .await
        .unwrap()
        .value;
    let continued_round = continued_attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let output = continued_round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    output
        .append("The branch continued independently.")
        .await
        .unwrap();
    output.finish(RequestId::new()).await.unwrap();
    let provider_state = json!({"control_replay": "preserved"});
    continued_round
        .finish(RequestId::new(), Some(provider_state.clone()), true)
        .await
        .unwrap();
    continued_attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("branch.jsonl");
    let mut target = tokio::fs::File::create(&path).await.unwrap();
    runtime
        .ports
        .reader
        .export_jsonl(nested, &mut target)
        .await
        .unwrap();
    target.sync_all().await.unwrap();
    drop(target);
    let exported = archive_records(&tokio::fs::read(&path).await.unwrap());
    assert!(
        exported.iter().any(|record| record["kind"] == "blob"
            && record["data"]["descriptor"]["id"] == json!(blob_id))
    );
    assert!(
        exported.iter().any(|record| record["kind"] == "round"
            && record["data"]["execution_id"] == json!(continued.id))
    );
    assert!(exported.iter().any(|record| record["kind"] == "round_input"
        && record["data"]["round_id"] == json!(continued_round.id())));
    let restored = runtime
        .ports
        .sessions
        .import_archive(RequestId::new(), workspace, &path)
        .await
        .unwrap()
        .value
        .id;
    let history = runtime
        .ports
        .reader
        .history(restored, None, 32)
        .await
        .unwrap();
    let restored_tool = history
        .entries
        .iter()
        .find_map(|entry| match entry.payload {
            EntryPayload::Tool { tool_id } => Some(tool_id),
            _ => None,
        })
        .unwrap();
    let anchor = history
        .entries
        .iter()
        .find(|entry| {
            entry.id == entry.message_id && matches!(entry.payload, EntryPayload::Notice { .. })
        })
        .unwrap();
    let restored_control = runtime
        .ports
        .reader
        .execution(restored, anchor.execution_id)
        .await
        .unwrap();
    assert_eq!(restored_control.purpose, ExecutionPurpose::Continuation);
    let restored_execution = restored_control.id;
    let restored_text = history
        .entries
        .iter()
        .find_map(|entry| match entry.payload {
            EntryPayload::Text { content_id } => Some(content_id),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        runtime
            .ports
            .reader
            .text_range(restored, restored_text, None, 1024)
            .await
            .unwrap()
            .bytes,
        b"The branch continued independently."
    );
    drop(restored_control);
    drop(history);

    // Archives retain complete raw dependencies. Re-export exposes their
    // remapped identities without granting the ordinary reader access to them.
    let mut bytes = Vec::new();
    runtime
        .ports
        .reader
        .export_jsonl(restored, &mut bytes)
        .await
        .unwrap();
    let records = archive_records(&bytes);
    let descriptor = &records
        .iter()
        .find(|record| record["kind"] == "blob")
        .unwrap()["data"]["descriptor"];
    let restored_blob: BlobId = serde_json::from_value(descriptor["id"].clone()).unwrap();
    assert_ne!(restored_blob, blob_id);
    assert_late_blob_is_hidden(&runtime, restored, restored_tool, restored_blob).await;
    let round = &records
        .iter()
        .find(|record| {
            record["kind"] == "round" && record["data"]["execution_id"] == json!(restored_execution)
        })
        .unwrap()["data"];
    let restored_round: RoundId = serde_json::from_value(round["id"].clone()).unwrap();
    let replay = runtime
        .ports
        .reader
        .round(restored, restored_round)
        .await
        .unwrap();
    assert_eq!(replay.provider_state, Some(provider_state));
    assert!(!replay.inputs.is_empty());
    let prompt = runtime
        .ports
        .reader
        .prompt(restored, restored_round)
        .await
        .unwrap();
    let frozen_tool = prompt
        .entries
        .iter()
        .find_map(|entry| entry.tool.as_ref().filter(|tool| tool.id == restored_tool))
        .unwrap();
    assert_eq!(frozen_tool.phase, ToolPhase::Running);
    assert!(frozen_tool.result.is_none());
    runtime.ports.clone().shutdown().await.unwrap();
}
