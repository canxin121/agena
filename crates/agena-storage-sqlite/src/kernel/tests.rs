use std::{collections::BTreeSet, future::Future, task::Poll};

use agena_domain::{
    ComposerDocument, ComposerNode, ExecutionSelection, PermissionMode, PermissionScope,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

use super::*;

pub(super) fn document(text: &str) -> ComposerDocument {
    ComposerDocument(vec![ComposerNode::Text { text: text.into() }])
}

pub(super) fn permission_decision(
    interaction: InteractionId,
    allowed: bool,
    remember: Option<&RememberPermission>,
) -> InteractionDecision {
    use agena_domain::{PermissionReply, PermissionReplyKind};
    InteractionDecision::Permission {
        reply: PermissionReply {
            request_id: interaction.get().to_string(),
            kind: match (allowed, remember.is_some()) {
                (true, false) => PermissionReplyKind::AllowOnce,
                (true, true) => PermissionReplyKind::AllowAlways,
                (false, false) => PermissionReplyKind::DenyOnce,
                (false, true) => PermissionReplyKind::DenyAlways,
            },
            reason: None,
            scope: remember.map(|rule| rule.scope),
        },
        allowed,
    }
}

pub(super) async fn fixture(
    memory: bool,
) -> (
    KernelPorts,
    Arc<sea_orm::DatabaseConnection>,
    tempfile::TempDir,
    String,
) {
    fixture_with_diagnostics(memory, KernelDiagnostics::default()).await
}

pub(super) async fn fixture_with_diagnostics(
    memory: bool,
    diagnostics: KernelDiagnostics,
) -> (
    KernelPorts,
    Arc<sea_orm::DatabaseConnection>,
    tempfile::TempDir,
    String,
) {
    let root = tempfile::tempdir().unwrap();
    let url = if memory {
        "sqlite::memory:".into()
    } else {
        format!(
            "sqlite://{}?mode=rwc",
            root.path().join("kernel.db").display()
        )
    };
    let database = KernelDatabase::open(&url).await.unwrap();
    let admin = database.writer.clone();
    let ports = open_database(database, diagnostics).await.unwrap();
    admin
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "INSERT INTO agena_workspaces (id,path,created_at_ms,updated_at_ms) VALUES (1,?,0,0)",
            [root.path().to_string_lossy().into_owned().into()],
        ))
        .await
        .unwrap();
    (ports, admin, root, url)
}

pub(super) async fn sql(database: &sea_orm::DatabaseConnection, text: &str) {
    database
        .execute(Statement::from_string(DatabaseBackend::Sqlite, text))
        .await
        .unwrap();
}

#[tokio::test]
async fn full_database_rolls_back_new_batch_work_and_preserves_an_earlier_durable_replay() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "full batch".into())
            .await
            .unwrap()
            .value
            .id;
        let request = RequestId::new();
        let input = document("durable before quota");
        let sent = ports
            .sessions
            .send(request, session, input.clone())
            .await
            .unwrap();
        let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        sql(&admin, &format!("PRAGMA max_page_count={}", pages + 2)).await;
        let replay = ports.sessions.send(request, session, input);
        let newer = ports.sessions.send(
            RequestId::new(),
            session,
            document(&"big new message".repeat(8192)),
        );
        let (replayed, failed) = tokio::join!(biased;replay,newer);
        assert_eq!(replayed.unwrap(), sent);
        assert!(failed.is_err());
        sql(&admin, "PRAGMA max_page_count=2147483647").await;
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
        ports
            .sessions
            .send(RequestId::new(), session, document("writer recovered"))
            .await
            .unwrap();
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn a_durable_replay_keeps_its_receipt_when_an_unrelated_batch_commit_is_rejected() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "original".into())
            .await
            .unwrap()
            .value
            .id;
        let request = RequestId::new();
        let input = document("already committed");
        let sent = ports
            .sessions
            .send(request, session, input.clone())
            .await
            .unwrap();
        sql(&admin,"CREATE TABLE injected_failure (id INTEGER REFERENCES agena_entries(id) DEFERRABLE INITIALLY DEFERRED)").await;
        sql(&admin,"CREATE TRIGGER reject_new AFTER INSERT ON agena_command_results BEGIN INSERT INTO injected_failure VALUES (-1); END").await;
        let replay = ports.sessions.send(request, session, input);
        let change = ports.sessions.update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                title: Some("must roll back".into()),
                ..Default::default()
            },
        );
        let (replayed, changed) = tokio::join!(biased;replay,change);
        assert_eq!(replayed.unwrap(), sent);
        assert!(matches!(changed, Err(KernelError::Storage(_))));
        assert_eq!(
            ports.reader.metadata(session).await.unwrap().title,
            "original"
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
        sql(&admin, "DROP TRIGGER reject_new").await;
        sql(&admin, "DROP TABLE injected_failure").await;
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn identical_requests_in_one_batch_publish_only_one_membership_change() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "replay".into())
            .await
            .unwrap()
            .value
            .id;
        let mut watch = ports.reader.watch(session, None).unwrap();
        let request = RequestId::new();
        let first = ports.sessions.send(request, session, document("once"));
        let second = ports.sessions.send(request, session, document("once"));
        let (first, second) = tokio::join!(biased;first,second);
        let first = first.unwrap();
        assert_eq!(first, second.unwrap());
        assert_eq!(watch.recv().await.unwrap().cursor.position, first.position);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), watch.recv())
                .await
                .is_err()
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
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn public_command_flow_is_atomic_idempotent_and_identical_for_file_and_memory() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "session".into())
            .await
            .unwrap()
            .value
            .id;
        let request = RequestId::new();
        let input = document("  hello 世界\n\0");
        let sent = ports
            .sessions
            .send(request, session, input.clone())
            .await
            .unwrap();
        assert_eq!(
            sent,
            ports
                .sessions
                .send(request, session, input.clone())
                .await
                .unwrap()
        );
        assert!(matches!(
            ports
                .sessions
                .send(request, session, document("different"))
                .await,
            Err(KernelError::Conflict(_))
        ));
        let attempt = ports
            .executor
            .claim(RequestId::new(), session, sent.value.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_round(RequestId::new(), vec![sent.value.message_id])
            .await
            .unwrap()
            .value;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "call-1".into(),
                    name: "shell.exec".into(),
                    input: serde_json::json!({"command":"fixture"}),
                },
            )
            .await
            .unwrap()
            .value;
        let interaction = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: "shell.exec".into(),
                    description: "fixture permission".into(),
                },
            )
            .await
            .unwrap()
            .value;
        let remember = Some(RememberPermission {
            mode: PermissionMode::Allow,
            scope: PermissionScope::Session,
        });
        let request = RequestId::new();
        let replied = ports
            .sessions
            .reply(
                request,
                session,
                interaction,
                super::tests::permission_decision(interaction, true, remember.as_ref()),
                remember.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            replied,
            ports
                .sessions
                .reply(
                    request,
                    session,
                    interaction,
                    super::tests::permission_decision(interaction, true, remember.as_ref()),
                    remember.clone()
                )
                .await
                .unwrap()
        );
        ports
            .sessions
            .reply(
                RequestId::new(),
                session,
                interaction,
                super::tests::permission_decision(interaction, true, remember.as_ref()),
                remember,
            )
            .await
            .unwrap();
        assert!(matches!(
            ports
                .sessions
                .reply(
                    RequestId::new(),
                    session,
                    interaction,
                    super::tests::permission_decision(interaction, false, None),
                    None
                )
                .await,
            Err(KernelError::Conflict(_))
        ));
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"exit_code":0,"exact":"\u{1b}[31m\r\n\0"}),
            },
        )
        .await
        .unwrap();
        let opaque = serde_json::json!({"items":[{"encrypted_content":"opaque\n世界\0"}]});
        round
            .finish(RequestId::new(), Some(opaque.clone()), true)
            .await
            .unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .execution(session, sent.value.id)
                .await
                .unwrap()
                .phase,
            ExecutionPhase::Succeeded
        );
        assert_eq!(
            ports
                .reader
                .round(session, round.id())
                .await
                .unwrap()
                .provider_state,
            Some(opaque)
        );
        assert_eq!(
            ports.reader.tool(session, tool.id()).await.unwrap().phase,
            ToolPhase::Succeeded
        );
        assert_eq!(
            ports
                .reader
                .interaction(session, interaction)
                .await
                .unwrap()
                .decision,
            Some(permission_decision(
                interaction,
                true,
                Some(&RememberPermission {
                    mode: PermissionMode::Allow,
                    scope: PermissionScope::Session
                })
            ))
        );
        let mut before = None;
        let mut entries = Vec::new();
        loop {
            let page = ports.reader.history(session, before, 2).await.unwrap();
            entries.extend(page.entries.iter().cloned());
            if !page.has_more {
                break;
            }
            before = page.next;
        }
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().any(
            |entry| matches!(&entry.payload,EntryPayload::User { document } if document==&input)
        ));
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.id)
                .collect::<BTreeSet<_>>()
                .len(),
            3
        );
        drop((attempt, round, tool, admin));
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn cancelled_attempt_cannot_overwrite_or_enter_a_retry_or_foreign_session() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "a".into())
            .await
            .unwrap()
            .value
            .id;
        let foreign = ports
            .sessions
            .create(RequestId::new(), 1, "b".into())
            .await
            .unwrap()
            .value
            .id;
        let execution = ports
            .sessions
            .send(RequestId::new(), session, document("go"))
            .await
            .unwrap()
            .value;
        assert!(matches!(
            ports
                .executor
                .claim(RequestId::new(), foreign, execution.id)
                .await,
            Err(KernelError::Ownership)
        ));
        let attempt = ports
            .executor
            .claim(RequestId::new(), session, execution.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_round(RequestId::new(), vec![execution.message_id])
            .await
            .unwrap()
            .value;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "start".into(),
                    name: "fixture".into(),
                    input: serde_json::json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        ports
            .sessions
            .cancel(RequestId::new(), session, execution.id)
            .await
            .unwrap();
        assert!(matches!(
            attempt
                .finish(RequestId::new(), ExecutionPhase::Succeeded)
                .await,
            Err(KernelError::Conflict(_))
        ));
        assert_eq!(
            ports
                .reader
                .execution(session, execution.id)
                .await
                .unwrap()
                .phase,
            ExecutionPhase::Cancelled
        );
        let retried = ports
            .sessions
            .retry(RequestId::new(), session, execution.id)
            .await
            .unwrap();
        assert!(retried.value.generation > attempt.generation());
        let next = ports
            .executor
            .claim(RequestId::new(), session, execution.id)
            .await
            .unwrap()
            .value;
        assert!(matches!(
            attempt.begin_round(RequestId::new(), vec![]).await,
            Err(KernelError::StaleAttempt)
        ));
        assert!(matches!(
            tool.complete(
                RequestId::new(),
                ToolResult {
                    phase: ToolPhase::Succeeded,
                    value: serde_json::json!(true)
                }
            )
            .await,
            Err(KernelError::StaleAttempt)
        ));
        next.finish(RequestId::new(), ExecutionPhase::Interrupted)
            .await
            .unwrap();
        drop((admin, attempt, round, tool, next));
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn permission_rule_and_decision_roll_back_together_at_the_middle_failure_window() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "permission".into())
            .await
            .unwrap()
            .value
            .id;
        let execution = ports
            .sessions
            .send(RequestId::new(), session, document("run"))
            .await
            .unwrap()
            .value;
        let attempt = ports
            .executor
            .claim(RequestId::new(), session, execution.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_round(RequestId::new(), vec![])
            .await
            .unwrap()
            .value;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "p".into(),
                    name: "fixture".into(),
                    input: serde_json::json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        let interaction = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: "fixture".into(),
                    description: "test".into(),
                },
            )
            .await
            .unwrap()
            .value;
        sql(&admin,"CREATE TRIGGER fail_decision BEFORE UPDATE OF decision_json ON agena_interactions BEGIN SELECT RAISE(ABORT,'injected decision failure'); END").await;
        let remember = Some(RememberPermission {
            mode: PermissionMode::Allow,
            scope: PermissionScope::Session,
        });
        let request = RequestId::new();
        assert!(
            ports
                .sessions
                .reply(
                    request,
                    session,
                    interaction,
                    super::tests::permission_decision(interaction, true, remember.as_ref()),
                    remember.clone()
                )
                .await
                .is_err()
        );
        let row = admin.query_one(Statement::from_string(DatabaseBackend::Sqlite,
            "SELECT (SELECT count(*) FROM agena_permission_rules) AS rules,(SELECT count(*) FROM agena_interactions WHERE decision_json IS NOT NULL) AS decisions")).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i64>("", "rules").unwrap(), 0);
        assert_eq!(row.try_get::<i64>("", "decisions").unwrap(), 0);
        assert!(
            ports
                .reader
                .interaction(session, interaction)
                .await
                .unwrap()
                .decision
                .is_none()
        );
        sql(&admin, "DROP TRIGGER fail_decision").await;
        ports
            .sessions
            .reply(
                request,
                session,
                interaction,
                super::tests::permission_decision(interaction, true, remember.as_ref()),
                remember,
            )
            .await
            .unwrap();
        drop((admin, attempt, round, tool));
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_commands_share_real_transactions_without_losing_independent_fields() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "batch".into())
            .await
            .unwrap()
            .value
            .id;
        let gate = Arc::new(tokio::sync::Barrier::new(34));
        let mut tasks = Vec::new();
        for index in 0..33 {
            let sessions = ports.sessions.clone();
            let gate = gate.clone();
            tasks.push(tokio::spawn(async move {
                gate.wait().await;
                if index == 0 {
                    sessions
                        .update_metadata(
                            RequestId::new(),
                            session,
                            MetadataChange {
                                title: Some("renamed".into()),
                                ..Default::default()
                            },
                        )
                        .await
                        .map(|r| r.position)
                } else if index == 1 {
                    sessions
                        .update_metadata(
                            RequestId::new(),
                            session,
                            MetadataChange {
                                favorite: Some(true),
                                ..Default::default()
                            },
                        )
                        .await
                        .map(|r| r.position)
                } else if index == 2 {
                    sessions
                        .send(
                            RequestId::new(),
                            SessionId::try_from(9999).unwrap(),
                            document("invalid owner"),
                        )
                        .await
                        .map(|r| r.position)
                } else {
                    sessions
                        .send(
                            RequestId::new(),
                            session,
                            document(&format!("message-{index}")),
                        )
                        .await
                        .map(|r| r.position)
                }
            }));
        }
        gate.wait().await;
        let mut positions = Vec::new();
        let mut errors = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(position) => positions.push(position),
                Err(KernelError::NotFound(_)) => errors += 1,
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(errors, 1);
        assert_eq!(positions.len(), 32);
        assert_eq!(positions.iter().copied().collect::<BTreeSet<_>>().len(), 32);
        assert!(
            positions
                .iter()
                .map(|p| p.sequence)
                .collect::<BTreeSet<_>>()
                .len()
                < 32,
            "there must be actual multi-command transactions"
        );
        let metadata = ports.reader.metadata(session).await.unwrap();
        assert_eq!(metadata.title, "renamed");
        assert!(metadata.favorite);
        assert_eq!(
            ports
                .reader
                .history(session, None, 100)
                .await
                .unwrap()
                .entries
                .len(),
            30
        );
        drop(admin);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn losing_the_response_future_preserves_the_committed_identity_and_receipt() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "lost-response".into())
            .await
            .unwrap()
            .value
            .id;
        let request = RequestId::new();
        let mut response = Box::pin(
            ports
                .sessions
                .send(request, session, document("one message")),
        );
        std::future::poll_fn(|cx| {
            assert!(response.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(response);
        let replay = ports
            .sessions
            .send(request, session, document("one message"))
            .await
            .unwrap();
        assert_eq!(
            replay,
            ports
                .sessions
                .send(request, session, document("one message"))
                .await
                .unwrap()
        );
        assert_eq!(
            ports
                .reader
                .history(session, None, 100)
                .await
                .unwrap()
                .entries
                .len(),
            1
        );
        drop(admin);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn selection_changes_merge_fields_and_history_cursors_are_scope_and_generation_bound() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "fields".into())
            .await
            .unwrap()
            .value
            .id;
        let (first, second) = tokio::join!(
            ports.sessions.update_selection(
                RequestId::new(),
                session,
                SelectionChange::ThinkingMode(Some("high".into()))
            ),
            ports.sessions.update_selection(
                RequestId::new(),
                session,
                SelectionChange::Verbosity(Some("low".into()))
            )
        );
        first.unwrap();
        second.unwrap();
        let selection = ports
            .reader
            .metadata(session)
            .await
            .unwrap()
            .selection
            .clone()
            .unwrap();
        assert_eq!(
            selection,
            ExecutionSelection {
                thinking_mode: Some("high".into()),
                verbosity: Some("low".into()),
                ..Default::default()
            }
        );
        for index in 0..3 {
            ports
                .sessions
                .send(RequestId::new(), session, document(&format!("{index}")))
                .await
                .unwrap();
        }
        let page = ports.reader.history(session, None, 1).await.unwrap();
        let cursor = page.next.unwrap();
        ports
            .sessions
            .send(RequestId::new(), session, document("changes membership"))
            .await
            .unwrap();
        assert!(matches!(
            ports.reader.history(session, Some(cursor), 1).await,
            Err(KernelError::Conflict(_))
        ));
        drop(admin);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn commit_failure_never_issues_a_success_receipt_and_retry_does_not_duplicate_facts() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "commit-failure".into())
            .await
            .unwrap()
            .value
            .id;
        sql(&admin,"CREATE TABLE injected_failure (id INTEGER REFERENCES agena_sessions(id) DEFERRABLE INITIALLY DEFERRED)").await;
        sql(&admin,"CREATE TRIGGER fail_commit AFTER INSERT ON agena_command_results BEGIN INSERT INTO injected_failure VALUES (-1); END").await;
        let request = RequestId::new();
        assert!(matches!(
            ports
                .sessions
                .send(request, session, document("retry this fact"))
                .await,
            Err(KernelError::Storage(_))
        ));
        assert!(
            ports
                .reader
                .history(session, None, 100)
                .await
                .unwrap()
                .entries
                .is_empty()
        );
        sql(&admin, "DROP TRIGGER fail_commit").await;
        sql(&admin, "DROP TABLE injected_failure").await;
        let committed = ports
            .sessions
            .send(request, session, document("retry this fact"))
            .await
            .unwrap();
        assert_eq!(
            committed,
            ports
                .sessions
                .send(request, session, document("retry this fact"))
                .await
                .unwrap()
        );
        assert_eq!(
            ports
                .reader
                .history(session, None, 100)
                .await
                .unwrap()
                .entries
                .len(),
            1
        );
        drop(admin);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn reopening_recovers_inflight_work_without_repeating_it_and_keeps_receipts_exact() {
    let (ports, admin, _root, url) = fixture(false).await;
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "restart".into())
        .await
        .unwrap()
        .value
        .id;
    let request = RequestId::new();
    let sent = ports
        .sessions
        .send(request, session, document("recover me"))
        .await
        .unwrap();
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, sent.value.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_round(RequestId::new(), vec![])
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "real-start".into(),
                name: "fixture".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let interaction = tool
        .request_interaction(
            RequestId::new(),
            InteractionRequest::Permission {
                action_key: "fixture".into(),
                description: "await reply".into(),
            },
        )
        .await
        .unwrap()
        .value;
    let tool_id = tool.id();
    let old_cursor = ports.reader.watch(session, None).unwrap().head().cloned();
    drop((admin, attempt, round, tool));
    ports.shutdown().await.unwrap();
    let reopened = open_session_kernel(&url).await.unwrap();
    assert_eq!(
        reopened
            .reader
            .execution(session, sent.value.id)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Interrupted
    );
    assert_eq!(
        reopened
            .sessions
            .send(request, session, document("recover me"))
            .await
            .unwrap(),
        sent
    );
    assert_eq!(
        reopened
            .reader
            .history(session, None, 100)
            .await
            .unwrap()
            .entries
            .len(),
        3
    );
    assert_eq!(
        reopened.reader.tool(session, tool_id).await.unwrap().phase,
        ToolPhase::Interrupted
    );
    assert!(
        reopened
            .reader
            .watch(session, old_cursor)
            .unwrap()
            .needs_snapshot()
    );
    assert!(matches!(
        reopened
            .sessions
            .reply(
                RequestId::new(),
                session,
                interaction,
                super::tests::permission_decision(interaction, true, None),
                None
            )
            .await,
        Err(KernelError::Conflict(_))
    ));
    reopened
        .sessions
        .reply(
            RequestId::new(),
            session,
            interaction,
            InteractionDecision::Cancel,
            None,
        )
        .await
        .unwrap();
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn bounded_history_crosses_segment_boundaries_without_retaining_obsolete_tail_copies() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "segments".into())
            .await
            .unwrap()
            .value
            .id;
        let mut expected = BTreeSet::new();
        for index in 0..257 {
            expected.insert(
                ports
                    .sessions
                    .send(RequestId::new(), session, document(&format!("{index}")))
                    .await
                    .unwrap()
                    .value
                    .message_id,
            );
        }
        let mut before = None;
        let mut actual = BTreeSet::new();
        loop {
            let page = ports.reader.history(session, before, 31).await.unwrap();
            assert!(page.entries.len() <= 31);
            assert!(page.entries.windows(2).all(|pair| pair[0].id < pair[1].id));
            for entry in &page.entries {
                assert!(actual.insert(entry.id));
            }
            if !page.has_more {
                break;
            }
            before = page.next;
        }
        assert_eq!(actual, expected);
        let summary = ports.reader.summary(session).await.unwrap();
        assert_eq!(summary.entry_count, 257);
        assert_eq!(summary.user_message_count, 257);
        assert_eq!(summary.queued_executions, 257);
        let row=admin.query_one(Statement::from_string(DatabaseBackend::Sqlite,"SELECT (SELECT count(*) FROM agena_history_segments) AS segments,(SELECT count(*) FROM agena_segment_entries) AS members")).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i64>("", "segments").unwrap(), 3);
        assert_eq!(row.try_get::<i64>("", "members").unwrap(), 257);
        drop(admin);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn watch_covers_the_snapshot_race_and_idempotent_replay_does_not_publish_twice() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "watch".into())
            .await
            .unwrap()
            .value
            .id;
        let mut watch = ports.reader.watch(session, None).unwrap();
        assert!(watch.needs_snapshot());
        let _snapshot = ports.reader.metadata(session).await.unwrap();
        let request = RequestId::new();
        let receipt = ports
            .sessions
            .send(request, session, document("after subscription"))
            .await
            .unwrap();
        let change = watch.recv().await.unwrap();
        assert_eq!(change.cursor.position, receipt.position);
        assert_eq!(change.kind, ChangeKind::Membership);
        let mut resumed = ports
            .reader
            .watch(session, Some(change.cursor.clone()))
            .unwrap();
        assert!(!resumed.needs_snapshot());
        let renamed = ports
            .sessions
            .update_metadata(
                RequestId::new(),
                session,
                MetadataChange {
                    title: Some("changed".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            resumed.recv().await.unwrap().cursor.position,
            renamed.position
        );
        ports
            .sessions
            .send(request, session, document("after subscription"))
            .await
            .unwrap();
        let mut next = Box::pin(resumed.recv());
        std::future::poll_fn(|cx| {
            assert!(next.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(next);
        drop((watch, resumed, admin));
        ports.shutdown().await.unwrap();
    }
}
