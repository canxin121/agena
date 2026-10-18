use std::collections::BTreeSet;

use super::{
    tests::{document, fixture, fixture_with_diagnostics, sql},
    *,
};

fn counts(summary: &SessionSummaryView) -> (u64, u64, u64, u64) {
    (
        summary.entry_count,
        summary.user_message_count,
        summary.queued_executions,
        summary.running_executions,
    )
}

#[tokio::test]
async fn temporary_children_never_count_in_parent_summaries_and_visibility_changes_invalidate_cache()
 {
    use agena_domain::{ConversationMode, SessionConversation, SessionRelationKind};
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let parent = ports
            .sessions
            .create(RequestId::new(), 1, "parent".into())
            .await
            .unwrap()
            .value
            .id;
        let child = ports
            .sessions
            .create_configured(
                RequestId::new(),
                1,
                "visible child".into(),
                SessionConfiguration {
                    parent_session_id: Some(parent),
                    relation_kind: SessionRelationKind::Child,
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let warm = ports.reader.summary(parent).await.unwrap();
        assert_eq!(warm.child_session_count, 1);
        assert!(warm.ptr_eq(&ports.reader.summary(parent).await.unwrap()));
        let temporary = SessionConversation {
            mode: ConversationMode::Btw,
            parent_session_id: parent.get(),
        };
        ports
            .sessions
            .configure(
                RequestId::new(),
                child,
                SessionConfigurationChange {
                    conversation: Some(Some(temporary.clone())),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let hidden = ports.reader.summary(parent).await.unwrap();
        assert_eq!(hidden.child_session_count, 0);
        assert!(!warm.ptr_eq(&hidden));
        // Identical configuration must not drift the projected count.
        ports
            .sessions
            .configure(
                RequestId::new(),
                child,
                SessionConfigurationChange {
                    conversation: Some(Some(temporary.clone())),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .summary(parent)
                .await
                .unwrap()
                .child_session_count,
            0
        );
        ports
            .sessions
            .configure(
                RequestId::new(),
                child,
                SessionConfigurationChange {
                    conversation: Some(Some(SessionConversation {
                        mode: ConversationMode::Side,
                        parent_session_id: parent.get(),
                    })),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .summary(parent)
                .await
                .unwrap()
                .child_session_count,
            1
        );
        let initially_temporary = ports
            .sessions
            .create_configured(
                RequestId::new(),
                1,
                "born hidden".into(),
                SessionConfiguration {
                    parent_session_id: Some(parent),
                    relation_kind: SessionRelationKind::Child,
                    conversation: Some(temporary),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        assert_eq!(
            ports
                .reader
                .summary(parent)
                .await
                .unwrap()
                .child_session_count,
            1
        );
        ports
            .sessions
            .delete(RequestId::new(), initially_temporary)
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .summary(parent)
                .await
                .unwrap()
                .child_session_count,
            1
        );
        ports
            .sessions
            .delete(RequestId::new(), child)
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .summary(parent)
                .await
                .unwrap()
                .child_session_count,
            0
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn conversation_forks_publish_their_final_visibility_in_the_first_committed_change() {
    use agena_domain::{ConversationMode, SessionConversation};
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let parent = ports
            .sessions
            .create(RequestId::new(), 1, "parent".into())
            .await
            .unwrap()
            .value
            .id;
        let source = ports
            .sessions
            .send(RequestId::new(), parent, document("shared context"))
            .await
            .unwrap()
            .value;
        ports
            .sessions
            .cancel(RequestId::new(), parent, source.id)
            .await
            .unwrap();
        for mode in [ConversationMode::Btw, ConversationMode::Side] {
            let visible = mode == ConversationMode::Side;
            let warm_parent = ports.reader.summary(parent).await.unwrap();
            assert_eq!(warm_parent.child_session_count, 0);
            let mut watch = ports.reader.watch_all(None).unwrap();
            let request = RequestId::new();
            let fork = ports
                .sessions
                .fork_conversation(request, parent, source.message_id, mode)
                .await
                .unwrap();
            let child = fork.value.id;
            let created = watch.recv().await.unwrap();
            assert_eq!(created.session, child);
            assert_eq!(created.include_in_lists, visible);
            let list = created.list_change.as_ref().unwrap();
            assert!(list.before.is_none());
            assert_eq!(list.after.as_ref().unwrap().include_in_lists, visible);
            assert_eq!(
                ports
                    .reader
                    .configuration(child)
                    .await
                    .unwrap()
                    .conversation,
                Some(SessionConversation {
                    mode,
                    parent_session_id: parent.get()
                })
            );
            assert_eq!(
                ports
                    .reader
                    .summary(parent)
                    .await
                    .unwrap()
                    .child_session_count,
                u64::from(visible)
            );
            let replay = ports
                .sessions
                .fork_conversation(request, parent, source.message_id, mode)
                .await
                .unwrap();
            assert_eq!(fork.position, replay.position);
            assert_eq!(child, replay.value.id);
            assert!(watch.try_recv().unwrap().is_none());
            let listed = ports
                .reader
                .query_summaries(SummaryQuery {
                    parent_id: Some(parent),
                    limit: 128,
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(listed.sessions.len(), usize::from(visible));
            ports
                .sessions
                .update_metadata(
                    RequestId::new(),
                    child,
                    MetadataChange {
                        title: Some("conversation title".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(watch.recv().await.unwrap().include_in_lists, visible);
            ports
                .sessions
                .delete(RequestId::new(), child)
                .await
                .unwrap();
            assert_eq!(watch.recv().await.unwrap().include_in_lists, visible);
            assert_eq!(
                ports
                    .reader
                    .summary(parent)
                    .await
                    .unwrap()
                    .child_session_count,
                0
            );
        }
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn workspace_pages_are_scoped_bounded_and_invalidated_by_committed_session_changes() {
    for memory in [true, false] {
        let diagnostics = KernelDiagnostics::default();
        let (ports, admin, _root, _url) =
            fixture_with_diagnostics(memory, diagnostics.clone()).await;
        sql(&admin, "INSERT INTO agena_workspaces (id,path,created_at_ms,updated_at_ms) VALUES (2,'summary-other-workspace',0,0)").await;
        let empty = ports.reader.summaries(1, None, 3).await.unwrap();
        let other = ports.reader.summaries(2, None, 3).await.unwrap();
        assert!(empty.sessions.is_empty() && other.sessions.is_empty());
        assert_eq!(diagnostics.read_budget().sessions, 0);
        assert_eq!(diagnostics.read_budget().workspaces, 2);
        let mut expected = BTreeSet::new();
        for index in 0..9 {
            expected.insert(
                ports
                    .sessions
                    .create(RequestId::new(), 1, format!("session {index}"))
                    .await
                    .unwrap()
                    .value
                    .id,
            );
        }
        let first = ports.reader.summaries(1, None, 3).await.unwrap();
        assert_eq!(first.sessions.len(), 3);
        assert!(!first.ptr_eq(&empty));
        assert!(other.ptr_eq(&ports.reader.summaries(2, None, 3).await.unwrap()));
        let start = diagnostics.snapshot();
        for _ in 0..32 {
            assert!(first.ptr_eq(&ports.reader.summaries(1, None, 3).await.unwrap()));
        }
        assert_eq!(
            diagnostics.snapshot().summary_queries,
            start.summary_queries
        );
        assert_eq!(
            diagnostics.snapshot().history_queries,
            start.history_queries
        );
        let mut before = None;
        let mut seen = BTreeSet::new();
        let mut last_key = None;
        loop {
            let start = diagnostics.snapshot();
            let page = ports.reader.summaries(1, before, 3).await.unwrap();
            assert!(diagnostics.snapshot().summary_rows - start.summary_rows <= 4);
            for summary in &page.sessions {
                assert_eq!(summary.workspace_id, 1);
                let key = (summary.updated_at_ms, summary.id);
                assert!(last_key.is_none_or(|last| key < last));
                last_key = Some(key);
                assert!(seen.insert(summary.id));
            }
            before = page.next;
            if !page.has_more {
                break;
            }
            assert!(before.is_some());
        }
        assert_eq!(seen, expected);
        assert!(matches!(
            ports.reader.summaries(2, first.next, 3).await,
            Err(KernelError::Ownership)
        ));
        assert!(matches!(
            ports.reader.summaries(0, None, 3).await,
            Err(KernelError::Invalid(_))
        ));
        assert!(
            ports
                .reader
                .summaries(i64::MAX, None, 3)
                .await
                .unwrap()
                .sessions
                .is_empty()
        );
        let session = *expected.first().unwrap();
        let sent = ports
            .sessions
            .send(RequestId::new(), session, document("now active"))
            .await
            .unwrap()
            .value;
        let page = ports.reader.summaries(1, None, 3).await.unwrap();
        assert_eq!(page.sessions[0].id, session);
        assert_eq!(counts(&page.sessions[0]), (1, 1, 1, 0));
        ports
            .sessions
            .cancel(RequestId::new(), session, sent.id)
            .await
            .unwrap();
        let page = ports.reader.summaries(1, None, 3).await.unwrap();
        assert_eq!(counts(&page.sessions[0]), (1, 1, 0, 0));
        sql(&admin, "CREATE TABLE rejected_workspace_summary (owner INTEGER REFERENCES agena_sessions(id) DEFERRABLE INITIALLY DEFERRED)").await;
        sql(&admin, "CREATE TRIGGER reject_workspace_summary AFTER UPDATE OF title ON agena_sessions BEGIN INSERT INTO rejected_workspace_summary VALUES (-123); END").await;
        assert!(
            ports
                .sessions
                .update_metadata(
                    RequestId::new(),
                    session,
                    MetadataChange {
                        title: Some("rejected".into()),
                        ..Default::default()
                    }
                )
                .await
                .is_err()
        );
        assert!(page.ptr_eq(&ports.reader.summaries(1, None, 3).await.unwrap()));
        sql(&admin, "DROP TRIGGER reject_workspace_summary").await;
        ports
            .sessions
            .update_metadata(
                RequestId::new(),
                session,
                MetadataChange {
                    title: Some("committed".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = ports.reader.summaries(1, None, 3).await.unwrap();
        assert_eq!(updated.sessions[0].title, "committed");
        assert!(other.ptr_eq(&ports.reader.summaries(2, None, 3).await.unwrap()));
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn workspace_pages_use_the_workspace_order_index_and_single_statistics_lookups() {
    let (ports, admin, _root, _url) = fixture(true).await;
    for before in [false, true] {
        let sql = format!("EXPLAIN QUERY PLAN {}", reads::summaries_sql(before));
        let mut query = sqlx::query(&sql).bind(1i64);
        if before {
            query = query.bind(1234i64).bind(5i64);
        }
        let rows = query
            .bind(201i64)
            .fetch_all(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        let detail = rows
            .iter()
            .map(|row| row.get::<String, _>("detail"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(detail.contains("idx_agena_sessions_workspace"), "{detail}");
        assert!(
            detail.contains("SEARCH t USING INTEGER PRIMARY KEY"),
            "{detail}"
        );
        assert!(
            !detail.contains("SCAN") && !detail.contains("TEMP B-TREE"),
            "{detail}"
        );
    }
    ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn summary_tracks_current_attempts_once_and_warm_reads_do_no_sql() {
    for memory in [true, false] {
        let diagnostics = KernelDiagnostics::default();
        let (ports, _admin, _root, _url) =
            fixture_with_diagnostics(memory, diagnostics.clone()).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "summary".into())
            .await
            .unwrap()
            .value
            .id;
        let empty = ports.reader.summary(session).await.unwrap();
        assert_eq!(counts(&empty), (0, 0, 0, 0));
        assert_eq!(empty.last_message_at_ms, None);
        let before = diagnostics.snapshot();
        for _ in 0..64 {
            assert!(empty.ptr_eq(&ports.reader.summary(session).await.unwrap()));
        }
        assert_eq!(
            diagnostics.snapshot().summary_queries,
            before.summary_queries
        );
        assert_eq!(
            diagnostics.snapshot().history_queries,
            before.history_queries
        );

        let request = RequestId::new();
        let sent = ports
            .sessions
            .send(request, session, document("one message"))
            .await
            .unwrap();
        let summary = ports.reader.summary(session).await.unwrap();
        assert_eq!(counts(&summary), (1, 1, 1, 0));
        assert!(summary.last_message_at_ms.is_some());
        assert!(summary.updated_at_ms > empty.updated_at_ms);
        assert_eq!(
            ports
                .sessions
                .send(request, session, document("one message"))
                .await
                .unwrap(),
            sent
        );
        assert!(summary.ptr_eq(&ports.reader.summary(session).await.unwrap()));
        ports
            .sessions
            .cancel(RequestId::new(), session, sent.value.id)
            .await
            .unwrap();
        let cancelled = ports.reader.summary(session).await.unwrap();
        assert_eq!(counts(&cancelled), (1, 1, 0, 0));
        ports
            .sessions
            .cancel(RequestId::new(), session, sent.value.id)
            .await
            .unwrap();
        assert_eq!(*ports.reader.summary(session).await.unwrap(), *cancelled);
        ports
            .sessions
            .retry(RequestId::new(), session, sent.value.id)
            .await
            .unwrap();
        assert_eq!(
            counts(&ports.reader.summary(session).await.unwrap()),
            (1, 1, 1, 0)
        );
        let attempt = ports
            .executor
            .claim(RequestId::new(), session, sent.value.id)
            .await
            .unwrap()
            .value;
        assert_eq!(
            counts(&ports.reader.summary(session).await.unwrap()),
            (1, 1, 0, 1)
        );
        assert!(
            ports
                .executor
                .claim(RequestId::new(), session, sent.value.id)
                .await
                .is_err()
        );
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        let finished = ports.reader.summary(session).await.unwrap();
        assert_eq!(counts(&finished), (1, 1, 0, 0));
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        assert_eq!(*ports.reader.summary(session).await.unwrap(), *finished);
        assert!(
            attempt
                .finish(RequestId::new(), ExecutionPhase::Failed)
                .await
                .is_err()
        );
        assert_eq!(*ports.reader.summary(session).await.unwrap(), *finished);
        ports
            .sessions
            .update_metadata(
                RequestId::new(),
                session,
                MetadataChange {
                    title: Some("renamed".into()),
                    favorite: Some(true),
                    pinned: Some(true),
                },
            )
            .await
            .unwrap();
        let renamed = ports.reader.summary(session).await.unwrap();
        assert_eq!(renamed.title, "renamed");
        assert!(renamed.favorite && renamed.pinned);
        assert_eq!(counts(&renamed), (1, 1, 0, 0));
        assert!(renamed.metadata_version > finished.metadata_version);
        assert!(renamed.updated_at_ms > finished.updated_at_ms);
        assert_eq!(renamed.last_message_at_ms, finished.last_message_at_ms);
        assert!(matches!(
            ports
                .reader
                .summary(SessionId::try_from(i64::MAX).unwrap())
                .await,
            Err(KernelError::NotFound(_))
        ));
        let reader = ports.reader.clone();
        ports.shutdown().await.unwrap();
        assert!(matches!(
            reader.summary(session).await,
            Err(KernelError::Closed)
        ));
    }
}

#[tokio::test]
async fn summary_rolls_back_with_failed_commands_and_rejected_commit() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "rollback".into())
            .await
            .unwrap()
            .value
            .id;
        let empty = ports.reader.summary(session).await.unwrap();
        // This fires after the send has updated membership and statistics.
        sql(&admin, "CREATE TRIGGER reject_summary_receipt BEFORE INSERT ON agena_command_results BEGIN SELECT RAISE(ABORT,'injected summary savepoint failure'); END").await;
        let request = RequestId::new();
        assert!(
            ports
                .sessions
                .send(request, session, document("not yet saved"))
                .await
                .is_err()
        );
        assert_eq!(*ports.reader.summary(session).await.unwrap(), *empty);
        sql(&admin, "DROP TRIGGER reject_summary_receipt").await;
        sql(&admin, "CREATE TABLE injected_summary_failure (owner INTEGER REFERENCES agena_sessions(id) DEFERRABLE INITIALLY DEFERRED)").await;
        sql(&admin, "CREATE TRIGGER reject_summary_commit AFTER INSERT ON agena_command_results BEGIN INSERT INTO injected_summary_failure VALUES (-123); END").await;
        assert!(
            ports
                .sessions
                .send(request, session, document("not yet saved"))
                .await
                .is_err()
        );
        assert!(empty.ptr_eq(&ports.reader.summary(session).await.unwrap()));
        sql(&admin, "DROP TRIGGER reject_summary_commit").await;
        sql(&admin, "DROP TABLE injected_summary_failure").await;
        ports
            .sessions
            .send(request, session, document("not yet saved"))
            .await
            .unwrap();
        assert_eq!(
            counts(&ports.reader.summary(session).await.unwrap()),
            (1, 1, 1, 0)
        );
        assert_eq!(
            ports
                .reader
                .history(session, None, 200)
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
async fn summary_counts_all_entry_kinds_and_recovery_only_clears_running_work() {
    let (ports, admin, _root, url) = fixture(false).await;
    let (session, running, round) = content_tests::round(&ports).await;
    let queued = ports
        .sessions
        .send(RequestId::new(), session, document("still queued"))
        .await
        .unwrap()
        .value;
    let text = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    text.append("unfinished output").await.unwrap();
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "summary-tool".into(),
                name: "fixture".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    tool.request_interaction(
        RequestId::new(),
        InteractionRequest::Permission {
            action_key: "fixture".into(),
            description: "await decision".into(),
        },
    )
    .await
    .unwrap();
    let before = ports.reader.summary(session).await.unwrap();
    assert_eq!(counts(&before), (5, 2, 1, 1));
    drop((round, tool, text, admin));
    ports.shutdown().await.unwrap();
    let reopened = open_session_kernel(&url).await.unwrap();
    let after = reopened.reader.summary(session).await.unwrap();
    assert_eq!(counts(&after), (5, 2, 1, 0));
    assert!(after.updated_at_ms > before.updated_at_ms);
    assert!(after.last_message_at_ms >= before.last_message_at_ms);
    assert_eq!(
        reopened
            .reader
            .execution(session, running.id)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Interrupted
    );
    assert_eq!(
        reopened
            .reader
            .execution(session, queued.id)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Queued
    );
    assert!(
        reopened
            .reader
            .pending_interactions(session, None, 200)
            .await
            .unwrap()
            .interactions
            .is_empty()
    );
    reopened.shutdown().await.unwrap();
    let reopened = open_session_kernel(&url).await.unwrap();
    assert_eq!(*reopened.reader.summary(session).await.unwrap(), *after);
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn summary_query_reads_only_compact_session_and_statistics_facts() {
    let (ports, admin, _root, _url) = fixture(true).await;
    let pool = admin.get_sqlite_connection_pool();
    let rows = sqlx::query(&format!("EXPLAIN {}", *reads::SUMMARY_SQL))
        .bind(1i64)
        .fetch_all(pool)
        .await
        .unwrap();
    let session_root: i64 =
        sqlx::query_scalar("SELECT rootpage FROM sqlite_schema WHERE name='agena_sessions'")
            .fetch_one(pool)
            .await
            .unwrap();
    let mut session_cursor = None;
    let mut read_tables = Vec::new();
    for row in &rows {
        if row.get::<String, _>("opcode") == "OpenRead" {
            let root = row.get::<i64, _>("p2");
            read_tables.push(root);
            if root == session_root {
                session_cursor = Some(row.get::<i64, _>("p1"));
            }
        }
    }
    assert_eq!(
        read_tables.len(),
        2,
        "summary must not join history, attempts, or entries"
    );
    let session_cursor = session_cursor.unwrap();
    let configuration_columns = sqlx::query("PRAGMA table_info(agena_sessions)")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .filter(|row| {
            matches!(
                row.get::<String, _>("name").as_str(),
                "selection_json" | "configuration_json"
            )
        })
        .map(|row| row.get::<i64, _>("cid"))
        .collect::<Vec<_>>();
    assert_eq!(configuration_columns.len(), 2);
    for row in &rows {
        if row.get::<String, _>("opcode") == "Column" && row.get::<i64, _>("p1") == session_cursor {
            assert!(
                !configuration_columns.contains(&row.get::<i64, _>("p2")),
                "summary must not read configuration or selection JSON"
            );
        }
    }
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row.get::<String, _>("opcode").as_str(), "Rewind" | "Next"))
    );
    ports.shutdown().await.unwrap();
}
