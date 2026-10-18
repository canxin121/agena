use super::{
    tests::{document, sql},
    *,
};

pub(super) async fn fixture(
    memory: bool,
) -> (
    KernelPorts,
    port::KernelMaintenance,
    Arc<sea_orm::DatabaseConnection>,
    KernelDiagnostics,
    tempfile::TempDir,
) {
    let root = tempfile::tempdir().unwrap();
    let url = if memory {
        "sqlite::memory:".into()
    } else {
        format!(
            "sqlite://{}?mode=rwc",
            root.path().join("snapshot.db").display()
        )
    };
    let database = KernelDatabase::open(&url).await.unwrap();
    let admin = database.writer.clone();
    let diagnostics = KernelDiagnostics::default();
    let (ports, maintenance) = compose_database(database, diagnostics.clone())
        .await
        .unwrap();
    sql(&admin,"INSERT INTO agena_workspaces (id,path,created_at_ms,updated_at_ms) VALUES (1,'snapshot-fixture',0,0)").await;
    (ports, maintenance, admin, diagnostics, root)
}

async fn count(admin: &sea_orm::DatabaseConnection, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(admin.get_sqlite_connection_pool())
        .await
        .unwrap()
}

async fn create(ports: &KernelPorts, title: &str) -> SessionId {
    ports
        .sessions
        .create(RequestId::new(), 1, title.into())
        .await
        .unwrap()
        .value
        .id
}

#[tokio::test]
async fn pinned_history_keeps_old_tail_membership_during_appends_without_a_full_manifest_copy() {
    for memory in [true, false] {
        let (ports, maintenance, admin, diagnostics, _root) = fixture(memory).await;
        let session = create(&ports, "snapshot title").await;
        let mut expected = Vec::new();
        for index in 0..150 {
            expected.push(
                ports
                    .sessions
                    .send(
                        RequestId::new(),
                        session,
                        document(&format!("original {index}")),
                    )
                    .await
                    .unwrap()
                    .value
                    .message_id,
            );
        }
        let first = ports.reader.history_snapshot(session).await.unwrap();
        let clone = first.clone();
        assert_eq!(first.info().entry_count, 150);
        assert_eq!(count(&admin, "agena_retired_session_segments").await, 0);
        assert_eq!(count(&admin, "agena_history_segments").await, 2);
        for _ in 0..5 {
            ports
                .sessions
                .send(RequestId::new(), session, document("between snapshots"))
                .await
                .unwrap();
        }
        let second = ports.reader.history_snapshot(session).await.unwrap();
        assert_eq!(second.info().entry_count, 155);
        assert!(matches!(
            ports.reader.history_snapshot(session).await,
            Err(KernelError::Busy)
        ));
        ports
            .sessions
            .update_metadata(
                RequestId::new(),
                session,
                MetadataChange {
                    title: Some("new title".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        for _ in 0..140 {
            ports
                .sessions
                .send(RequestId::new(), session, document("later append"))
                .await
                .unwrap();
        }
        // Each root pins just the one replaced tail it needs. Further copies
        // made for later appends are reclaimed by the normal append path.
        assert_eq!(count(&admin, "agena_retired_session_segments").await, 2);
        assert_eq!(count(&admin, "agena_history_segments").await, 5);
        assert_eq!(
            maintenance.collect_history_snapshots().await.unwrap(),
            port::HistorySnapshotGc::default()
        );
        let mut after = None;
        let mut actual = Vec::new();
        loop {
            let before = diagnostics.snapshot();
            let page = first.page(after, 17).await.unwrap();
            let metrics = diagnostics.snapshot();
            assert!(page.entries.len() <= 17);
            assert!(metrics.snapshot_candidate_rows - before.snapshot_candidate_rows <= 18);
            assert!(metrics.snapshot_body_bytes - before.snapshot_body_bytes <= 1024 * 1024);
            for member in &page.entries {
                assert!(actual.last().is_none_or(|last| *last < member.entry.id));
                actual.push(member.entry.id);
            }
            ports
                .sessions
                .send(
                    RequestId::new(),
                    session,
                    document("append while consuming"),
                )
                .await
                .unwrap();
            after = page.next;
            if !page.has_more {
                break;
            }
            assert!(after.is_some());
        }
        assert_eq!(actual, expected);
        assert_eq!(first.info().metadata.title, "snapshot title");
        let foreign_cursor = second.page(None, 1).await.unwrap().entries[0].position;
        assert!(matches!(
            first.page(Some(foreign_cursor), 1).await,
            Err(KernelError::Ownership)
        ));
        let mut after = None;
        let mut seen = 0;
        loop {
            let page = second.page(after, 200).await.unwrap();
            seen += page.entries.len();
            after = page.next;
            if !page.has_more {
                break;
            }
        }
        assert_eq!(seen, 155);
        drop(first);
        assert_eq!(
            maintenance.collect_history_snapshots().await.unwrap().roots,
            0
        );
        assert_eq!(count(&admin, "agena_history_snapshots").await, 2);
        drop((clone, second));
        let gc = maintenance.collect_history_snapshots().await.unwrap();
        assert_eq!(gc.roots, 2);
        assert_eq!(gc.references, 2);
        assert_eq!(gc.segments, 2);
        assert_eq!(count(&admin, "agena_retired_session_segments").await, 0);
        assert_eq!(count(&admin, "agena_history_segments").await, 3);
        ports.shutdown().await.unwrap();
        assert_eq!(diagnostics.read_budget().bytes, 0);
    }
}

#[tokio::test]
async fn snapshot_roots_are_globally_bounded_and_empty_roots_stay_empty_after_send() {
    for memory in [true, false] {
        let (ports, maintenance, admin, diagnostics, _root) = fixture(memory).await;
        let mut roots = Vec::new();
        let mut session = None;
        for index in 0..4 {
            let id = create(&ports, &format!("owner {index}")).await;
            roots.push(ports.reader.history_snapshot(id).await.unwrap());
            roots.push(ports.reader.history_snapshot(id).await.unwrap());
            session = Some(id);
        }
        assert_eq!(count(&admin, "agena_history_snapshots").await, 8);
        let extra = create(&ports, "extra owner").await;
        assert!(matches!(
            ports.reader.history_snapshot(extra).await,
            Err(KernelError::Busy)
        ));
        let empty = roots.last().unwrap().clone();
        ports
            .sessions
            .send(
                RequestId::new(),
                session.unwrap(),
                document("after empty cut"),
            )
            .await
            .unwrap();
        assert!(empty.page(None, 200).await.unwrap().entries.is_empty());
        assert_eq!(empty.info().entry_count, 0);
        drop(empty);
        roots.pop();
        let extra_root = ports.reader.history_snapshot(extra).await.unwrap();
        assert_eq!(count(&admin, "agena_history_snapshots").await, 8);
        drop((roots, extra_root));
        maintenance.collect_history_snapshots().await.unwrap();
        assert_eq!(count(&admin, "agena_history_snapshots").await, 0);
        ports.shutdown().await.unwrap();
        assert_eq!(diagnostics.read_budget().bytes, 0);
    }
}

#[tokio::test]
async fn snapshot_open_append_archival_and_reclamation_failures_roll_back_without_losing_members() {
    for memory in [true, false] {
        let (ports, maintenance, admin, _diagnostics, _root) = fixture(memory).await;
        let session = create(&ports, "rollback").await;
        ports
            .sessions
            .send(RequestId::new(), session, document("first"))
            .await
            .unwrap();
        sql(&admin,"CREATE TRIGGER reject_snapshot_open BEFORE INSERT ON agena_history_snapshots BEGIN SELECT RAISE(ABORT,'snapshot pin failure'); END").await;
        assert!(ports.reader.history_snapshot(session).await.is_err());
        assert_eq!(count(&admin, "agena_history_snapshots").await, 0);
        sql(&admin, "DROP TRIGGER reject_snapshot_open").await;
        let snapshot = ports.reader.history_snapshot(session).await.unwrap();
        let warm = ports.reader.history(session, None, 20).await.unwrap();
        sql(&admin,"CREATE TABLE snapshot_commit_failure (owner INTEGER REFERENCES agena_sessions(id) DEFERRABLE INITIALLY DEFERRED)").await;
        sql(&admin,"CREATE TRIGGER reject_snapshot_append AFTER INSERT ON agena_command_results BEGIN INSERT INTO snapshot_commit_failure VALUES (-123); END").await;
        let request = RequestId::new();
        assert!(
            ports
                .sessions
                .send(request, session, document("later"))
                .await
                .is_err()
        );
        assert!(warm.ptr_eq(&ports.reader.history(session, None, 20).await.unwrap()));
        assert_eq!(count(&admin, "agena_retired_session_segments").await, 0);
        assert_eq!(snapshot.page(None, 20).await.unwrap().entries.len(), 1);
        sql(&admin, "DROP TRIGGER reject_snapshot_append").await;
        ports
            .sessions
            .send(request, session, document("later"))
            .await
            .unwrap();
        assert_eq!(count(&admin, "agena_retired_session_segments").await, 1);
        assert_eq!(snapshot.page(None, 20).await.unwrap().entries.len(), 1);
        sql(&admin,"CREATE TRIGGER reject_snapshot_gc BEFORE DELETE ON agena_retired_session_segments BEGIN SELECT RAISE(ABORT,'snapshot gc failure'); END").await;
        drop(snapshot);
        assert!(maintenance.collect_history_snapshots().await.is_err());
        assert_eq!(count(&admin, "agena_history_snapshots").await, 1);
        assert_eq!(count(&admin, "agena_retired_session_segments").await, 1);
        sql(&admin, "DROP TRIGGER reject_snapshot_gc").await;
        maintenance.collect_history_snapshots().await.unwrap();
        assert_eq!(count(&admin, "agena_retired_session_segments").await, 0);
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
async fn snapshot_pages_preflight_large_unicode_bodies_before_fetching_them() {
    for memory in [true, false] {
        let (ports, maintenance, _admin, diagnostics, _root) = fixture(memory).await;
        let session = create(&ports, "bounded snapshot bodies").await;
        let body = "\u{754c}".repeat(60_000);
        let mut expected = Vec::new();
        for index in 0..20 {
            expected.push(
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
        let snapshot = ports.reader.history_snapshot(session).await.unwrap();
        // Force this root to read its old tail through the retired-edge arm.
        ports
            .sessions
            .send(RequestId::new(), session, document("outside the root"))
            .await
            .unwrap();
        let mut after = None;
        let mut actual = Vec::new();
        let mut pages = 0;
        loop {
            let before = diagnostics.snapshot();
            let page = snapshot.page(after, 200).await.unwrap();
            let end = diagnostics.snapshot();
            assert!(!page.entries.is_empty());
            assert!(page.entries.len() <= 5);
            assert!(end.snapshot_body_bytes - before.snapshot_body_bytes <= 1024 * 1024);
            assert!(end.snapshot_candidate_rows - before.snapshot_candidate_rows <= 201);
            for entry in &page.entries {
                let EntryPayload::User { document } = &entry.entry.payload else {
                    panic!("expected user document")
                };
                assert!(serde_json::to_string(document).unwrap().contains(&body));
                actual.push(entry.entry.id);
            }
            pages += 1;
            after = page.next;
            if !page.has_more {
                break;
            }
            assert!(after.is_some());
        }
        assert_eq!(actual, expected);
        assert_eq!(pages, 4);
        drop(snapshot);
        maintenance.collect_history_snapshots().await.unwrap();
        ports.shutdown().await.unwrap();
        assert_eq!(diagnostics.read_budget().bytes, 0);
    }
}

#[tokio::test]
async fn snapshot_windows_use_membership_indexes_without_reading_entry_json_in_preflight() {
    for memory in [true, false] {
        let (ports, _maintenance, admin, _diagnostics, _root) = fixture(memory).await;
        let pool = admin.get_sqlite_connection_pool();
        let segments = sqlx::query(&format!("EXPLAIN QUERY PLAN {}", snapshots::SEGMENTS_SQL))
            .bind(1i64)
            .bind(0i64)
            .bind(1000i64)
            .bind(1i64)
            .bind(202i64)
            .bind(1i64)
            .bind(0i64)
            .bind(1000i64)
            .bind(1i64)
            .bind(1i64)
            .bind(202i64)
            .bind(202i64)
            .fetch_all(pool)
            .await
            .unwrap();
        let details = segments
            .iter()
            .map(|row| row.get::<String, _>("detail"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            details.contains("SEARCH agena_session_segments"),
            "{details}"
        );
        assert!(
            details.contains("SEARCH agena_retired_session_segments"),
            "{details}"
        );
        assert!(
            !details.contains("SCAN agena_session_segments"),
            "{details}"
        );
        assert!(
            !details.contains("SCAN agena_retired_session_segments"),
            "{details}"
        );
        let members = sqlx::query(&format!("EXPLAIN QUERY PLAN {}", snapshots::MEMBERS_SQL))
            .bind(1i64)
            .bind(0i64)
            .bind(128i64)
            .bind(201i64)
            .fetch_all(pool)
            .await
            .unwrap();
        let details = members
            .iter()
            .map(|row| row.get::<String, _>("detail"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(details.contains("idx_agena_entry_bytes"), "{details}");
        assert!(details.contains("SEARCH m"), "{details}");
        assert!(!details.contains("SCAN agena_entries"), "{details}");
        let root: i64 =
            sqlx::query_scalar("SELECT rootpage FROM sqlite_schema WHERE name='agena_entries'")
                .fetch_one(pool)
                .await
                .unwrap();
        let bytecode = sqlx::query(&format!("EXPLAIN {}", snapshots::MEMBERS_SQL))
            .bind(1i64)
            .bind(0i64)
            .bind(128i64)
            .bind(201i64)
            .fetch_all(pool)
            .await
            .unwrap();
        let cursors = bytecode
            .iter()
            .filter(|row| {
                row.get::<String, _>("opcode") == "OpenRead" && row.get::<i64, _>("p2") == root
            })
            .map(|row| row.get::<i64, _>("p1"))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            !bytecode
                .iter()
                .any(|row| row.get::<String, _>("opcode") == "Column"
                    && cursors.contains(&row.get::<i64, _>("p1"))),
            "snapshot preflight reads entry table columns: {details}"
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn holding_a_snapshot_and_returned_page_does_not_pin_the_wal_read_transaction() {
    let (ports, maintenance, admin, _diagnostics, _root) = fixture(false).await;
    let session = create(&ports, "wal lifetime").await;
    ports
        .sessions
        .send(RequestId::new(), session, document("before"))
        .await
        .unwrap();
    let snapshot = ports.reader.history_snapshot(session).await.unwrap();
    let page = snapshot.page(None, 200).await.unwrap();
    for _ in 0..20 {
        ports
            .sessions
            .send(RequestId::new(), session, document(&"w".repeat(10_000)))
            .await
            .unwrap();
    }
    let checkpoint = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
    assert_eq!(
        checkpoint.get::<i64, _>(0),
        0,
        "a consumer-held snapshot/page pinned the WAL"
    );
    assert_eq!(checkpoint.get::<i64, _>(1), 0);
    assert_eq!(page.entries.len(), 1);
    assert_eq!(snapshot.page(None, 200).await.unwrap().entries.len(), 1);
    drop((page, snapshot));
    maintenance.collect_history_snapshots().await.unwrap();
    ports.shutdown().await.unwrap();
}
