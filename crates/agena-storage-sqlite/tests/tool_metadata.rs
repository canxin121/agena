//! Accounting uses canonical result attribution from each session's own tools.
use agena_domain::{ComposerDocument, ComposerNode};
use agena_provider::PROVIDER_TOOL_USAGE_METADATA_KEY;
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
async fn session_usage_excludes_inherited_and_projected_tool_copies() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let (first, first_attempt, first_round) = start(&runtime, "First owner").await;
    let (other, other_attempt, other_round) = start(&runtime, "Other owner").await;

    let first_tool = tool(&first_round, "first").await;
    complete(
        &first_tool,
        ToolPhase::Succeeded,
        json!({
            "metadata": {(PROVIDER_TOOL_USAGE_METADATA_KEY): "original-first"},
            "raw_output": {"payload": {"metadata": {(PROVIDER_TOOL_USAGE_METADATA_KEY): "duplicate-first"}}},
            "result": {"metadata": {(PROVIDER_TOOL_USAGE_METADATA_KEY): "model-projection"}}
        }),
    )
    .await;
    let other_tool = tool(&other_round, "other").await;
    complete(
        &other_tool,
        ToolPhase::Succeeded,
        json!({"metadata": {(PROVIDER_TOOL_USAGE_METADATA_KEY): "other-owner"}}),
    )
    .await;
    let projected = tool(&first_round, "projection-only").await;
    complete(
        &projected,
        ToolPhase::Succeeded,
        json!({"raw_output": {"payload": {"metadata": {(PROVIDER_TOOL_USAGE_METADATA_KEY): "not-canonical"}}}}),
    )
    .await;
    let second_tool = tool(&first_round, "second").await;
    complete(
        &second_tool,
        ToolPhase::Failed,
        json!({"metadata": {(PROVIDER_TOOL_USAGE_METADATA_KEY): "original-second"}}),
    )
    .await;
    let pending = tool(&first_round, "not-terminal").await;

    let reader = &runtime.ports.reader;
    assert_eq!(
        tool_usage(reader, first).await,
        ["original-first", "original-second"]
    );
    assert_eq!(tool_usage(reader, other).await, ["other-owner"]);

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
    assert!(tool_usage(reader, fork).await.is_empty());
    assert_eq!(
        tool_usage(reader, first).await,
        ["original-first", "original-second"]
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

async fn tool_usage(reader: &KernelReader, session: SessionId) -> Vec<String> {
    let mut observations = Vec::new();
    reader
        .visit_session_usage(session, &mut |row| {
            assert_eq!(row.session_id, session);
            if let StoredUsageSource::Tool(payload) = row.source {
                observations.push(payload);
            }
        })
        .await
        .unwrap();
    observations.sort_unstable();
    observations
}
