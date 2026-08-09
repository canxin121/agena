//! Tests for the SQLite persistence engine: the same invariants the in-memory
//! engine enforces (lease write-ownership, steal-abort atomicity, shared-part
//! read/append-only, retry, idempotency, GC refcount guard, JSONL round-trip,
//! usage grouping) exercised against the real database, plus cross-process
//! concurrency tests (gate 5): two connection pools over one file model the
//! multi-process deployment.

use std::sync::Arc;

use agena_domain::SessionRelationKind;
use agena_storage::{
    WorkspaceRepository,
    store::{
        LeaseAcquire, NewPart, NewSession, PartRole, PartState, PartVisibility, PersistenceEngine,
        RunOutcome, SessionFacade, SessionStore, SessionView,
    },
};
use serde_json::json;

use crate::{SeaWorkspaceRepository, SqliteEngine, initialize_schema};

/// A workspace-scoped engine with a ready session that already holds a fresh
/// lease under `owner-a` at `now_ms = 1_000_000`.
async fn setup(db: Arc<sea_orm::DatabaseConnection>) -> (SqliteEngine, i64) {
    initialize_schema(&db).await.expect("schema");
    let workspace_id = SeaWorkspaceRepository::new(db.clone())
        .ensure_id("/test/workspace")
        .await
        .expect("workspace");
    let engine = SqliteEngine::new(db);
    let meta = engine
        .create_session(NewSession {
            workspace_id,
            parent_id: None,
            relation_kind: SessionRelationKind::Root,
            cutoff_part_id: None,
            title: "test".to_owned(),
            task_id: None,
            config_json: None,
            provider_anchors_json: None,
        })
        .await
        .expect("create session");
    let session_id = meta.id;
    let acquire = engine
        .try_acquire_lease(session_id, "owner-a", 1_000_000)
        .await
        .expect("acquire lease");
    assert!(
        matches!(acquire, LeaseAcquire::Acquired { reconciled_runs } if reconciled_runs.is_empty())
    );
    (engine, session_id)
}

async fn in_memory_db() -> Arc<sea_orm::DatabaseConnection> {
    Arc::new(
        sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("open in-memory database"),
    )
}

fn text_part(text: &str) -> NewPart {
    NewPart::pending("text", PartRole::User, json!({ "text": text }))
}

async fn submit_hello(engine: &SqliteEngine, session_id: i64) -> (i64, SessionView) {
    let outcome = engine
        .submit_user_message(
            session_id,
            "owner-a",
            vec![text_part("hello")],
            None,
            1_000_000,
        )
        .await
        .expect("submit");
    let view = engine.load_session(session_id).await.expect("load");
    (outcome.run_id, view)
}

#[tokio::test]
async fn user_send_creates_marker_and_parts_with_membership() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let (run_id, view) = submit_hello(&engine, session_id).await;

    assert_eq!(view.parts.len(), 2);
    let marker = &view.parts[0];
    assert!(marker.is_run_marker());
    assert_eq!(marker.content["run_kind"], "user_send");
    assert_eq!(
        run_id, marker.part_id,
        "marker is the first allocated part (id 1)"
    );
    let text = &view.parts[1];
    assert_eq!(text.kind, "text");
    assert_eq!(text.run_id, Some(marker.part_id));
    assert_eq!(text.origin_session_id, session_id);
}

#[tokio::test]
async fn writes_without_a_fresh_lease_are_refused() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;

    // A different owner is refused outright.
    let held = engine
        .submit_user_message(
            session_id,
            "owner-b",
            vec![text_part("nope")],
            None,
            1_000_000,
        )
        .await
        .expect_err("other owner cannot write");
    assert!(matches!(
        held,
        agena_storage::store::StoreError::LeaseHeldByOther { .. }
    ));

    // Releasing the lease makes the original owner a non-holder too.
    assert!(
        engine
            .release_lease(session_id, "owner-a")
            .await
            .expect("release")
    );
    let missing = engine
        .submit_user_message(
            session_id,
            "owner-a",
            vec![text_part("nope")],
            None,
            1_000_000,
        )
        .await
        .expect_err("no lease");
    assert!(matches!(
        missing,
        agena_storage::store::StoreError::LeaseNotHeld { .. }
    ));
}

#[tokio::test]
async fn lease_steal_aborts_stale_run_atomically() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let (run_id, _view) = submit_hello(&engine, session_id).await;

    // The original owner goes quiet past the staleness threshold; a new owner
    // steals the lease and the residual run is aborted in the same transaction.
    let acquire = engine
        .try_acquire_lease(session_id, "owner-b", 1_000_000 + 60_000)
        .await
        .expect("steal stale lease");
    assert!(matches!(
        acquire,
        LeaseAcquire::Acquired { reconciled_runs } if reconciled_runs == vec![run_id]
    ));

    let view = engine.load_session(session_id).await.expect("load");
    let marker = view
        .parts
        .iter()
        .find(|part| part.is_run_marker())
        .expect("marker");
    assert_eq!(marker.state, PartState::Failed);
    assert_eq!(marker.content["abort_reason"], "lease_stolen");
    // The child of the aborted run is cancelled.
    let text = view
        .parts
        .iter()
        .find(|part| part.kind == "text")
        .expect("text");
    assert_eq!(text.state, PartState::Cancelled);
}

#[tokio::test]
async fn fork_copies_edges_up_to_cutoff_and_child_reads_shared_prefix() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let (_run_id, view) = submit_hello(&engine, session_id).await;
    let marker_id = view.parts[0].part_id;
    let text_id = view.parts[1].part_id;

    // Fork at the text part: child inherits the marker + text.
    let child = engine
        .fork_session(session_id, text_id, "fork".to_owned(), false, 1_000_000)
        .await
        .expect("fork");
    let child_view = engine.load_session(child.id).await.expect("load child");
    let child_ids: Vec<i64> = child_view.parts.iter().map(|part| part.part_id).collect();
    assert_eq!(child_ids, vec![marker_id, text_id]);

    // Fork at the marker only: child stops before the text part.
    let early = engine
        .fork_session(session_id, marker_id, "early".to_owned(), false, 1_000_000)
        .await
        .expect("fork early");
    let early_view = engine.load_session(early.id).await.expect("load early");
    assert_eq!(early_view.parts.len(), 1);
    assert_eq!(early_view.parts[0].part_id, marker_id);

    // Shared parts are read/append-only: the child cannot update a part its
    // parent created (8.4). The child holds a fresh lease before it tries.
    engine
        .try_acquire_lease(child.id, "owner-a", 1_000_000)
        .await
        .expect("child acquires lease");
    let error = engine
        .update_part(
            child.id,
            "owner-a",
            text_id,
            agena_storage::store::PartDelta {
                state: Some(PartState::Completed),
                ..Default::default()
            },
            1_000_000,
        )
        .await
        .expect_err("child cannot update shared part");
    assert!(matches!(
        error,
        agena_storage::store::StoreError::InvalidState(_)
    ));
}

#[tokio::test]
async fn retry_transitions_failed_to_in_progress_with_revision_bump_but_not_for_runs() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    // A tool_call created `in_progress` (not `pending`), so
    // in_progress -> failed -> in_progress is the valid retry flow.
    let outcome = engine
        .submit_user_message(
            session_id,
            "owner-a",
            vec![NewPart {
                kind: "tool_call".to_owned(),
                role: PartRole::Assistant,
                content: json!({ "name": "fs.read", "input": {} }),
                summary: None,
                visibility: PartVisibility::Both,
                rendered_markdown: None,
                parent_part_id: None,
                state: PartState::InProgress,
            }],
            None,
            1_000_000,
        )
        .await
        .expect("submit");
    let view = engine.load_session(session_id).await.expect("load");
    let text_id = view.parts[1].part_id;
    let _ = outcome;

    // Fail the text part, then retry it -> revision bumps to 2.
    let failed = engine
        .update_part(
            session_id,
            "owner-a",
            text_id,
            agena_storage::store::PartDelta {
                state: Some(PartState::Failed),
                ..Default::default()
            },
            1_000_000,
        )
        .await
        .expect("fail text part");
    assert_eq!(failed.state, PartState::Failed);
    let retried = engine
        .update_part(
            session_id,
            "owner-a",
            text_id,
            agena_storage::store::PartDelta {
                state: Some(PartState::InProgress),
                ..Default::default()
            },
            1_000_001,
        )
        .await
        .expect("retry text part");
    assert_eq!(retried.state, PartState::InProgress);
    // Revision 1 at creation, +1 for the fail, +1 for the retry.
    assert_eq!(retried.revision, 3);
    assert_eq!(retried.finished_at_ms, None, "retry clears finished_at");

    // A failed run marker is terminal: retrying is a new continue run (18.2).
    let run_id = view.parts[0].part_id;
    engine
        .complete_run(
            session_id,
            "owner-a",
            run_id,
            RunOutcome {
                status: PartState::Failed,
                abort_reason: Some("process_restart".to_owned()),
                content: None,
                provider_state: None,
            },
            1_000_001,
        )
        .await
        .expect("fail run");
    let error = engine
        .update_part(
            session_id,
            "owner-a",
            run_id,
            agena_storage::store::PartDelta {
                state: Some(PartState::InProgress),
                ..Default::default()
            },
            1_000_002,
        )
        .await
        .expect_err("run marker is terminal");
    assert!(matches!(
        error,
        agena_storage::store::StoreError::InvalidState(_)
    ));
}

#[tokio::test]
async fn idempotency_key_deduplicates_user_send() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;

    let first = engine
        .submit_user_message(
            session_id,
            "owner-a",
            vec![text_part("once")],
            Some("key-1".to_owned()),
            1_000_000,
        )
        .await
        .expect("first send");
    assert!(first.created);

    let replay = engine
        .submit_user_message(
            session_id,
            "owner-a",
            vec![text_part("once")],
            Some("key-1".to_owned()),
            1_000_000,
        )
        .await
        .expect("replay");
    assert!(!replay.created, "replay returns the prior run");
    assert_eq!(replay.run_id, first.run_id);
    // The replay returns the prior run's content parts (the marker itself is
    // not a member of its own run, so it is excluded), matching the in-memory
    // engine's `run_parts`.
    assert_eq!(replay.parts, first.parts[1..]);

    let view = engine.load_session(session_id).await.expect("load");
    assert_eq!(view.parts.len(), 2, "no duplicate parts were created");
}

#[tokio::test]
async fn gc_deletes_only_refcount_orphans() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let (run_id, _view) = submit_hello(&engine, session_id).await;

    // Complete the run so rows are no longer referenced by an active run.
    engine
        .complete_run(
            session_id,
            "owner-a",
            run_id,
            RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: None,
                provider_state: None,
            },
            1_000_001,
        )
        .await
        .expect("complete run");

    // Delete the session: membership edges cascade, parts become orphans.
    engine.delete_session(session_id).await.expect("delete");
    let outcome = engine.maintenance(1_000_002).await.expect("maintenance");
    assert!(outcome.reaped_sessions.is_empty());
    assert_eq!(outcome.gc_deleted_parts, 2, "marker + text both GC'd");

    let view = engine.load_session(session_id).await;
    assert!(matches!(
        view,
        Err(agena_storage::store::StoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn jsonl_round_trip_preserves_ordering_and_references() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db.clone()).await;
    let (_run_id, _view) = submit_hello(&engine, session_id).await;

    let bundle = engine
        .export_session_jsonl(session_id)
        .await
        .expect("export");

    let workspace_id = SeaWorkspaceRepository::new(db)
        .ensure_id("/import/workspace")
        .await
        .expect("workspace");
    let imported = engine
        .import_session_jsonl(workspace_id, &bundle, 1_000_000)
        .await
        .expect("import");

    let original = engine.load_session(session_id).await.expect("original");
    let restored = engine.load_session(imported).await.expect("restored");
    assert_eq!(restored.parts.len(), original.parts.len());
    for (left, right) in restored.parts.iter().zip(original.parts.iter()) {
        assert_eq!(left.kind, right.kind);
        assert_eq!(left.role, right.role);
        assert_eq!(left.state, right.state);
        assert_eq!(left.content, right.content);
        assert_eq!(left.run_id.is_some(), right.run_id.is_some());
        // References were remapped consistently: the marker is the first part.
        assert!(restored.parts[0].is_run_marker());
    }
    assert_eq!(restored.parts[0].run_id, None, "marker stays a root");
}

#[tokio::test]
async fn usage_stats_groups_by_provider_and_model() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;

    engine
        .record_usage(agena_storage::store::UsageRecord {
            workspace_id: 1,
            session_id,
            run_id: None,
            provider_id: "anthropic".to_owned(),
            model_id: "sonnet".to_owned(),
            created_at_ms: 1_000_000,
            input_tokens: 100,
            output_tokens: 50,
            reasoning_tokens: 10,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            tool_use_tokens: 0,
            other_tokens: 0,
            total_cost_micros: 25,
            recorded_cost_micros: None,
            cost_estimate_incomplete: false,
            detail_json: None,
        })
        .await
        .expect("record usage 1");
    engine
        .record_usage(agena_storage::store::UsageRecord {
            workspace_id: 1,
            session_id,
            run_id: None,
            provider_id: "anthropic".to_owned(),
            model_id: "sonnet".to_owned(),
            created_at_ms: 1_000_001,
            input_tokens: 200,
            output_tokens: 75,
            reasoning_tokens: 5,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            tool_use_tokens: 0,
            other_tokens: 0,
            total_cost_micros: 50,
            recorded_cost_micros: None,
            cost_estimate_incomplete: false,
            detail_json: None,
        })
        .await
        .expect("record usage 2");
    engine
        .record_usage(agena_storage::store::UsageRecord {
            workspace_id: 1,
            session_id,
            run_id: None,
            provider_id: "openai".to_owned(),
            model_id: "gpt".to_owned(),
            created_at_ms: 1_000_002,
            input_tokens: 10,
            output_tokens: 5,
            reasoning_tokens: 0,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            tool_use_tokens: 0,
            other_tokens: 0,
            total_cost_micros: 3,
            recorded_cost_micros: None,
            cost_estimate_incomplete: false,
            detail_json: None,
        })
        .await
        .expect("record usage 3");

    let stats = engine
        .usage_stats(agena_storage::store::UsageQuery::default())
        .await
        .expect("stats");
    assert_eq!(stats.total_calls, 3);
    assert_eq!(stats.groups.len(), 2);
    let anthropic = stats
        .groups
        .iter()
        .find(|group| group.provider_id == "anthropic")
        .expect("anthropic group");
    assert_eq!(anthropic.calls, 2);
    assert_eq!(anthropic.input_tokens, 300);
    assert_eq!(anthropic.total_cost_micros, 75);
}

async fn connect_file(tempdir: &tempfile::TempDir, name: &str) -> Arc<sea_orm::DatabaseConnection> {
    let path = tempdir.path().join(name);
    Arc::new(
        sea_orm::Database::connect(format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("connect file database"),
    )
}

/// Two independent connection pools over one file model the multi-process
/// deployment: process A holds the lease, goes silent past staleness, and
/// process B steals it — the residual run is aborted atomically (gate 5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lease_steal_aborts_stale_run_across_processes() {
    let directory = tempfile::tempdir().expect("temp directory");
    let db_a = connect_file(&directory, "steal.db").await;
    initialize_schema(&db_a).await.expect("schema");
    let engine_a = SqliteEngine::new(db_a.clone());

    let workspace_id = SeaWorkspaceRepository::new(db_a.clone())
        .ensure_id("/x/ws")
        .await
        .expect("workspace");
    let meta = engine_a
        .create_session(NewSession {
            workspace_id,
            parent_id: None,
            relation_kind: SessionRelationKind::Root,
            cutoff_part_id: None,
            title: "cross-process".to_owned(),
            task_id: None,
            config_json: None,
            provider_anchors_json: None,
        })
        .await
        .expect("create session");
    let session_id = meta.id;

    let acquire = engine_a
        .try_acquire_lease(session_id, "proc-a", 1_000_000)
        .await
        .expect("proc-a acquires");
    assert!(matches!(
        acquire,
        LeaseAcquire::Acquired { reconciled_runs } if reconciled_runs.is_empty()
    ));
    let run_id = engine_a
        .submit_user_message(
            session_id,
            "proc-a",
            vec![text_part("hello")],
            None,
            1_000_000,
        )
        .await
        .expect("proc-a submits")
        .run_id;

    // Process B (a separate connection pool) steals after proc-a's lease
    // goes stale.
    let db_b = connect_file(&directory, "steal.db").await;
    let engine_b = SqliteEngine::new(db_b);
    let acquire = engine_b
        .try_acquire_lease(session_id, "proc-b", 1_000_000 + 60_000)
        .await
        .expect("proc-b steals");
    assert!(matches!(
        acquire,
        LeaseAcquire::Acquired { reconciled_runs } if reconciled_runs == vec![run_id]
    ));

    // Process A (now a reader) sees the aborted run.
    let view = engine_a.load_session(session_id).await.expect("load via A");
    let marker = view
        .parts
        .iter()
        .find(|part| part.is_run_marker())
        .expect("marker");
    assert_eq!(marker.state, PartState::Failed);
    assert_eq!(marker.content["abort_reason"], "lease_stolen");
    let text = view
        .parts
        .iter()
        .find(|part| part.kind == "text")
        .expect("text");
    assert_eq!(text.state, PartState::Cancelled);
}

/// A facade caches a session view, a second facade (separate connection pool,
/// modeling another process) writes to the same file, and the first facade's
/// cache is invalidated on the next read — cross-process catch-up through the
/// facade via version + newest-member cursor (gate 5, 14.4).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn facade_cross_process_cache_invalidation() {
    let directory = tempfile::tempdir().expect("temp directory");
    let db_a = connect_file(&directory, "facade.db").await;
    initialize_schema(&db_a).await.expect("schema");
    let engine_a = SqliteEngine::new(db_a.clone());
    let facade_a = SessionFacade::new(engine_a, "owner-a", 16);

    let workspace_id = SeaWorkspaceRepository::new(db_a.clone())
        .ensure_id("/f/ws")
        .await
        .expect("workspace");
    let session_id = facade_a
        .engine()
        .create_session(NewSession {
            workspace_id,
            parent_id: None,
            relation_kind: SessionRelationKind::Root,
            cutoff_part_id: None,
            title: "cache".to_owned(),
            task_id: None,
            config_json: None,
            provider_anchors_json: None,
        })
        .await
        .expect("create session")
        .id;
    facade_a
        .submit_user_message(
            session_id,
            "owner-a",
            vec![NewPart::pending(
                "text",
                PartRole::User,
                json!({"text": "first"}),
            )],
            None,
        )
        .await
        .expect("first submit");

    // facade_a caches the two-part view.
    let cached = facade_a.load(session_id).await.expect("cached load");
    assert_eq!(cached.parts.len(), 2);

    // Process B (fresh connection) appends a part through its own facade.
    let db_b = connect_file(&directory, "facade.db").await;
    let facade_b = SessionFacade::new(SqliteEngine::new(db_b), "owner-a", 16);
    let run_id = cached.parts[0].part_id;
    facade_b
        .append_parts(
            session_id,
            "owner-a",
            run_id,
            vec![NewPart::pending(
                "text",
                PartRole::Assistant,
                json!({"text": "second"}),
            )],
        )
        .await
        .expect("process B appends");

    // facade_a's cache is invalidated (cursor moved); it sees the new part.
    let refreshed = facade_a.load(session_id).await.expect("refreshed load");
    assert_eq!(refreshed.parts.len(), 3, "cache invalidated, catch-up read");
    assert!(
        refreshed
            .parts
            .iter()
            .any(|part| part.content["text"] == "second"),
        "newest member cursor catch-up sees process B's part"
    );
}

/// Process B reads exactly what process A committed — cross-process catch-up
/// is a plain read of the shared database (gate 5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn second_process_reads_committed_parts() {
    let directory = tempfile::tempdir().expect("temp directory");
    let db_a = connect_file(&directory, "catchup.db").await;
    initialize_schema(&db_a).await.expect("schema");
    let engine_a = SqliteEngine::new(db_a.clone());

    let workspace_id = SeaWorkspaceRepository::new(db_a.clone())
        .ensure_id("/y/ws")
        .await
        .expect("workspace");
    let meta = engine_a
        .create_session(NewSession {
            workspace_id,
            parent_id: None,
            relation_kind: SessionRelationKind::Root,
            cutoff_part_id: None,
            title: "catchup".to_owned(),
            task_id: None,
            config_json: None,
            provider_anchors_json: None,
        })
        .await
        .expect("create session");
    let session_id = meta.id;
    engine_a
        .try_acquire_lease(session_id, "proc-a", 1_000_000)
        .await
        .expect("acquire");
    let run_id = engine_a
        .submit_user_message(
            session_id,
            "proc-a",
            vec![text_part("hello")],
            None,
            1_000_000,
        )
        .await
        .expect("submit")
        .run_id;
    let _ = run_id;

    // Process B opens a fresh connection to the same file and reads the
    // committed parts without any local state.
    let db_b = connect_file(&directory, "catchup.db").await;
    initialize_schema(&db_b).await.expect("schema (idempotent)");
    let engine_b = SqliteEngine::new(db_b);
    let view = engine_b.load_session(session_id).await.expect("B loads");
    assert_eq!(view.parts.len(), 2);
    assert!(view.parts[0].is_run_marker());
    assert_eq!(view.parts[1].content["text"], "hello");
}

#[tokio::test]
async fn subagent_helpers_find_create_and_update_subtask_state() {
    let db = in_memory_db().await;
    let (engine, parent_id) = setup(db).await;
    let child = engine
        .create_subagent_session(parent_id, "task-9".to_owned(), "sub".to_owned(), 1_000_000)
        .await
        .expect("create subagent");
    assert_eq!(child.parent_id, Some(parent_id));
    assert_eq!(child.task_id.as_deref(), Some("task-9"));
    assert_eq!(child.relation_kind, SessionRelationKind::Subagent);
    // Depth and root follow the hierarchy invariant: depth = parent.depth + 1,
    // root inherited from the parent.
    let parent_meta = engine.session_meta(parent_id).await.expect("parent meta");
    assert_eq!(child.depth, parent_meta.depth + 1);
    assert_eq!(child.root_id, parent_meta.root_id);

    let found = engine
        .find_subagent_by_task_id(parent_id, "task-9")
        .await
        .expect("find")
        .expect("subagent exists");
    assert_eq!(found.id, child.id);

    let missing = engine
        .find_subagent_by_task_id(parent_id, "nope")
        .await
        .expect("find missing");
    assert!(missing.is_none(), "unknown task id yields None");

    // `running`: started, no finish, no failure (schema lifecycle trigger).
    let updated = engine
        .update_subtask_state(
            child.id,
            Some("running".to_owned()),
            Some(1_000_001),
            None,
            None,
        )
        .await
        .expect("update subtask");
    assert_eq!(updated.subtask_status.as_deref(), Some("running"));
    assert_eq!(updated.subtask_started_at_ms, Some(1_000_001));
    assert_eq!(updated.subtask_finished_at_ms, None);
    assert_eq!(updated.subtask_failure, None);

    // `failed`: started + finished + full failure shape.
    let failure = json!({
        "id": "task-9",
        "code": "execution_failed",
        "user": {"fallback": "The subtask failed."}
    });
    let failed = engine
        .update_subtask_state(
            child.id,
            Some("failed".to_owned()),
            Some(1_000_001),
            Some(1_000_002),
            Some(failure.clone()),
        )
        .await
        .expect("update subtask");
    assert_eq!(failed.subtask_status.as_deref(), Some("failed"));
    assert_eq!(failed.subtask_finished_at_ms, Some(1_000_002));
    assert_eq!(failed.subtask_failure, Some(failure));

    // The unique (parent_id, task_id) index refuses a duplicate create.
    let err = engine
        .create_subagent_session(parent_id, "task-9".to_owned(), "dup".to_owned(), 1_000_000)
        .await
        .expect_err("duplicate subagent refused");
    assert!(
        matches!(
            err,
            agena_storage::store::StoreError::Database(_)
                | agena_storage::store::StoreError::InvalidState(_)
        ),
        "duplicate (parent, task) is rejected: {err:?}"
    );
}
