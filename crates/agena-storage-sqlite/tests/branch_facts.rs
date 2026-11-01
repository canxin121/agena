use agena_domain::{
    ComposerDocument, ComposerNode, PermissionReply, PermissionReplyKind, SessionStateKind,
};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};

#[tokio::test]
async fn presenting_a_parent_question_does_not_change_the_frozen_branch_request() {
    let (runtime, root) = fixture().await;
    let execution = send(&runtime, root).await;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), root, execution.id)
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
                call_id: "question".into(),
                name: "ask".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let interaction = tool
        .request_interaction(
            RequestId::new(),
            InteractionRequest::Input {
                request: agena_domain::UserInputRequest {
                    request_id: "question".into(),
                    session_id: Some(root.get()),
                    title: "Choose".into(),
                    body_markdown: String::new(),
                    kind: agena_domain::UserInputKind::AskUser,
                    source: agena_domain::UserInputSource::Host,
                    auto_resolution_ms: None,
                    presented_at: None,
                    questions: Vec::new(),
                    created_at: chrono::Utc::now(),
                },
            },
        )
        .await
        .unwrap()
        .value;
    let entry = runtime
        .ports
        .reader
        .tool(root, tool.id())
        .await
        .unwrap()
        .entry_id;
    let branch = fork(&runtime, root, entry).await;
    let presented = runtime
        .ports
        .sessions
        .present_interaction(RequestId::new(), root, interaction)
        .await
        .unwrap()
        .value;
    assert!(
        matches!(presented.request, InteractionRequest::Input { request } if request.presented_at.is_some())
    );
    let nested = fork(&runtime, branch, entry).await;
    for session in [branch, nested] {
        let frozen = runtime
            .ports
            .reader
            .interaction(session, interaction)
            .await
            .unwrap();
        assert!(
            matches!(&frozen.request, InteractionRequest::Input { request } if request.presented_at.is_none())
        );
        let own = send(&runtime, session).await;
        let owner = runtime
            .ports
            .executor
            .claim_next(RequestId::new(), session, own.id)
            .await
            .unwrap()
            .value;
        let request = owner
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let prompt = request.prompt().await.unwrap();
        let frozen = prompt
            .entries
            .iter()
            .find_map(|entry| {
                entry
                    .interaction
                    .as_ref()
                    .filter(|view| view.id == interaction)
            })
            .unwrap();
        assert!(
            matches!(&frozen.request, InteractionRequest::Input { request } if request.presented_at.is_none())
        );
        request.finish(RequestId::new(), None, true).await.unwrap();
        owner
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
    }
    runtime
        .ports
        .sessions
        .cancel(RequestId::new(), root, execution.id)
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
}

async fn fixture() -> (SqliteKernel, SessionId) {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/branch-facts-tests")
        .await
        .unwrap();
    let root = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Root".into())
        .await
        .unwrap()
        .value
        .id;
    (runtime, root)
}

async fn send(runtime: &SqliteKernel, session: SessionId) -> ExecutionView {
    runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Use this session's frozen context".into(),
            }]),
        )
        .await
        .unwrap()
        .value
}

async fn fork(runtime: &SqliteKernel, source: SessionId, through: EntryId) -> SessionId {
    runtime
        .ports
        .sessions
        .fork(RequestId::new(), source, through)
        .await
        .unwrap()
        .value
        .id
}

#[tokio::test]
async fn inherited_tool_interaction_and_provider_state_stay_at_the_nested_fork_cut() {
    let (runtime, root) = fixture().await;
    let execution = send(&runtime, root).await;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), root, execution.id)
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
                call_id: "parent-call".into(),
                name: "inspect".into(),
                input: serde_json::json!({"at":"fork"}),
            },
        )
        .await
        .unwrap()
        .value;
    let interaction = tool
        .request_interaction(
            RequestId::new(),
            InteractionRequest::Permission {
                action_key: "parent-permission".into(),
                description: "Only the parent owns this request".into(),
            },
        )
        .await
        .unwrap()
        .value;
    let original = runtime.ports.reader.tool(root, tool.id()).await.unwrap();
    let first = fork(&runtime, root, original.entry_id).await;
    runtime
        .ports
        .sessions
        .reply(
            RequestId::new(),
            root,
            interaction,
            InteractionDecision::Permission {
                reply: PermissionReply {
                    request_id: interaction.get().to_string(),
                    kind: PermissionReplyKind::AllowOnce,
                    reason: None,
                    scope: None,
                },
                allowed: true,
            },
            None,
        )
        .await
        .unwrap();
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!({"later":"parent result"}),
        },
    )
    .await
    .unwrap();
    round
        .finish(
            RequestId::new(),
            Some(serde_json::json!({"later":"provider state"})),
            true,
        )
        .await
        .unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    // Construct the second fork only after all source facts have changed.
    let nested = fork(&runtime, first, original.entry_id).await;

    for branch in [first, nested] {
        let frozen = runtime.ports.reader.tool(branch, tool.id()).await.unwrap();
        assert_eq!(frozen.revision, original.revision);
        assert_eq!(frozen.phase, ToolPhase::Running);
        assert!(frozen.result.is_none());
        assert!(frozen.finished_at_ms.is_none());
        assert_eq!(frozen.interactions, vec![interaction]);
        let decision = runtime
            .ports
            .reader
            .interaction(branch, interaction)
            .await
            .unwrap();
        assert!(decision.decision.is_none());
        assert!(decision.decided_at_ms.is_none());
        let replay = runtime
            .ports
            .reader
            .round(branch, round.id())
            .await
            .unwrap();
        assert_eq!(replay.phase, ProviderRoundPhase::Running);
        assert!(replay.provider_state.is_none());
        assert!(replay.finished_at_ms.is_none());
        let state = runtime
            .ports
            .reader
            .control_snapshot(branch, 8)
            .await
            .unwrap();
        assert_eq!(state.summary.state, SessionStateKind::Ready);
        assert!(state.pending_interactions.interactions.is_empty());
        assert!(state.active_executions.executions.is_empty());

        let own = send(&runtime, branch).await;
        let owner = runtime
            .ports
            .executor
            .claim_next(RequestId::new(), branch, own.id)
            .await
            .unwrap()
            .value;
        let request = owner
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let prompt = request.prompt().await.unwrap();
        let tool_input = prompt
            .entries
            .iter()
            .find_map(|entry| {
                entry
                    .tool
                    .as_ref()
                    .filter(|candidate| candidate.id == tool.id())
            })
            .unwrap();
        assert_eq!(tool_input.phase, ToolPhase::Running);
        assert!(tool_input.result.is_none());
        let interaction_input = prompt
            .entries
            .iter()
            .find_map(|entry| {
                entry
                    .interaction
                    .as_ref()
                    .filter(|candidate| candidate.id == interaction)
            })
            .unwrap();
        assert!(interaction_input.decision.is_none());
        assert!(
            prompt
                .replays
                .iter()
                .all(|replay| replay.round_id != round.id() || replay.provider_state.is_none())
        );
        request.finish(RequestId::new(), None, true).await.unwrap();
        owner
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
    }
    assert_eq!(
        runtime
            .ports
            .reader
            .tool(root, tool.id())
            .await
            .unwrap()
            .phase,
        ToolPhase::Succeeded
    );
    assert!(
        runtime
            .ports
            .reader
            .interaction(root, interaction)
            .await
            .unwrap()
            .decision
            .is_some()
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn a_preparing_tool_in_a_fork_never_acquires_later_arguments_or_interactions() {
    let (runtime, root) = fixture().await;
    let execution = send(&runtime, root).await;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), root, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let draft = round
        .begin_tool_draft(RequestId::new(), "draft".into(), "inspect".into())
        .await
        .unwrap()
        .value;
    let original = runtime.ports.reader.tool(root, draft.id()).await.unwrap();
    let branch = fork(&runtime, root, original.entry_id).await;
    let tool = draft
        .finalize(RequestId::new(), serde_json::json!({"later":"arguments"}))
        .await
        .unwrap()
        .value;
    let interaction = tool
        .request_interaction(
            RequestId::new(),
            InteractionRequest::Permission {
                action_key: "later-permission".into(),
                description: "Created after the fork".into(),
            },
        )
        .await
        .unwrap()
        .value;
    let nested = fork(&runtime, branch, original.entry_id).await;
    for session in [branch, nested] {
        let frozen = runtime.ports.reader.tool(session, tool.id()).await.unwrap();
        assert_eq!(frozen.phase, ToolPhase::Preparing);
        assert!(frozen.call.input.is_null());
        assert!(frozen.interactions.is_empty());
        assert!(
            runtime
                .ports
                .reader
                .interaction(session, interaction)
                .await
                .is_err()
        );
    }
    runtime
        .ports
        .sessions
        .cancel(RequestId::new(), root, execution.id)
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn parent_retries_do_not_change_branch_execution_facts_or_original_round_inputs() {
    let (runtime, root) = fixture().await;
    let execution = send(&runtime, root).await;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), root, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let original_prompt = round.prompt().await.unwrap();
    let original_user = original_prompt
        .entries
        .iter()
        .find(|entry| entry.entry.id == execution.message_id)
        .unwrap();
    assert_eq!(original_user.entry.execution_phase, ExecutionPhase::Running);
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "finished".into(),
                name: "inspect".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!({"at":"fork"}),
        },
    )
    .await
    .unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let tool_entry = runtime
        .ports
        .reader
        .tool(root, tool.id())
        .await
        .unwrap()
        .entry_id;
    let branch = fork(&runtime, root, tool_entry).await;
    let frozen_user = runtime
        .ports
        .reader
        .entry(branch, execution.message_id)
        .await
        .unwrap();
    assert_eq!(frozen_user.execution_phase, ExecutionPhase::Succeeded);
    let retry = runtime
        .ports
        .sessions
        .continue_execution(RequestId::new(), root, execution.message_id)
        .await
        .unwrap()
        .value;
    assert_eq!(retry.generation.get(), 2);
    runtime
        .ports
        .sessions
        .cancel(RequestId::new(), root, retry.id)
        .await
        .unwrap();
    let nested = fork(&runtime, branch, tool_entry).await;

    for session in [branch, nested] {
        let user = runtime
            .ports
            .reader
            .entry(session, execution.message_id)
            .await
            .unwrap();
        assert_eq!(user.execution_phase, ExecutionPhase::Succeeded);
        assert_eq!(user.execution_generation.get(), 1);
        assert_eq!(user.revision, frozen_user.revision);
        let inherited_execution = runtime
            .ports
            .reader
            .execution(session, execution.id)
            .await
            .unwrap();
        assert_eq!(inherited_execution.generation.get(), 1);
        assert_eq!(inherited_execution.phase, ExecutionPhase::Succeeded);
        // The original provider request predates the fork and must retain its
        // own captured state, even though the branch cut saw a completed run.
        let inherited_prompt = runtime
            .ports
            .reader
            .prompt(session, round.id())
            .await
            .unwrap();
        let inherited_user = inherited_prompt
            .entries
            .iter()
            .find(|entry| entry.entry.id == execution.message_id)
            .unwrap();
        assert_eq!(
            inherited_user.entry.execution_phase,
            ExecutionPhase::Running
        );
        assert_eq!(inherited_user.entry.revision, original_user.entry.revision);

        let own = send(&runtime, session).await;
        let owner = runtime
            .ports
            .executor
            .claim_next(RequestId::new(), session, own.id)
            .await
            .unwrap()
            .value;
        let request = owner
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let prompt = request.prompt().await.unwrap();
        let source_user = prompt
            .entries
            .iter()
            .find(|entry| entry.entry.id == execution.message_id)
            .unwrap();
        assert_eq!(source_user.entry.execution_phase, ExecutionPhase::Succeeded);
        assert_eq!(source_user.entry.execution_generation.get(), 1);
        request.finish(RequestId::new(), None, true).await.unwrap();
        owner
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
    }
    runtime.ports.clone().shutdown().await.unwrap();
}
