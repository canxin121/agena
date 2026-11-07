//! Canonical accounting metadata must retain original ownership and pagination
//! independently of the tool's duplicated result projections.
use agena_domain::{ComposerDocument, ComposerNode};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};
use serde_json::{Value, json};

async fn start(runtime: &SqliteKernel, title: &str) -> (SessionId, AttemptHandle, RoundHandle) {
    let workspace = runtime
        .workspaces
        .ensure_id("/tool-metadata")
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, title.into())
        .await
        .unwrap()
        .value
        .id;
    let execution = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text { text: title.into() }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    (session, attempt, round)
}

async fn tool(round: &RoundHandle, call: &str) -> ToolHandle {
    round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: call.into(),
                name: "metadata-producer".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value
}

async fn complete(tool: &ToolHandle, phase: ToolPhase, value: Value) {
    tool.complete(RequestId::new(), ToolResult { phase, value })
        .await
        .unwrap();
}

#[tokio::test]
async fn canonical_tool_metadata_is_owned_paged_and_excludes_inherited_or_projected_copies() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let (first, first_attempt, first_round) = start(&runtime, "First owner").await;
    let (other, other_attempt, other_round) = start(&runtime, "Other owner").await;

    let first_tool = tool(&first_round, "first").await;
    complete(
        &first_tool,
        ToolPhase::Succeeded,
        json!({
            "metadata": {"accounting": "original-first"},
            "raw_output": {"payload": {"metadata": {"accounting": "duplicate-first"}}},
            "result": {"metadata": {"accounting": "model-projection"}}
        }),
    )
    .await;
    let other_tool = tool(&other_round, "other").await;
    complete(
        &other_tool,
        ToolPhase::Succeeded,
        json!({"metadata": {"accounting": "other-owner"}}),
    )
    .await;
    let projected = tool(&first_round, "projection-only").await;
    complete(
        &projected,
        ToolPhase::Succeeded,
        json!({"raw_output": {"payload": {"metadata": {"accounting": "not-canonical"}}}}),
    )
    .await;
    let second_tool = tool(&first_round, "second").await;
    complete(
        &second_tool,
        ToolPhase::Failed,
        json!({"metadata": {"accounting": "original-second"}}),
    )
    .await;
    let pending = tool(&first_round, "not-terminal").await;

    let reader = &runtime.ports.reader;
    let page = reader.owned_tool_metadata(first, None, 1).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id, first_tool.id());
    assert_eq!(page[0].metadata, json!({"accounting": "original-first"}));
    assert!(page[0].finished_at_ms >= page[0].created_at_ms);
    let after = page[0].id;
    drop(page);

    let page = reader
        .owned_tool_metadata(first, Some(after), 1)
        .await
        .unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id, second_tool.id());
    assert_eq!(page[0].metadata, json!({"accounting": "original-second"}));
    drop(page);
    assert!(
        reader
            .owned_tool_metadata(first, Some(second_tool.id()), 1)
            .await
            .unwrap()
            .is_empty()
    );

    let other_page = reader.owned_tool_metadata(other, None, 128).await.unwrap();
    assert_eq!(other_page.len(), 1);
    assert_eq!(other_page[0].id, other_tool.id());
    assert_eq!(other_page[0].metadata, json!({"accounting": "other-owner"}));
    drop(other_page);

    complete(&pending, ToolPhase::Succeeded, json!({"done": true})).await;
    first_round
        .finish(RequestId::new(), None, true)
        .await
        .unwrap();
    first_attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    other_round
        .finish(RequestId::new(), None, true)
        .await
        .unwrap();
    other_attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();

    let entry = reader.tool(first, second_tool.id()).await.unwrap().entry_id;
    let fork = runtime
        .ports
        .sessions
        .fork(RequestId::new(), first, entry)
        .await
        .unwrap()
        .value
        .id;
    assert!(
        reader
            .tool(fork, first_tool.id())
            .await
            .unwrap()
            .result
            .is_some()
    );
    assert!(
        reader
            .owned_tool_metadata(fork, None, 128)
            .await
            .unwrap()
            .is_empty()
    );
    let originals = reader.owned_tool_metadata(first, None, 128).await.unwrap();
    assert_eq!(
        originals.iter().map(|item| item.id).collect::<Vec<_>>(),
        vec![first_tool.id(), second_tool.id()],
    );
    drop(originals);
    runtime.ports.clone().shutdown().await.unwrap();
}
