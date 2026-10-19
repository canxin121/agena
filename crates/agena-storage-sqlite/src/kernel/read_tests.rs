use std::{
    collections::BTreeSet,
    future::{Future, poll_fn},
    task::Poll,
};

use super::{tests::document, *};

#[tokio::test]
async fn saturated_content_read_admission_keeps_cold_control_queries_available() {
    for memory in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let url = if memory {
            "sqlite::memory:".to_owned()
        } else {
            format!(
                "sqlite://{}?mode=rwc",
                root.path().join("kernel.db").display()
            )
        };
        let database = KernelDatabase::open(&url).await.unwrap();
        let reader_pool = database.reader.get_sqlite_connection_pool().clone();
        let admin = database.writer.clone();
        let diagnostics = KernelDiagnostics::default();
        let (ports, maintenance) = compose_database(database, diagnostics.clone())
            .await
            .unwrap();
        tests::sql(
            &admin,
            "INSERT INTO agena_workspaces (id,path,created_at_ms,updated_at_ms) VALUES (1,'read admission fixture',0,0)",
        )
        .await;
        let (session, execution, round) = content_tests::round(&ports).await;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        let text = "eight concurrent observers keep state and cancellation available";
        output.append(text).await.unwrap();
        let mut watches = Vec::new();
        for _ in 0..8 {
            watches.push(
                ports
                    .reader
                    .watch_content(session, output.id())
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(diagnostics.read_budget().subscribers, 8);
        maintenance.invalidate_read_cache();

        // Hold the real reader pool so all eight content operations remain
        // admitted at once, without timing sleeps or a synthetic permit hook.
        let mut connections = Vec::new();
        for _ in 0..if memory { 1 } else { 4 } {
            connections.push(reader_pool.acquire().await.unwrap());
        }
        let mut ranges = (0..8)
            .map(|_| Box::pin(ports.reader.text_range(session, output.id(), None, 1024)))
            .collect::<Vec<_>>();
        poll_fn(|cx| {
            for range in &mut ranges {
                assert!(range.as_mut().poll(cx).is_pending());
            }
            Poll::Ready(())
        })
        .await;
        assert!(matches!(
            ports
                .reader
                .text_range(session, output.id(), None, 1024)
                .await,
            Err(KernelError::Busy)
        ));

        // These are cold reads used by the ordinary state/permission paths.
        // A full content lane must leave them queued for SQL, not reject them.
        let mut control = Box::pin(async {
            tokio::try_join!(
                ports.reader.metadata(session),
                ports.reader.summary(session),
                ports.reader.configuration(session),
                ports.reader.pending_interactions(session, None, 1),
                ports.reader.active_executions(session, None, 1),
                ports.reader.latest_round(session),
                ports.reader.prompt_window_status(session),
                ports.reader.control_version(session),
            )
        });
        poll_fn(|cx| {
            assert!(control.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(connections);
        let (control, pages) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(control, async {
                let mut pages = Vec::new();
                for range in ranges {
                    pages.push(range.await.unwrap());
                }
                pages
            })
        })
        .await
        .expect("admitted content and control reads drain after the SQL pool is released");
        let (metadata, summary, _configuration, pending, active, latest, _prompt, version) =
            control.expect("content saturation must not reject control reads");
        assert_eq!(metadata.title, "text");
        assert_eq!(summary.id, session);
        assert!(pending.interactions.is_empty());
        assert_eq!(active.executions[0].id, execution.id);
        assert!(latest.is_none());
        assert!(*version > 0);
        assert_eq!(pages.len(), 8);
        for page in &pages {
            assert_eq!(page.bytes, text.as_bytes());
        }

        // Retain the watches and returned pages across another read and a real
        // cancellation. Their accounting leases must not retain SQL admission.
        let another = ports
            .reader
            .text_range(session, output.id(), None, 1024)
            .await
            .unwrap();
        assert_eq!(another.bytes, text.as_bytes());
        ports
            .sessions
            .cancel(RequestId::new(), session, execution.id)
            .await
            .unwrap();
        assert!(
            ports
                .reader
                .execution(session, execution.id)
                .await
                .unwrap()
                .phase
                .is_terminal()
        );
        assert_eq!(diagnostics.read_budget().subscribers, 8);
        drop(watches);
        assert_eq!(diagnostics.read_budget().subscribers, 0);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn latest_usage_round_skips_a_failure_without_a_measurement() {
    let (ports, _admin, _root, _url) = tests::fixture(true).await;
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "usage".into())
        .await
        .unwrap()
        .value
        .id;
    let sent = ports
        .sessions
        .send(RequestId::new(), session, document("measure me"))
        .await
        .unwrap()
        .value;
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, sent.id)
        .await
        .unwrap()
        .value;
    let measured_round = attempt
        .begin_round(RequestId::new(), vec![sent.message_id])
        .await
        .unwrap()
        .value;
    measured_round
        .finish(
            RequestId::new(),
            Some(serde_json::json!({
                "usage": {
                    "input_tokens": 100,
                    "cache_read_tokens": 900,
                    "output_tokens": 50
                }
            })),
            true,
        )
        .await
        .unwrap();
    let failed = attempt
        .begin_round(RequestId::new(), vec![sent.message_id])
        .await
        .unwrap()
        .value;
    failed
        .finish(
            RequestId::new(),
            Some(serde_json::json!({"failure": {"user": {"fallback": "stopped"}}})),
            false,
        )
        .await
        .unwrap();

    let latest = ports.reader.latest_round(session).await.unwrap();
    assert_eq!(latest.as_ref().map(|round| round.id), Some(failed.id()));

    let latest_measured = ports.reader.latest_usage_round(session).await.unwrap();
    assert_eq!(
        latest_measured.as_ref().map(|round| round.id),
        Some(measured_round.id())
    );
    ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn active_pages_and_entry_details_are_owned_bounded_cached_and_invalidated() {
    for memory in [true, false] {
        let diagnostics = KernelDiagnostics::default();
        let (ports, _admin, _root, _url) =
            tests::fixture_with_diagnostics(memory, diagnostics.clone()).await;
        let (session, running, _round) = content_tests::round(&ports).await;
        let other = ports
            .sessions
            .create(RequestId::new(), 1, "other".into())
            .await
            .unwrap()
            .value
            .id;
        ports
            .sessions
            .send(RequestId::new(), other, document("foreign queued"))
            .await
            .unwrap();
        let mut expected = BTreeSet::from([running.id]);
        let mut sent = None;
        for index in 0..9 {
            let value = ports
                .sessions
                .send(
                    RequestId::new(),
                    session,
                    document(&format!("queued {index}")),
                )
                .await
                .unwrap()
                .value;
            expected.insert(value.id);
            sent = Some(value);
        }
        let sent = sent.unwrap();
        let entry = ports.reader.entry(session, sent.message_id).await.unwrap();
        assert!(
            matches!(&entry.payload,EntryPayload::User {document} if document.text()=="queued 8")
        );
        let start = diagnostics.snapshot();
        assert!(entry.ptr_eq(&ports.reader.entry(session, sent.message_id).await.unwrap()));
        assert_eq!(diagnostics.snapshot().detail_queries, start.detail_queries);
        assert!(matches!(
            ports.reader.entry(other, sent.message_id).await,
            Err(KernelError::NotFound(_))
        ));
        let first = ports
            .reader
            .active_executions(session, None, 3)
            .await
            .unwrap();
        let start = diagnostics.snapshot();
        assert!(
            first.ptr_eq(
                &ports
                    .reader
                    .active_executions(session, None, 3)
                    .await
                    .unwrap()
            )
        );
        assert_eq!(
            diagnostics.snapshot().activity_queries,
            start.activity_queries
        );
        let mut cursor = None;
        let mut seen = BTreeSet::new();
        loop {
            let start = diagnostics.snapshot();
            let page = ports
                .reader
                .active_executions(session, cursor, 3)
                .await
                .unwrap();
            assert!(diagnostics.snapshot().activity_rows - start.activity_rows <= 4);
            for execution in &page.executions {
                assert!(seen.insert(execution.id));
                assert_eq!(execution.session_id, session);
                assert!(!execution.phase.is_terminal());
            }
            cursor = page.next;
            if !page.has_more {
                break;
            }
        }
        assert_eq!(seen, expected);
        let cursor = ActiveExecutionCursor {
            session: other,
            after: running.id,
        };
        assert!(matches!(
            ports
                .reader
                .active_executions(session, Some(cursor), 3)
                .await,
            Err(KernelError::Ownership)
        ));
        ports
            .sessions
            .cancel(RequestId::new(), session, running.id)
            .await
            .unwrap();
        let page = ports
            .reader
            .active_executions(session, None, 200)
            .await
            .unwrap();
        assert!(
            !page
                .executions
                .iter()
                .any(|execution| execution.id == running.id)
        );
        let retried = ports
            .sessions
            .retry(RequestId::new(), session, running.id)
            .await
            .unwrap();
        let page = ports
            .reader
            .active_executions(session, None, 200)
            .await
            .unwrap();
        let restored = page
            .executions
            .iter()
            .filter(|execution| execution.id == running.id)
            .collect::<Vec<_>>();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].generation, retried.value.generation);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn pending_interactions_select_a_byte_window_and_leave_the_active_index_after_decision() {
    for memory in [true, false] {
        let diagnostics = KernelDiagnostics::default();
        let (ports, admin, _root, _url) =
            tests::fixture_with_diagnostics(memory, diagnostics.clone()).await;
        let (session, execution, round) = content_tests::round(&ports).await;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "activity".into(),
                    name: "fixture".into(),
                    input: serde_json::json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        let text = "界".repeat(60_000);
        let mut expected = BTreeSet::new();
        for index in 0..12 {
            let id = tool
                .request_interaction(
                    RequestId::new(),
                    InteractionRequest::Permission {
                        action_key: format!("fixture/{index}"),
                        description: text.clone(),
                    },
                )
                .await
                .unwrap()
                .value;
            expected.insert(id);
        }
        let page = ports
            .reader
            .pending_interactions(session, None, 200)
            .await
            .unwrap();
        assert_eq!(page.interactions.len(), 5);
        assert!(page.has_more);
        let start = diagnostics.snapshot();
        assert!(
            page.ptr_eq(
                &ports
                    .reader
                    .pending_interactions(session, None, 200)
                    .await
                    .unwrap()
            )
        );
        assert_eq!(
            diagnostics.snapshot().activity_queries,
            start.activity_queries
        );
        let first = page.interactions[0].id;
        ports
            .sessions
            .reply(
                RequestId::new(),
                session,
                first,
                super::tests::permission_decision(first, true, None),
                None,
            )
            .await
            .unwrap();
        expected.remove(&first);
        let mut cursor = None;
        let mut seen = BTreeSet::new();
        loop {
            let start = diagnostics.snapshot();
            let page = ports
                .reader
                .pending_interactions(session, cursor, 200)
                .await
                .unwrap();
            let end = diagnostics.snapshot();
            assert!(end.interaction_body_bytes - start.interaction_body_bytes <= 1024 * 1024);
            assert!(end.activity_rows - start.activity_rows <= 5);
            for interaction in &page.interactions {
                assert!(seen.insert(interaction.id));
                assert!(interaction.decision.is_none());
            }
            cursor = page.next;
            if !page.has_more {
                break;
            }
        }
        assert_eq!(seen, expected);
        assert!(page.interactions[0].decision.is_none());
        let other = ports
            .sessions
            .create(RequestId::new(), 1, "other".into())
            .await
            .unwrap()
            .value
            .id;
        assert!(
            ports
                .reader
                .pending_interactions(other, None, 200)
                .await
                .unwrap()
                .interactions
                .is_empty()
        );
        assert!(matches!(
            ports
                .reader
                .pending_interactions(
                    other,
                    Some(PendingInteractionCursor {
                        session,
                        after: first
                    }),
                    200
                )
                .await,
            Err(KernelError::Ownership)
        ));
        let foreign = ports
            .sessions
            .send(RequestId::new(), other, document("foreign attempt"))
            .await
            .unwrap();
        // Infrastructure corruption fixture: ownership columns cannot point
        // to a valid but unrelated attempt while retaining this tool identity.
        assert!(sqlx::query("INSERT INTO agena_interactions (tool_id,session_id,execution_id,generation,request_kind,request_json,created_at_ms) VALUES (?,?,?,1,'permission','{}',0)")
            .bind(tool.id().get()).bind(other.get()).bind(foreign.value.id.to_string()).execute(admin.get_sqlite_connection_pool()).await.is_err());
        ports
            .sessions
            .cancel(RequestId::new(), session, execution.id)
            .await
            .unwrap();
        assert!(
            ports
                .reader
                .pending_interactions(session, None, 200)
                .await
                .unwrap()
                .interactions
                .is_empty()
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn activity_queries_use_their_partial_indexes_and_pending_preflight_never_reads_json() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = tests::fixture(memory).await;
        let plan = sqlx::query(&format!(
            "EXPLAIN QUERY PLAN {}",
            reads::ACTIVE_EXECUTIONS_SQL
        ))
        .bind(1i64)
        .bind("")
        .bind(201i64)
        .fetch_all(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
        assert!(plan.iter().any(|row| {
            row.try_get::<String, _>("detail")
                .unwrap()
                .contains("idx_agena_attempt_active")
        }));
        let root: i64 = sqlx::query_scalar(
            "SELECT rootpage FROM sqlite_schema WHERE name='agena_interactions'",
        )
        .fetch_one(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
        let code = sqlx::query(&format!(
            "EXPLAIN {}",
            reads::PENDING_INTERACTIONS_PREFLIGHT_SQL
        ))
        .bind(1i64)
        .bind(0i64)
        .bind(201i64)
        .fetch_all(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
        let cursors = code
            .iter()
            .filter(|row| {
                row.try_get::<String, _>("opcode").unwrap() == "OpenRead"
                    && row.try_get::<i64, _>("p2").unwrap() == root
            })
            .map(|row| row.try_get::<i64, _>("p1").unwrap())
            .collect::<BTreeSet<_>>();
        assert!(!code.iter().any(
            |row| row.try_get::<String, _>("opcode").unwrap() == "Column"
                && cursors.contains(&row.try_get::<i64, _>("p1").unwrap())
        ));
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn valid_warm_views_use_no_sql_and_committed_changes_invalidate_before_receipts() {
    for memory in [true, false] {
        let diagnostics = KernelDiagnostics::default();
        let (ports, admin, _root, _url) =
            tests::fixture_with_diagnostics(memory, diagnostics.clone()).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "before".into())
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
        let other_view = ports.reader.metadata(other).await.unwrap();
        let mut sent = None;
        for index in 0..8 {
            sent = Some(
                ports
                    .sessions
                    .send(
                        RequestId::new(),
                        session,
                        document(&format!("entry {index}")),
                    )
                    .await
                    .unwrap()
                    .value,
            );
        }
        let sent = sent.unwrap();
        let metadata = ports.reader.metadata(session).await.unwrap();
        let history = ports.reader.history(session, None, 3).await.unwrap();
        let execution = ports.reader.execution(session, sent.id).await.unwrap();
        let before = diagnostics.snapshot();
        for _ in 0..32 {
            assert!(metadata.ptr_eq(&ports.reader.metadata(session).await.unwrap()));
            assert!(history.ptr_eq(&ports.reader.history(session, None, 3).await.unwrap()));
            assert!(execution.ptr_eq(&ports.reader.execution(session, sent.id).await.unwrap()));
        }
        let after = diagnostics.snapshot();
        assert_eq!(before.metadata_queries, after.metadata_queries);
        assert_eq!(before.history_queries, after.history_queries);
        assert_eq!(before.detail_queries, after.detail_queries);
        assert!(diagnostics.read_cache().hits >= 96);
        assert!(diagnostics.read_cache().bytes <= 16 * 1024 * 1024);
        ports
            .sessions
            .update_metadata(
                RequestId::new(),
                session,
                MetadataChange {
                    title: Some("after".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(metadata.title, "before");
        assert_eq!(ports.reader.metadata(session).await.unwrap().title, "after");
        assert_eq!(
            ports
                .reader
                .history(session, None, 3)
                .await
                .unwrap()
                .metadata
                .title,
            "after"
        );
        assert!(other_view.ptr_eq(&ports.reader.metadata(other).await.unwrap()));
        let fresh = ports
            .sessions
            .send(RequestId::new(), session, document("new membership"))
            .await
            .unwrap();
        let page = ports.reader.history(session, None, 3).await.unwrap();
        assert_eq!(page.entries.last().unwrap().id, fresh.value.message_id);
        // Append-only membership growth preserves every existing position, so
        // a cursor handed to the client before the new message stays valid.
        assert!(
            ports.reader.history(session, history.next, 3).await.is_ok(),
            "an append must not invalidate an older history cursor"
        );
        let mut moved_anchor = history.next.unwrap();
        moved_anchor.anchor =
            Some(EntryId::try_from(page.entries.first().unwrap().id.get()).unwrap());
        assert!(matches!(
            ports.reader.history(session, Some(moved_anchor), 3).await,
            Err(KernelError::Conflict(_))
        ));
        let mut future_generation = history.next.unwrap();
        future_generation.membership = page.metadata.membership_generation.next().unwrap();
        assert!(matches!(
            ports
                .reader
                .history(session, Some(future_generation), 3)
                .await,
            Err(KernelError::Conflict(_))
        ));
        ports
            .sessions
            .cancel(RequestId::new(), session, sent.id)
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .execution(session, sent.id)
                .await
                .unwrap()
                .phase,
            ExecutionPhase::Cancelled
        );

        // A definite deferred-FK COMMIT rejection must preserve warm truth.
        let warm = ports.reader.metadata(session).await.unwrap();
        tests::sql(&admin,"CREATE TABLE injected_cache_failure (owner INTEGER REFERENCES agena_sessions(id) DEFERRABLE INITIALLY DEFERRED)").await;
        tests::sql(&admin,"CREATE TRIGGER reject_cache_commit AFTER UPDATE OF title ON agena_sessions BEGIN INSERT INTO injected_cache_failure VALUES (-123); END").await;
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
        assert!(warm.ptr_eq(&ports.reader.metadata(session).await.unwrap()));
        assert_eq!(warm.title, "after");
        tests::sql(&admin, "DROP TRIGGER reject_cache_commit").await;
        tests::sql(&admin, "DROP TABLE injected_cache_failure").await;
        drop((metadata, history, execution, other_view, page, warm));
        let reader = ports.reader.clone();
        ports.shutdown().await.unwrap();
        assert!(matches!(
            reader.metadata(session).await,
            Err(KernelError::Closed)
        ));
        assert!(matches!(
            reader.watch(session, None),
            Err(KernelError::Closed)
        ));
        assert_eq!(diagnostics.read_budget().bytes, 0);
    }
}

#[tokio::test]
async fn consumer_held_ranges_and_watches_retain_budget_and_recover_when_dropped() {
    for memory in [true, false] {
        let diagnostics = KernelDiagnostics::default();
        let (ports, _admin, _root, _url) =
            tests::fixture_with_diagnostics(memory, diagnostics.clone()).await;
        let (session, _sent, round) = content_tests::round(&ports).await;
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        output.append(&"x".repeat(256 * 1024)).await.unwrap();
        output.finish(RequestId::new()).await.unwrap();
        diagnostics.1.clear();
        let baseline = diagnostics.read_budget().bytes;
        let view = ports
            .reader
            .text_range(session, output.id(), None, 256 * 1024)
            .await
            .unwrap();
        let retained = diagnostics.read_budget().bytes;
        let clone = view.clone();
        assert!(retained >= baseline + 256 * 1024);
        drop(view);
        assert_eq!(diagnostics.read_budget().bytes, retained);
        drop(clone);
        assert_eq!(diagnostics.read_budget().bytes, baseline);
        let mut views = Vec::new();
        loop {
            match ports
                .reader
                .text_range(session, output.id(), None, 256 * 1024)
                .await
            {
                Ok(view) => views.push(view),
                Err(KernelError::Busy) => break,
                Err(error) => panic!("unexpected read error {error}"),
            }
        }
        assert!(views.len() > 100 && views.len() < 400);
        assert!(diagnostics.read_budget().bytes <= 256 * 1024 * 1024);
        drop(views);
        assert!(
            ports
                .reader
                .text_range(session, output.id(), None, 1)
                .await
                .is_ok()
        );
        let mut watches = Vec::new();
        for _ in 0..512 {
            watches.push(ports.reader.watch(session, None).unwrap());
        }
        assert!(matches!(
            ports.reader.watch(session, None),
            Err(KernelError::Busy)
        ));
        assert!(matches!(
            ports.reader.watch_content(session, output.id()).await,
            Err(KernelError::Busy)
        ));
        assert_eq!(diagnostics.read_budget().subscribers, 512);
        drop(watches.pop());
        let watch = ports
            .reader
            .watch_content(session, output.id())
            .await
            .unwrap();
        drop((watches, watch));
        assert_eq!(diagnostics.read_budget().subscribers, 0);
        ports.shutdown().await.unwrap();
        assert_eq!(diagnostics.read_budget().bytes, 0);
    }
}

#[tokio::test]
async fn history_selects_its_byte_window_before_materializing_large_entry_bodies() {
    for memory in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let url = if memory {
            "sqlite::memory:".into()
        } else {
            format!(
                "sqlite://{}?mode=rwc",
                root.path().join("read-window.db").display()
            )
        };
        let kernel = open_kernel_runtime(&url).await.unwrap();
        let workspace = kernel
            .workspaces
            .create(root.path().to_string_lossy().into_owned())
            .await
            .unwrap();
        let ports = &kernel.ports;
        let session = ports
            .sessions
            .create(RequestId::new(), workspace.id, "bounded bodies".into())
            .await
            .unwrap()
            .value
            .id;
        let body = "界".repeat(60000);
        let mut expected = BTreeSet::new();
        for index in 0..20 {
            expected.insert(
                ports
                    .sessions
                    .send(
                        RequestId::new(),
                        session,
                        document(&format!("{index}: {body}")),
                    )
                    .await
                    .unwrap()
                    .value
                    .message_id,
            );
        }
        let mut before = None;
        let mut seen = BTreeSet::new();
        loop {
            let start = kernel.diagnostics.snapshot();
            let page = ports.reader.history(session, before, 200).await.unwrap();
            let end = kernel.diagnostics.snapshot();
            assert!(page.entries.len() <= 5);
            // These count actual returned SQL payloads, not a post-fetch slice.
            assert!(end.history_entry_body_bytes - start.history_entry_body_bytes <= 1024 * 1024);
            assert!(end.history_entry_rows - start.history_entry_rows <= page.entries.len() as u64);
            assert!(end.history_candidate_rows - start.history_candidate_rows <= 201);
            for entry in &page.entries {
                assert!(seen.insert(entry.id));
            }
            before = page.next;
            if !page.has_more {
                break;
            }
            assert!(before.is_some());
        }
        assert_eq!(seen, expected);
        kernel.ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn history_window_preflight_uses_a_covering_index_instead_of_loading_cold_json() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = super::tests::fixture(memory).await;
        let rows = sqlx::query(&format!(
            "EXPLAIN QUERY PLAN {}",
            reads::HISTORY_PREFLIGHT_SQL
        ))
        .bind(1i64)
        .bind(0i64)
        .bind(128i64)
        .bind(201i64)
        .fetch_all(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
        let details = rows
            .iter()
            .map(|row| row.try_get::<String, _>("detail").unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(details.contains("INDEX idx_agena_entry_bytes"), "{details}");
        assert!(!details.contains("SCAN agena_entries"), "{details}");
        // SQLite does not always label an index over a generated stored column
        // as COVERING. Verify the VM: no Column reads from the entries table,
        // including its cold JSON payload. DeferredSeek alone performs no read.
        let root: i64 =
            sqlx::query_scalar("SELECT rootpage FROM sqlite_schema WHERE name='agena_entries'")
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap();
        let bytecode = sqlx::query(&format!("EXPLAIN {}", reads::HISTORY_PREFLIGHT_SQL))
            .bind(1i64)
            .bind(0i64)
            .bind(128i64)
            .bind(201i64)
            .fetch_all(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        let table_cursors = bytecode
            .iter()
            .filter(|row| {
                row.try_get::<String, _>("opcode").unwrap() == "OpenRead"
                    && row.try_get::<i64, _>("p2").unwrap() == root
            })
            .map(|row| row.try_get::<i64, _>("p1").unwrap())
            .collect::<BTreeSet<_>>();
        assert!(
            !bytecode.iter().any(
                |row| row.try_get::<String, _>("opcode").unwrap() == "Column"
                    && table_cursors.contains(&row.try_get::<i64, _>("p1").unwrap())
            ),
            "preflight reads entries table columns: {details}"
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn deferred_foreign_key_checks_use_reverse_indexes_instead_of_scanning_cold_history() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = tests::fixture(memory).await;
        let roots=sqlx::query("SELECT name,rootpage FROM sqlite_schema WHERE type IN ('table','index') AND rootpage>0")
            .fetch_all(admin.get_sqlite_connection_pool()).await.unwrap().into_iter()
            .map(|row|(row.try_get::<i64,_>("rootpage").unwrap(),row.try_get::<String,_>("name").unwrap()))
            .collect::<std::collections::BTreeMap<_,_>>();
        for (sql, required) in [
            (
                "INSERT INTO agena_entries (origin_session_id,execution_id,generation,kind,body_json,created_at_ms) VALUES (1,'20261018-0000-4000-8000-ffffffeeeeee',1,'user','[]',0)",
                vec!["idx_agena_execution_message"],
            ),
            (
                "INSERT INTO agena_attempts (execution_id,generation,session_id,phase,finished_at_ms) VALUES ('20261018-0000-4000-8000-ffffffeeeeee',1,1,'succeeded',0)",
                vec!["idx_agena_entry_attempt"],
            ),
            (
                "INSERT INTO agena_content_streams (id,entry_id,session_id,execution_id,generation,kind,epoch,capture_state) VALUES ('20261018-0000-4000-8000-ffffffeeeeed',1,1,'20261018-0000-4000-8000-ffffffeeeeee',1,'text','20261018-0000-4000-8000-ffffffeeeeec','open')",
                vec!["idx_agena_entry_content"],
            ),
            (
                "DELETE FROM agena_entries WHERE id=1",
                vec![
                    "idx_agena_entry_message",
                    "idx_agena_round_input_entry",
                    "idx_agena_delivery_result",
                ],
            ),
            (
                "DELETE FROM agena_content_blocks WHERE content_id='20261018-0000-4000-8000-ffffffeeeeed' AND logical_start=0",
                vec!["idx_agena_checkpoint_block"],
            ),
        ] {
            let bytecode = sqlx::query(&format!("EXPLAIN {sql}"))
                .fetch_all(admin.get_sqlite_connection_pool())
                .await
                .unwrap();
            let mut cursors = std::collections::BTreeMap::new();
            let mut opened = BTreeSet::new();
            for row in bytecode {
                let opcode = row.try_get::<String, _>("opcode").unwrap();
                // Trigger subprograms restart instruction and cursor numbering.
                // A cursor from the parent program cannot name their table.
                if row.try_get::<i64, _>("addr").unwrap() == 0 {
                    cursors.clear();
                }
                let cursor = row.try_get::<i64, _>("p1").unwrap();
                if matches!(opcode.as_str(), "OpenRead" | "OpenWrite") {
                    let root = row.try_get::<i64, _>("p2").unwrap();
                    if let Some(name) = roots.get(&root) {
                        cursors.insert(cursor, name.clone());
                        if opcode == "OpenRead" {
                            opened.insert(name.clone());
                        }
                    }
                }
                if opcode == "Rewind"
                    && let Some(name) = cursors.get(&cursor)
                {
                    assert!(
                        !name.starts_with("agena_"),
                        "FK check scans growing object {name}: {sql}"
                    );
                }
            }
            for index in required {
                assert!(
                    opened.contains(index),
                    "FK check did not select {index}: {opened:?}"
                );
            }
        }
        ports.shutdown().await.unwrap();
    }
}
