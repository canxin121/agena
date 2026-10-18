use super::{
    snapshot_tests::fixture,
    tests::{document, sql},
    *,
};
use serde_json::json;

async fn active(ports: &KernelPorts) -> (SessionId, AttemptHandle, RoundHandle) {
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "tool assembly".into())
        .await
        .unwrap()
        .value
        .id;
    let sent = ports
        .sessions
        .send(RequestId::new(), session, document("fixture"))
        .await
        .unwrap()
        .value;
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, sent.id)
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

#[tokio::test]
async fn tool_draft_has_a_stable_identity_and_only_finalization_mints_execution_capability() {
    for memory in [true, false] {
        let (ports, _maintenance, _admin, _diagnostics, _root) = fixture(memory).await;
        let (session, attempt, round) = active(&ports).await;
        let draft = round
            .begin_tool_draft(RequestId::new(), "provider-id".into(), "tools_call".into())
            .await
            .unwrap()
            .value;
        let view = ports.reader.tool(session, draft.id()).await.unwrap();
        assert_eq!(view.phase, ToolPhase::Preparing);
        assert_eq!(view.execution_kind, ToolExecutionKind::Runtime);
        assert!(view.call.input.is_null());
        assert!(matches!(
            round.finish(RequestId::new(), None, true).await,
            Err(KernelError::Conflict(_))
        ));
        let sent = ports
            .sessions
            .send(RequestId::new(), session, document("another execution"))
            .await
            .unwrap()
            .value;
        let other = ports
            .executor
            .claim(RequestId::new(), session, sent.id)
            .await
            .unwrap()
            .value;
        let frozen = other
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let request = RequestId::new();
        let input = json!({"完整":[1,null,true],"exact":"\n\0"});
        let ready = draft.finalize(request, input.clone()).await.unwrap();
        let repeated = draft.finalize(request, input.clone()).await.unwrap();
        assert_eq!(ready.position, repeated.position);
        assert_eq!(ready.value.id(), draft.id());
        let updated = ports.reader.tool(session, draft.id()).await.unwrap();
        assert_eq!(updated.phase, ToolPhase::Running);
        assert_eq!(updated.call.input, input);
        assert!(updated.revision > view.revision);
        assert!(
            draft
                .finalize(RequestId::new(), json!({"different":true}))
                .await
                .is_err()
        );
        let facts = frozen.prompt().await.unwrap();
        let tool = facts
            .entries
            .iter()
            .find_map(|entry| entry.tool.as_ref())
            .unwrap();
        assert_eq!(tool.phase, ToolPhase::Preparing);
        assert!(
            tool.call.input.is_null(),
            "later finalized arguments leaked into a recorded input"
        );
        ready
            .value
            .complete(
                RequestId::new(),
                ToolResult {
                    phase: ToolPhase::Succeeded,
                    value: json!({"done":true}),
                },
            )
            .await
            .unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn rejected_finalization_preserves_draft_and_failed_round_cancel_and_recovery_interrupt_it() {
    for memory in [true, false] {
        let (ports, _maintenance, admin, _diagnostics, _root) = fixture(memory).await;
        let (session, attempt, round) = active(&ports).await;
        let draft = round
            .begin_tool_draft(RequestId::new(), "draft".into(), "tools_call".into())
            .await
            .unwrap()
            .value;
        let original = ports.reader.tool(session, draft.id()).await.unwrap();
        sql(&admin,"CREATE TRIGGER reject_tool_finalize BEFORE UPDATE ON agena_commits BEGIN SELECT RAISE(ABORT,'reject finalization'); END").await;
        let request = RequestId::new();
        assert!(matches!(
            draft.finalize(request, json!({"argument":1})).await,
            Err(KernelError::Storage(_))
        ));
        assert_eq!(
            *ports.reader.tool(session, draft.id()).await.unwrap(),
            *original
        );
        sql(&admin, "DROP TRIGGER reject_tool_finalize").await;
        round.finish(RequestId::new(), None, false).await.unwrap();
        assert_eq!(
            ports.reader.tool(session, draft.id()).await.unwrap().phase,
            ToolPhase::Interrupted
        );
        assert!(
            draft
                .finalize(request, json!({"argument":1}))
                .await
                .is_err()
        );
        let next = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let cancelled = next
            .begin_tool_draft(RequestId::new(), "cancelled".into(), "tools_call".into())
            .await
            .unwrap()
            .value;
        ports
            .sessions
            .cancel(RequestId::new(), session, attempt.execution_id())
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .tool(session, cancelled.id())
                .await
                .unwrap()
                .phase,
            ToolPhase::Interrupted
        );
        ports
            .sessions
            .retry(RequestId::new(), session, attempt.execution_id())
            .await
            .unwrap();
        assert!(matches!(
            cancelled.finalize(RequestId::new(), json!({})).await,
            Err(KernelError::StaleAttempt)
        ));
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn provider_hosted_operations_have_their_own_kind_and_cannot_ask_for_local_permission() {
    for memory in [true, false] {
        let (ports, _maintenance, _admin, _diagnostics, _root) = fixture(memory).await;
        let (session, _attempt, round) = active(&ports).await;
        let tool = round
            .begin_provider_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "hosted".into(),
                    name: "web_search".into(),
                    input: json!({"query":"fixture"}),
                },
            )
            .await
            .unwrap()
            .value;
        assert_eq!(
            ports
                .reader
                .tool(session, tool.id())
                .await
                .unwrap()
                .execution_kind,
            ToolExecutionKind::ProviderHosted
        );
        assert!(matches!(
            tool.request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: "fixture".into(),
                    description: "invalid local permission".into()
                }
            )
            .await,
            Err(KernelError::Conflict(_))
        ));
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Failed,
                value: json!({"reported_failure":true}),
            },
        )
        .await
        .unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        assert_eq!(
            ports.reader.tool(session, tool.id()).await.unwrap().phase,
            ToolPhase::Failed
        );
        assert_eq!(
            ports.reader.round(session, round.id()).await.unwrap().phase,
            ProviderRoundPhase::Succeeded
        );
        ports.shutdown().await.unwrap();
    }
}
