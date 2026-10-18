use super::{
    tests::{document, fixture},
    *,
};
use agena_domain::{SessionRelationKind, SessionStateKind};

#[tokio::test]
async fn concurrent_permission_replies_keep_the_complete_winner_and_its_timestamp_atomically() {
    use agena_domain::{PermissionMode, PermissionReply, PermissionReplyKind, PermissionScope};
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, _round, tool) = event_tests::tool(&ports).await;
        let id = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: "atomic-permission".into(),
                    description: "choose one complete reply".into(),
                },
            )
            .await
            .unwrap()
            .value;
        let before = ports.reader.interaction(session, id).await.unwrap();
        assert!(before.decided_at_ms.is_none());
        let created = before.created_at_ms;
        drop(before);
        let allow = InteractionDecision::Permission {
            reply: PermissionReply {
                request_id: id.get().to_string(),
                kind: PermissionReplyKind::AllowAlways,
                reason: Some("allow with the exact user's reason".into()),
                scope: Some(PermissionScope::Workspace),
            },
            allowed: true,
        };
        let deny = InteractionDecision::Permission {
            reply: PermissionReply {
                request_id: id.get().to_string(),
                kind: PermissionReplyKind::DenyAlways,
                reason: Some("deny with the other user's reason".into()),
                scope: Some(PermissionScope::Workspace),
            },
            allowed: false,
        };
        let allow_rule = Some(RememberPermission {
            mode: PermissionMode::Allow,
            scope: PermissionScope::Workspace,
        });
        let deny_rule = Some(RememberPermission {
            mode: PermissionMode::Deny,
            scope: PermissionScope::Workspace,
        });
        let (allowed, denied) = tokio::join!(
            ports.sessions.reply(
                RequestId::new(),
                session,
                id,
                allow.clone(),
                allow_rule.clone()
            ),
            ports.sessions.reply(
                RequestId::new(),
                session,
                id,
                deny.clone(),
                deny_rule.clone()
            )
        );
        let (winner, rule) = match (allowed, denied) {
            (Ok(_), Err(KernelError::Conflict(_))) => (allow, allow_rule),
            (Err(KernelError::Conflict(_)), Ok(_)) => (deny, deny_rule),
            other => panic!("permission replies did not resolve to one winner: {other:?}"),
        };
        let stored = ports.reader.interaction(session, id).await.unwrap();
        assert_eq!(stored.decision, Some(winner.clone()));
        assert_eq!(stored.remembered, rule);
        let decided = stored.decided_at_ms.unwrap();
        assert!(decided >= created);
        drop(stored);
        ports
            .sessions
            .reply(RequestId::new(), session, id, winner.clone(), rule.clone())
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .interaction(session, id)
                .await
                .unwrap()
                .decided_at_ms,
            Some(decided)
        );
        let mut changed_reason = winner;
        let InteractionDecision::Permission { reply, .. } = &mut changed_reason else {
            unreachable!();
        };
        reply.reason = Some("a different reason cannot overwrite the winner".into());
        assert!(matches!(
            ports
                .sessions
                .reply(RequestId::new(), session, id, changed_reason, rule)
                .await,
            Err(KernelError::Conflict(_))
        ));
        let rules: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM agena_permission_rules WHERE action_key='atomic-permission'",
        )
        .fetch_one(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
        assert_eq!(rules, 1);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn execution_guidance_and_successful_finish_have_one_atomic_order() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "guidance race".into())
            .await
            .unwrap()
            .value
            .id;
        for index in 0..8 {
            let execution = ports
                .sessions
                .send(RequestId::new(), session, document("delegated work"))
                .await
                .unwrap()
                .value;
            let attempt = ports
                .executor
                .claim(RequestId::new(), session, execution.id)
                .await
                .unwrap()
                .value;
            let payload = serde_json::json!({"kind":"task_guidance","text":"preserve this instruction","wake":false});
            let key = format!("race-{index}");
            let deliver = ports.executor.deliver_to_execution(
                RequestId::new(),
                session,
                execution.id,
                key.clone(),
                payload.clone(),
            );
            let finish = attempt.finish(RequestId::new(), ExecutionPhase::Succeeded);
            let (delivered, finished) = if index % 2 == 0 {
                tokio::join!(deliver, finish)
            } else {
                let (finished, delivered) = tokio::join!(finish, deliver);
                (delivered, finished)
            };
            match (delivered, finished) {
                (Ok(receipt), Err(KernelError::PendingDelivery(id))) => {
                    assert_eq!(id, execution.id);
                    assert_eq!(receipt.value.target_execution, Some(execution.id));
                    assert_eq!(
                        ports
                            .reader
                            .execution(session, execution.id)
                            .await
                            .unwrap()
                            .phase,
                        ExecutionPhase::Running
                    );
                    ports
                        .executor
                        .consume_delivery(
                            RequestId::new(),
                            session,
                            receipt.value.id,
                            execution.message_id,
                            "preserve this instruction".into(),
                        )
                        .await
                        .unwrap();
                    attempt
                        .finish(RequestId::new(), ExecutionPhase::Succeeded)
                        .await
                        .unwrap();
                    let replay = ports
                        .executor
                        .deliver_to_execution(RequestId::new(), session, execution.id, key, payload)
                        .await
                        .unwrap();
                    assert_eq!(replay.value.id, receipt.value.id);
                    assert!(replay.value.consumed);
                }
                (Err(KernelError::Conflict(message)), Ok(_)) => {
                    assert!(message.contains("tasks.followup"))
                }
                other => panic!("guidance was lost or terminal state admitted it: {other:?}"),
            }
            assert!(matches!(
                ports
                    .executor
                    .deliver_to_execution(
                        RequestId::new(),
                        session,
                        execution.id,
                        format!("late-{index}"),
                        serde_json::json!({"text":"too late"})
                    )
                    .await,
                Err(KernelError::Conflict(_))
            ));
        }
        let execution = ports
            .sessions
            .send(RequestId::new(), session, document("cancel guidance"))
            .await
            .unwrap()
            .value;
        let attempt = ports
            .executor
            .claim(RequestId::new(), session, execution.id)
            .await
            .unwrap()
            .value;
        ports
            .executor
            .deliver_to_execution(
                RequestId::new(),
                session,
                execution.id,
                "cancel-pending-guidance".into(),
                serde_json::json!({"text":"pending"}),
            )
            .await
            .unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Cancelled)
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .execution(session, execution.id)
                .await
                .unwrap()
                .phase,
            ExecutionPhase::Cancelled
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn targeted_guidance_cannot_transfer_ownership_or_survive_cancellation_and_recovery() {
    for memory in [true, false] {
        let (ports, admin, _root, url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "guidance ownership".into())
            .await
            .unwrap()
            .value
            .id;
        let mut executions = Vec::new();
        for text in ["owner", "another execution"] {
            executions.push(
                ports
                    .sessions
                    .send(RequestId::new(), session, document(text))
                    .await
                    .unwrap()
                    .value,
            );
        }
        let owner = &executions[0];
        let other = &executions[1];
        let attempt = ports
            .executor
            .claim(RequestId::new(), session, owner.id)
            .await
            .unwrap()
            .value;
        let guidance = ports
            .executor
            .deliver_to_execution(
                RequestId::new(),
                session,
                owner.id,
                "owned-guidance".into(),
                serde_json::json!({"text":"only the owner may consume this"}),
            )
            .await
            .unwrap()
            .value;
        assert!(matches!(
            ports
                .executor
                .consume_delivery(
                    RequestId::new(),
                    session,
                    guidance.id,
                    other.message_id,
                    "wrong execution".into(),
                )
                .await,
            Err(KernelError::Ownership)
        ));
        assert!(matches!(
            ports
                .executor
                .resume_delivery(
                    RequestId::new(),
                    session,
                    guidance.id,
                    owner.message_id,
                    "must not start another run".into(),
                )
                .await,
            Err(KernelError::Conflict(_))
        ));
        assert_eq!(
            ports
                .reader
                .history(session, None, 20)
                .await
                .unwrap()
                .entries
                .len(),
            2
        );
        let consumed = ports
            .executor
            .consume_delivery(
                RequestId::new(),
                session,
                guidance.id,
                owner.message_id,
                "owned notice".into(),
            )
            .await
            .unwrap()
            .value;
        assert_eq!(
            ports
                .reader
                .entry(session, consumed)
                .await
                .unwrap()
                .execution_id,
            owner.id
        );
        let cancelled = ports
            .executor
            .deliver_to_execution(
                RequestId::new(),
                session,
                owner.id,
                "cancelled-guidance".into(),
                serde_json::json!({"text":"do not replay after cancellation"}),
            )
            .await
            .unwrap()
            .value;
        ports
            .sessions
            .cancel(RequestId::new(), session, owner.id)
            .await
            .unwrap();
        let state: (String, Option<i64>) =
            sqlx::query_as("SELECT state,result_entry_id FROM agena_deliveries WHERE id=?")
                .bind(cancelled.id.to_string())
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap();
        assert_eq!(state, ("failed".into(), None));
        let retried = ports
            .sessions
            .retry(RequestId::new(), session, owner.id)
            .await
            .unwrap()
            .value;
        assert!(retried.generation > owner.generation);
        let retry = ports
            .executor
            .claim(RequestId::new(), session, owner.id)
            .await
            .unwrap()
            .value;
        assert!(matches!(
            ports
                .executor
                .consume_delivery(
                    RequestId::new(),
                    session,
                    cancelled.id,
                    owner.message_id,
                    "cannot resurrect retired guidance".into(),
                )
                .await,
            Err(KernelError::Conflict(_))
        ));
        retry
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        assert!(
            ports
                .reader
                .pending_deliveries(session, None, 64)
                .await
                .unwrap()
                .is_empty()
        );
        let running = ports
            .executor
            .claim(RequestId::new(), session, other.id)
            .await
            .unwrap()
            .value;
        ports
            .executor
            .deliver_to_execution(
                RequestId::new(),
                session,
                other.id,
                "interrupted-guidance".into(),
                serde_json::json!({"text":"do not replay after recovery"}),
            )
            .await
            .unwrap();
        drop((attempt, retry, running, admin));
        ports.shutdown().await.unwrap();
        if !memory {
            let reopened = open_session_kernel(&url).await.unwrap();
            assert_eq!(
                reopened
                    .reader
                    .execution(session, other.id)
                    .await
                    .unwrap()
                    .phase,
                ExecutionPhase::Interrupted
            );
            assert!(
                reopened
                    .reader
                    .pending_deliveries(session, None, 64)
                    .await
                    .unwrap()
                    .is_empty()
            );
            let history = reopened.reader.history(session, None, 20).await.unwrap();
            assert_eq!(history.entries.len(), 3);
            assert!(history.entries.iter().all(|entry| !matches!(
                &entry.payload,
                EntryPayload::Notice { text } if text.contains("replay") || text.contains("resurrect")
            )));
            drop(history);
            reopened.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn control_versions_track_committed_session_changes_without_replaying_or_touching_other_sessions()
 {
    for memory in [true, false] {
        let (ports, admin, _root, url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "control".into())
            .await
            .unwrap()
            .value
            .id;
        let other = ports
            .sessions
            .create(RequestId::new(), 1, "unrelated".into())
            .await
            .unwrap()
            .value
            .id;
        let other_version = *ports.reader.control_version(other).await.unwrap();
        let metadata_version = ports
            .reader
            .metadata(session)
            .await
            .unwrap()
            .metadata_version;
        let before = *ports.reader.control_version(session).await.unwrap();
        let request = RequestId::new();
        let sent = ports
            .sessions
            .send(request, session, document("versioned"))
            .await
            .unwrap();
        let sent_version = *ports.reader.control_version(session).await.unwrap();
        assert!(sent_version > before);
        assert_eq!(
            ports
                .sessions
                .send(request, session, document("versioned"))
                .await
                .unwrap(),
            sent
        );
        assert_eq!(
            *ports.reader.control_version(session).await.unwrap(),
            sent_version
        );
        assert!(
            ports
                .sessions
                .send(request, session, document("different request"))
                .await
                .is_err()
        );
        assert_eq!(
            *ports.reader.control_version(session).await.unwrap(),
            sent_version
        );
        let attempt = ports
            .executor
            .claim(RequestId::new(), session, sent.value.id)
            .await
            .unwrap()
            .value;
        let claimed_version = *ports.reader.control_version(session).await.unwrap();
        assert!(claimed_version > sent_version);
        assert_eq!(
            ports
                .reader
                .metadata(session)
                .await
                .unwrap()
                .metadata_version,
            metadata_version
        );
        assert_eq!(
            *ports.reader.control_version(other).await.unwrap(),
            other_version
        );
        drop(attempt);
        drop(admin);
        ports.shutdown().await.unwrap();
        if !memory {
            let reopened = open_session_kernel(&url).await.unwrap();
            assert!(*reopened.reader.control_version(session).await.unwrap() > claimed_version);
            assert_eq!(
                *reopened.reader.control_version(other).await.unwrap(),
                other_version
            );
            assert_eq!(
                reopened
                    .reader
                    .execution(session, sent.value.id)
                    .await
                    .unwrap()
                    .phase,
                ExecutionPhase::Interrupted
            );
            reopened.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn listing_changes_capture_both_sides_and_parent_counts_without_resetting_unrelated_shared_history()
 {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "list scopes".into())
            .await
            .unwrap()
            .value
            .id;
        let sent = ports
            .sessions
            .send(RequestId::new(), session, document("shared"))
            .await
            .unwrap()
            .value;
        ports
            .sessions
            .cancel(RequestId::new(), session, sent.id)
            .await
            .unwrap();
        let parent_before = ports.reader.summary(session).await.unwrap();
        assert_eq!(parent_before.child_session_count, 0);
        let mut watch = ports.reader.watch_all(None).unwrap();
        let child = ports
            .sessions
            .fork(RequestId::new(), session, sent.message_id)
            .await
            .unwrap()
            .value
            .id;
        let created = watch.recv().await.unwrap();
        let list = created.list_change.as_ref().unwrap();
        assert!(list.before.is_none());
        assert_eq!(list.after.as_ref().unwrap().session, child);
        let parent_created = ports.reader.summary(session).await.unwrap();
        assert_eq!(parent_created.child_session_count, 1);
        assert!(!parent_created.ptr_eq(&parent_before));
        assert_eq!(parent_before.child_session_count, 0);
        assert_eq!(
            list.parents
                .iter()
                .map(|state| state.session)
                .collect::<Vec<_>>(),
            vec![session]
        );
        ports
            .sessions
            .update_metadata(
                RequestId::new(),
                child,
                MetadataChange {
                    title: Some("renamed child".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let renamed = watch.recv().await.unwrap();
        let list = renamed.list_change.as_ref().unwrap();
        let before = list.before.as_ref().unwrap();
        let after = list.after.as_ref().unwrap();
        assert_eq!(before.session, child);
        assert_eq!(after.session, child);
        assert!(after.metadata_version > before.metadata_version);
        assert_eq!(before.pinned, after.pinned);
        assert_eq!(before.favorite, after.favorite);
        assert_eq!(before.processing_state, after.processing_state);
        assert!(list.parents.is_empty());
        assert!(
            ports
                .reader
                .summary(session)
                .await
                .unwrap()
                .ptr_eq(&parent_created)
        );
        ports
            .sessions
            .delete(RequestId::new(), child)
            .await
            .unwrap();
        let removed = watch.recv().await.unwrap();
        let list = removed.list_change.as_ref().unwrap();
        assert_eq!(list.before.as_ref().unwrap().session, child);
        assert!(list.after.is_none());
        assert_eq!(
            ports
                .reader
                .summary(session)
                .await
                .unwrap()
                .child_session_count,
            0
        );
        assert_eq!(
            list.parents
                .iter()
                .map(|state| state.session)
                .collect::<Vec<_>>(),
            vec![session]
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn forks_share_live_content_without_inheriting_execution_control_and_origin_deletion_keeps_history()
 {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "origin".into())
            .await
            .unwrap()
            .value
            .id;
        let sent = ports
            .sessions
            .send(RequestId::new(), session, document("request"))
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
        let text = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        text.append("before").await.unwrap();
        let child = ports
            .sessions
            .fork(RequestId::new(), session, text.entry_id())
            .await
            .unwrap()
            .value
            .id;
        assert!(
            ports
                .reader
                .active_executions(child, None, 10)
                .await
                .unwrap()
                .executions
                .is_empty()
        );
        assert!(
            ports
                .sessions
                .cancel(RequestId::new(), child, sent.id)
                .await
                .is_err()
        );
        text.append(" after").await.unwrap();
        assert_eq!(
            ports
                .reader
                .text_range(child, text.id(), None, 1024)
                .await
                .unwrap()
                .bytes,
            b"before after"
        );
        let inherited = ports.reader.execution(child, sent.id).await.unwrap();
        assert_eq!(inherited.session_id, session);
        assert_eq!(inherited.phase, ExecutionPhase::Running);
        drop(inherited);
        ports
            .sessions
            .delete(RequestId::new(), session)
            .await
            .unwrap();
        assert!(text.append("too late").await.is_err());
        let history = ports.reader.history(child, None, 20).await.unwrap();
        assert_eq!(history.entries.len(), 2);
        assert_eq!(
            ports
                .reader
                .text_range(child, text.id(), None, 1024)
                .await
                .unwrap()
                .bytes,
            b"before after"
        );
        assert_eq!(
            ports.reader.execution(child, sent.id).await.unwrap().phase,
            ExecutionPhase::Cancelled
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn rewind_restores_completed_input_before_its_boundary_and_withdraw_does_not_remove_a_fork() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "origin".into())
            .await
            .unwrap()
            .value
            .id;
        let first = ports
            .sessions
            .send(RequestId::new(), session, document("one"))
            .await
            .unwrap()
            .value;
        ports
            .sessions
            .cancel(RequestId::new(), session, first.id)
            .await
            .unwrap();
        let second = ports
            .sessions
            .send(RequestId::new(), session, document("two 界"))
            .await
            .unwrap()
            .value;
        assert!(
            ports
                .sessions
                .rewind(RequestId::new(), session, second.message_id)
                .await
                .is_err()
        );
        ports
            .sessions
            .cancel(RequestId::new(), session, second.id)
            .await
            .unwrap();
        let fork = ports
            .sessions
            .fork(RequestId::new(), session, second.message_id)
            .await
            .unwrap()
            .value
            .id;
        let old = ports.reader.history(session, None, 1).await.unwrap().next;
        let rewound = ports
            .sessions
            .rewind(RequestId::new(), session, second.message_id)
            .await
            .unwrap()
            .value;
        assert_eq!(rewound.document, document("two 界"));
        let entries = ports
            .reader
            .history(rewound.session.id, None, 10)
            .await
            .unwrap();
        assert_eq!(
            entries
                .entries
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![first.message_id]
        );
        assert_eq!(
            ports
                .reader
                .configuration(rewound.session.id)
                .await
                .unwrap()
                .relation_kind,
            SessionRelationKind::Rewind
        );
        ports
            .sessions
            .withdraw(RequestId::new(), session, second.message_id)
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .history(fork, None, 10)
                .await
                .unwrap()
                .entries
                .len(),
            2
        );
        assert_eq!(
            ports
                .reader
                .history(session, None, 10)
                .await
                .unwrap()
                .entries
                .len(),
            1
        );
        if let Some(old) = old {
            assert!(ports.reader.history(session, Some(old), 1).await.is_err());
        }
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn configured_creation_and_filter_counts_are_atomic_and_use_origin_processing_state() {
    let (ports, _admin, _root, _url) = fixture(true).await;
    let parent = ports
        .sessions
        .create(RequestId::new(), 1, "parent".into())
        .await
        .unwrap()
        .value
        .id;
    let config = SessionConfiguration {
        parent_session_id: Some(parent),
        relation_kind: SessionRelationKind::Child,
        ..Default::default()
    };
    let child = ports
        .sessions
        .create_configured(RequestId::new(), 1, "child needle".into(), config)
        .await
        .unwrap()
        .value
        .id;
    ports
        .sessions
        .update_metadata(
            RequestId::new(),
            child,
            MetadataChange {
                favorite: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let sent = ports
        .sessions
        .send(RequestId::new(), parent, document("busy"))
        .await
        .unwrap()
        .value;
    let query = SummaryQuery {
        workspace_id: Some(1),
        favorite: Some(true),
        search: Some("needle".into()),
        states: vec![SessionStateKind::Ready],
        limit: 10,
        ..Default::default()
    };
    let rows = ports.reader.query_summaries(query).await.unwrap();
    assert_eq!(rows.total, Some(1));
    assert_eq!(rows.sessions[0].id, child);
    assert_eq!(rows.sessions[0].parent_session_id, Some(parent));
    let counts = ports.reader.workspace_statistics(&[1]).await.unwrap();
    assert_eq!(counts[&1].running, 1);
    assert_eq!(counts[&1].roots, 1);
    assert_eq!(counts[&1].total, 2);
    ports
        .sessions
        .cancel(RequestId::new(), parent, sent.id)
        .await
        .unwrap();
    let invalid = SessionConfiguration {
        parent_session_id: Some(SessionId::try_from(999).unwrap()),
        relation_kind: SessionRelationKind::Child,
        ..Default::default()
    };
    assert!(
        ports
            .sessions
            .create_configured(RequestId::new(), 1, "must roll back".into(), invalid)
            .await
            .is_err()
    );
    assert_eq!(
        ports
            .reader
            .query_summaries(SummaryQuery {
                workspace_id: Some(1),
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        Some(2)
    );
    ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn reliable_delivery_consumption_is_atomic_and_duplicate_business_keys_cannot_change_the_payload()
 {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "delivery".into())
            .await
            .unwrap()
            .value
            .id;
        let sent = ports
            .sessions
            .send(RequestId::new(), session, document("request"))
            .await
            .unwrap()
            .value;
        ports
            .sessions
            .cancel(RequestId::new(), session, sent.id)
            .await
            .unwrap();
        let payload = serde_json::json!({"exit_code":0});
        let first = ports
            .executor
            .deliver(
                RequestId::new(),
                session,
                "process:one:exit".into(),
                payload.clone(),
            )
            .await
            .unwrap()
            .value;
        let repeated = ports
            .executor
            .deliver(
                RequestId::new(),
                session,
                "process:one:exit".into(),
                payload,
            )
            .await
            .unwrap()
            .value;
        assert_eq!(first.id, repeated.id);
        assert!(
            ports
                .executor
                .deliver(
                    RequestId::new(),
                    session,
                    "process:one:exit".into(),
                    serde_json::json!({"exit_code":1})
                )
                .await
                .is_err()
        );
        let request = RequestId::new();
        let consumed = ports
            .executor
            .consume_delivery(
                request,
                session,
                first.id,
                sent.message_id,
                "process finished".into(),
            )
            .await
            .unwrap();
        assert_eq!(
            ports
                .executor
                .consume_delivery(
                    request,
                    session,
                    first.id,
                    sent.message_id,
                    "process finished".into()
                )
                .await
                .unwrap(),
            consumed
        );
        assert!(
            ports
                .reader
                .pending_deliveries(session, None, 20)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            ports
                .reader
                .history(session, None, 20)
                .await
                .unwrap()
                .entries
                .len(),
            2
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn deterministic_membership_model_keeps_branch_history_and_foreign_keys_consistent() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let first = ports
            .sessions
            .create(RequestId::new(), 1, "model".into())
            .await
            .unwrap()
            .value
            .id;
        let mut branches = vec![(first, Vec::<EntryId>::new())];
        let mut seed = 0x20261018u64;
        for step in 0..256 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let index = (seed as usize) % branches.len();
            let (session, history) = &branches[index];
            let session = *session;
            match (seed >> 32) % 5 {
                0 | 1 => {
                    let sent = ports
                        .sessions
                        .send(
                            RequestId::new(),
                            session,
                            document(&format!("model step {step}")),
                        )
                        .await
                        .unwrap()
                        .value;
                    ports
                        .sessions
                        .cancel(RequestId::new(), session, sent.id)
                        .await
                        .unwrap();
                    branches[index].1.push(sent.message_id);
                }
                2 if !history.is_empty() => {
                    let cut = (seed as usize) % history.len();
                    let target = history[cut];
                    let expected = history[..=cut].to_vec();
                    let child = ports
                        .sessions
                        .fork(RequestId::new(), session, target)
                        .await
                        .unwrap()
                        .value
                        .id;
                    branches.push((child, expected));
                }
                3 if !history.is_empty() => {
                    let target = *history.last().unwrap();
                    let owned = ports
                        .reader
                        .entry(session, target)
                        .await
                        .unwrap()
                        .origin_session_id
                        == session;
                    if owned {
                        ports
                            .sessions
                            .withdraw(RequestId::new(), session, target)
                            .await
                            .unwrap();
                        branches[index].1.pop();
                    }
                }
                4 if branches.len() > 1 => {
                    ports
                        .sessions
                        .delete(RequestId::new(), session)
                        .await
                        .unwrap();
                    branches.remove(index);
                }
                _ => {}
            }
            for (session, expected) in &branches {
                let actual = ports.reader.history(*session, None, 200).await.unwrap();
                assert_eq!(
                    actual
                        .entries
                        .iter()
                        .map(|entry| entry.id)
                        .collect::<Vec<_>>(),
                    *expected,
                    "seed=0x20261018 step={step}"
                );
            }
            let violations = sqlx::query("PRAGMA foreign_key_check")
                .fetch_all(admin.get_sqlite_connection_pool())
                .await
                .unwrap();
            assert!(violations.is_empty(), "seed=0x20261018 step={step}");
        }
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn delivery_resume_queues_its_continuation_atomically_and_preserves_replayed_generation() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "resume".into())
            .await
            .unwrap()
            .value
            .id;
        let sent = ports
            .sessions
            .send(RequestId::new(), session, document("launch"))
            .await
            .unwrap()
            .value;
        ports
            .sessions
            .cancel(RequestId::new(), session, sent.id)
            .await
            .unwrap();
        let delivery = ports
            .executor
            .deliver(
                RequestId::new(),
                session,
                "task:resume".into(),
                serde_json::json!({"text":"finished"}),
            )
            .await
            .unwrap()
            .value;
        tests::sql(&admin,"CREATE TRIGGER fail_resume_notice BEFORE INSERT ON agena_entries WHEN NEW.kind='notice' BEGIN SELECT RAISE(ABORT,'notice failed'); END").await;
        let request = RequestId::new();
        assert!(
            ports
                .executor
                .resume_delivery(
                    request,
                    session,
                    delivery.id,
                    sent.message_id,
                    "finished".into()
                )
                .await
                .is_err()
        );
        assert_eq!(
            ports
                .reader
                .pending_deliveries(session, None, 10)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            ports
                .reader
                .execution(session, sent.id)
                .await
                .unwrap()
                .generation,
            sent.generation
        );
        tests::sql(&admin, "DROP TRIGGER fail_resume_notice").await;
        let resumed = ports
            .executor
            .resume_delivery(
                request,
                session,
                delivery.id,
                sent.message_id,
                "finished".into(),
            )
            .await
            .unwrap();
        assert_eq!(resumed.value.phase, ExecutionPhase::Queued);
        assert_eq!(resumed.value.generation, sent.generation.next().unwrap());
        assert!(
            ports
                .reader
                .pending_deliveries(session, None, 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            ports
                .executor
                .resume_delivery(
                    request,
                    session,
                    delivery.id,
                    sent.message_id,
                    "finished".into()
                )
                .await
                .unwrap(),
            resumed
        );
        let again = ports
            .executor
            .resume_delivery(
                RequestId::new(),
                session,
                delivery.id,
                sent.message_id,
                "finished".into(),
            )
            .await
            .unwrap()
            .value;
        assert_eq!(again, resumed.value);
        let history = ports
            .reader
            .history_after(session, Some(sent.message_id), 10)
            .await
            .unwrap();
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].generation, resumed.value.generation);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn fork_from_first_assistant_entry_includes_the_selected_complete_response() {
    let (ports, _admin, _root, _url) = fixture(true).await;
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "message boundary".into())
        .await
        .unwrap()
        .value
        .id;
    let sent = ports
        .sessions
        .send(RequestId::new(), session, document("request"))
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
    let first = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    first.append("first").await.unwrap();
    first.finish(RequestId::new()).await.unwrap();
    let last = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    last.append("last").await.unwrap();
    last.finish(RequestId::new()).await.unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let child = ports
        .sessions
        .fork(RequestId::new(), session, first.entry_id())
        .await
        .unwrap()
        .value
        .id;
    let history = ports.reader.history(child, None, 10).await.unwrap();
    assert_eq!(
        history
            .entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        [sent.message_id, first.entry_id(), last.entry_id()]
    );
    assert_eq!(history.entries[1].response_entry_id, Some(first.entry_id()));
    assert_eq!(history.entries[2].response_entry_id, Some(first.entry_id()));
    ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn compaction_policy_is_durable_scoped_and_does_not_rewrite_prompt_boundaries() {
    let (ports, _admin, root, _url) = fixture(false).await;
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "compaction policy".into())
        .await
        .unwrap()
        .value
        .id;
    let sent = ports
        .sessions
        .send(RequestId::new(), session, document("request"))
        .await
        .unwrap()
        .value;
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, sent.id)
        .await
        .unwrap()
        .value;
    let initial = ports.reader.prompt_window_status(session).await.unwrap();
    let generation = initial.generation;
    drop(initial);
    let request = RequestId::new();
    let first = attempt
        .record_compaction(request, "policy-a".into(), CompactionPolicyOutcome::Failed)
        .await
        .unwrap();
    assert_eq!(first.value.consecutive_compaction_failures, 1);
    assert_eq!(
        attempt
            .record_compaction(request, "policy-a".into(), CompactionPolicyOutcome::Failed)
            .await
            .unwrap(),
        first
    );
    for _ in 0..4 {
        attempt
            .record_compaction(
                RequestId::new(),
                "policy-a".into(),
                CompactionPolicyOutcome::Failed,
            )
            .await
            .unwrap();
    }
    let disabled = ports.reader.prompt_window_status(session).await.unwrap();
    assert_eq!(disabled.consecutive_compaction_failures, 3);
    assert!(disabled.auto_compaction_disabled);
    assert_eq!(disabled.generation, generation);
    assert_eq!(disabled.checkpoint_entry_id, None);
    drop(disabled);
    let reset = attempt
        .record_compaction(
            RequestId::new(),
            "policy-b".into(),
            CompactionPolicyOutcome::NativeUnavailable,
        )
        .await
        .unwrap()
        .value;
    assert_eq!(reset.consecutive_compaction_failures, 0);
    assert!(!reset.auto_compaction_disabled);
    assert!(reset.native_compaction_unavailable);
    let local = attempt
        .record_compaction(
            RequestId::new(),
            "policy-b".into(),
            CompactionPolicyOutcome::Succeeded {
                native: false,
                before_tokens: Some(9000),
                after_tokens: Some(2000),
            },
        )
        .await
        .unwrap()
        .value;
    assert!(local.native_compaction_unavailable);
    assert_eq!(local.last_before_tokens, Some(9000));
    let native = attempt
        .record_compaction(
            RequestId::new(),
            "policy-b".into(),
            CompactionPolicyOutcome::Succeeded {
                native: true,
                before_tokens: Some(2000),
                after_tokens: Some(900),
            },
        )
        .await
        .unwrap()
        .value;
    assert!(!native.native_compaction_unavailable);
    assert_eq!(native.last_after_tokens, Some(900));
    assert_eq!(native.generation, generation);
    let path = root.path().join("policy.jsonl");
    let mut bytes = Vec::new();
    ports
        .reader
        .export_jsonl(session, &mut bytes)
        .await
        .unwrap();
    tokio::fs::write(&path, bytes).await.unwrap();
    let imported = ports
        .sessions
        .import_archive(RequestId::new(), 1, &path)
        .await
        .unwrap()
        .value
        .id;
    assert_eq!(
        *ports.reader.prompt_window_status(imported).await.unwrap(),
        native
    );
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    assert!(
        attempt
            .record_compaction(
                RequestId::new(),
                "policy-b".into(),
                CompactionPolicyOutcome::Failed
            )
            .await
            .is_err()
    );
    ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn tool_interaction_history_has_real_identity_time_and_a_bounded_durable_limit() {
    let (ports, _admin, _root, _url) = fixture(true).await;
    let (session, _round, tool) = event_tests::tool(&ports).await;
    let mut ids = Vec::new();
    for index in 0..128 {
        let id = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: format!("approval-{index}"),
                    description: "approval fixture".into(),
                },
            )
            .await
            .unwrap()
            .value;
        ids.push(id);
        let before = ports.reader.interaction(session, id).await.unwrap();
        let created = before.created_at_ms;
        assert!(created > 0);
        drop(before);
        ports
            .sessions
            .reply(
                RequestId::new(),
                session,
                id,
                super::tests::permission_decision(id, true, None),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .interaction(session, id)
                .await
                .unwrap()
                .created_at_ms,
            created
        );
    }
    let view = ports.reader.tool(session, tool.id()).await.unwrap();
    assert_eq!(view.interactions, ids);
    assert!(
        matches!(ports.reader.entry(session,view.entry_id).await.unwrap().payload,EntryPayload::Tool{tool_id}if tool_id==tool.id())
    );
    drop(view);
    assert!(
        tool.request_interaction(
            RequestId::new(),
            InteractionRequest::Permission {
                action_key: "over-limit".into(),
                description: "approval fixture".into()
            }
        )
        .await
        .is_err()
    );
    ports.shutdown().await.unwrap();
}
