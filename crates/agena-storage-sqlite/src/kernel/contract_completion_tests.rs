//! Direct public-boundary evidence for the negative-reference and independent
//! concurrent-fact requirements, using the same file/memory operations.

use super::{
    tests::{document, fixture, permission_decision},
    *,
};

async fn count(database: &sea_orm::DatabaseConnection, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(database.get_sqlite_connection_pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn concurrently_admitted_cancel_and_finish_preserve_the_winner_and_fence_old_handles() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "cancel finish race".into())
            .await
            .unwrap()
            .value
            .id;
        // Biased polling admits each order deliberately into the same writer
        // queue; neither command waits for the other's result before admission.
        for cancel_first in [true, false] {
            let sent = ports
                .sessions
                .send(RequestId::new(), session, document("race"))
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
            let output = round
                .begin_text(RequestId::new(), false)
                .await
                .unwrap()
                .value;
            output
                .append("accepted before both terminal commands 世界🙂\n")
                .await
                .unwrap();
            round.finish(RequestId::new(), None, true).await.unwrap();
            let (cancelled, finished) = if cancel_first {
                tokio::join!(biased;
                    ports.sessions.cancel(RequestId::new(), session, sent.id),
                    attempt.finish(RequestId::new(), ExecutionPhase::Succeeded))
            } else {
                let (finished, cancelled) = tokio::join!(biased;
                    attempt.finish(RequestId::new(), ExecutionPhase::Succeeded),
                    ports.sessions.cancel(RequestId::new(), session, sent.id));
                (cancelled, finished)
            };
            let expected = if cancel_first {
                assert_eq!(cancelled.unwrap().value.phase, ExecutionPhase::Cancelled);
                assert!(matches!(finished, Err(KernelError::Conflict(_))));
                ExecutionPhase::Cancelled
            } else {
                finished.unwrap();
                assert!(matches!(cancelled, Err(KernelError::Conflict(_))));
                ExecutionPhase::Succeeded
            };
            assert_eq!(
                ports
                    .reader
                    .execution(session, sent.id)
                    .await
                    .unwrap()
                    .phase,
                expected
            );
            assert!(matches!(
                attempt
                    .finish(RequestId::new(), ExecutionPhase::Failed)
                    .await,
                Err(KernelError::Conflict(_))
            ));
            if cancel_first {
                let next = ports
                    .sessions
                    .retry(RequestId::new(), session, sent.id)
                    .await
                    .unwrap()
                    .value;
                assert!(next.generation > attempt.generation());
                assert!(matches!(
                    attempt.begin_current_round(RequestId::new()).await,
                    Err(KernelError::StaleAttempt)
                ));
                assert_eq!(
                    output.append("late old producer").await,
                    Err(KernelError::StaleAttempt)
                );
                ports
                    .sessions
                    .cancel(RequestId::new(), session, sent.id)
                    .await
                    .unwrap();
            }
        }
        assert!(
            sqlx::query("PRAGMA foreign_key_check")
                .fetch_all(admin.get_sqlite_connection_pool())
                .await
                .unwrap()
                .is_empty()
        );
        drop(admin);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn invalid_public_owners_and_round_references_leave_no_partial_records() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        assert!(
            ports
                .sessions
                .create(RequestId::new(), 9999, "missing workspace".into())
                .await
                .is_err()
        );
        assert_eq!(count(&admin, "agena_sessions").await, 0);
        assert_eq!(count(&admin, "agena_command_results").await, 0);
        let mut sessions = Vec::new();
        for title in ["owner", "foreign"] {
            sessions.push(
                ports
                    .sessions
                    .create(RequestId::new(), 1, title.into())
                    .await
                    .unwrap()
                    .value
                    .id,
            );
        }
        let owner = sessions[0];
        let foreign = sessions[1];
        let owned = ports
            .sessions
            .send(RequestId::new(), owner, document("owned input"))
            .await
            .unwrap()
            .value;
        let unrelated = ports
            .sessions
            .send(RequestId::new(), foreign, document("foreign input"))
            .await
            .unwrap()
            .value;
        let attempt = ports
            .executor
            .claim(RequestId::new(), owner, owned.id)
            .await
            .unwrap()
            .value;
        let receipts = count(&admin, "agena_command_results").await;
        for inputs in [
            vec![EntryId::try_from(i64::MAX).unwrap()],
            vec![unrelated.message_id],
            vec![owned.message_id, unrelated.message_id],
            vec![owned.message_id, owned.message_id],
        ] {
            assert!(matches!(
                attempt.begin_round(RequestId::new(), inputs).await,
                Err(KernelError::Ownership)
            ));
            assert_eq!(count(&admin, "agena_rounds").await, 0);
            assert_eq!(count(&admin, "agena_round_inputs").await, 0);
            assert_eq!(count(&admin, "agena_command_results").await, receipts);
        }
        assert!(matches!(
            ports
                .executor
                .claim(RequestId::new(), owner, unrelated.id)
                .await,
            Err(KernelError::Ownership)
        ));
        let missing = SessionId::try_from(i64::MAX).unwrap();
        assert!(
            ports
                .sessions
                .prepare_blob(RequestId::new(), missing, 1, &mut [7_u8].as_slice())
                .await
                .is_err()
        );
        assert_eq!(count(&admin, "agena_blobs").await, 0);
        assert_eq!(count(&admin, "agena_blob_chunks").await, 0);
        // Round/tool/source owners are supplied by minted capabilities. The
        // public constructors accept no caller-selected parent/round/tool IDs.
        let round = attempt
            .begin_round(RequestId::new(), vec![owned.message_id])
            .await
            .unwrap()
            .value;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "valid-after-rejections".into(),
                    name: "fixture".into(),
                    input: serde_json::json!({"no_external_effect":true}),
                },
            )
            .await
            .unwrap()
            .value;
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"valid":true}),
            },
        )
        .await
        .unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        assert_eq!(count(&admin, "agena_rounds").await, 1);
        assert_eq!(count(&admin, "agena_round_inputs").await, 1);
        assert!(
            sqlx::query("PRAGMA foreign_key_check")
                .fetch_all(admin.get_sqlite_connection_pool())
                .await
                .unwrap()
                .is_empty()
        );
        drop(admin);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn concurrent_tools_permission_reply_and_selection_keep_every_independent_fact() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "concurrent facts".into())
            .await
            .unwrap()
            .value
            .id;
        let sent = ports
            .sessions
            .send(RequestId::new(), session, document("run parallel tools"))
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
        let mut tools = Vec::new();
        for index in 0..3 {
            tools.push(
                round
                    .begin_tool(
                        RequestId::new(),
                        ToolStart {
                            call_id: format!("parallel-{index}"),
                            name: "fixture".into(),
                            input: serde_json::json!({"index":index}),
                        },
                    )
                    .await
                    .unwrap()
                    .value,
            );
        }
        let interaction = tools[2]
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: "fixture.parallel".into(),
                    description: "reply while other tools complete".into(),
                },
            )
            .await
            .unwrap()
            .value;
        let decision = permission_decision(interaction, true, None);
        let barrier = tokio::sync::Barrier::new(4);
        let result_a = ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!({"tool":0,"exact":"世界🙂\r\n\0"}),
        };
        let result_b = ToolResult {
            phase: ToolPhase::Failed,
            value: serde_json::json!({"tool":1,"error":"controlled independent failure"}),
        };
        let (first, second, reply, selection) = tokio::join!(
            async {
                barrier.wait().await;
                tools[0].complete(RequestId::new(), result_a.clone()).await
            },
            async {
                barrier.wait().await;
                tools[1].complete(RequestId::new(), result_b.clone()).await
            },
            async {
                barrier.wait().await;
                ports
                    .sessions
                    .reply(
                        RequestId::new(),
                        session,
                        interaction,
                        decision.clone(),
                        None,
                    )
                    .await
            },
            async {
                barrier.wait().await;
                ports
                    .sessions
                    .update_selection(
                        RequestId::new(),
                        session,
                        SelectionChange::ThinkingMode(Some("high".into())),
                    )
                    .await
            },
        );
        let positions = [
            first.unwrap().position,
            second.unwrap().position,
            reply.unwrap().position,
            selection.unwrap().position,
        ];
        assert_eq!(
            positions
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4
        );
        for (tool, expected) in tools.iter().zip([result_a, result_b]) {
            let actual = ports.reader.tool(session, tool.id()).await.unwrap();
            assert_eq!(actual.phase, expected.phase);
            assert_eq!(actual.result.as_ref(), Some(&expected.value));
        }
        assert_eq!(
            ports
                .reader
                .interaction(session, interaction)
                .await
                .unwrap()
                .decision,
            Some(decision)
        );
        assert_eq!(
            ports
                .reader
                .metadata(session)
                .await
                .unwrap()
                .selection
                .as_ref()
                .unwrap()
                .thinking_mode
                .as_deref(),
            Some("high")
        );
        tools[2]
            .complete(
                RequestId::new(),
                ToolResult {
                    phase: ToolPhase::Succeeded,
                    value: serde_json::json!({"permission":"observed"}),
                },
            )
            .await
            .unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        let history = ports.reader.history(session, None, 16).await.unwrap();
        assert_eq!(
            history
                .entries
                .iter()
                .filter(|entry| matches!(entry.payload, EntryPayload::Tool { .. }))
                .count(),
            3
        );
        assert!(
            sqlx::query("PRAGMA foreign_key_check")
                .fetch_all(admin.get_sqlite_connection_pool())
                .await
                .unwrap()
                .is_empty()
        );
        drop(admin);
        ports.shutdown().await.unwrap();
    }
}
