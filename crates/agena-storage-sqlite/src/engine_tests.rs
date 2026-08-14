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
        BackgroundDeliveryPhase, BackgroundEventRequest, BackgroundOperationKind,
        BackgroundOperationPhase, BackgroundOperationTransition, LeaseAcquire,
        NewBackgroundOperation, NewPart, NewSession, PartDelta, PartRole, PartState,
        PartVisibility, PersistenceEngine, RunOutcome, SessionFacade, SessionListQuery,
        SessionStore, SessionView,
    },
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
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
        matches!(acquire, LeaseAcquire::Acquired { reconciled_runs, .. } if reconciled_runs.is_empty())
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

fn completed_text_part(text: &str) -> NewPart {
    let mut part = text_part(text);
    part.state = PartState::Completed;
    part
}

async fn submit_hello(engine: &SqliteEngine, session_id: i64) -> (i64, SessionView) {
    let outcome = engine
        .submit_user_run(
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
async fn completed_user_send_is_a_terminal_input_receipt_not_a_liveness_guard() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let outcome = engine
        .submit_user_run(
            session_id,
            "owner-a",
            vec![completed_text_part("already committed")],
            None,
            1_000_000,
        )
        .await
        .expect("submit completed input");
    let marker = &outcome.parts[0];
    assert_eq!(marker.state, PartState::Completed);
    assert!(marker.finished_at_ms.is_some());
    assert_eq!(marker.content["abort_reason"], serde_json::Value::Null);

    let view = engine.load_session(session_id).await.expect("load input");
    assert!(
        view.parts
            .iter()
            .filter(|part| part.is_run_marker())
            .all(|part| part.state.is_terminal()),
        "a completed input contributes no in-flight run marker"
    );
}

#[tokio::test]
async fn semantic_checkpoint_flushes_a_buffered_part_before_companion_append() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let durable_probe = engine.clone();
    let facade = SessionFacade::new(engine, "owner-a", 16)
        // Keep the synthetic InProgress content update buffered until the
        // background transaction explicitly checkpoints it.
        .with_streaming_flush_delta_count(64);

    let mut tool = NewPart::pending(
        "tool_call",
        PartRole::Assistant,
        json!({"operation": {"phase": "starting"}}),
    );
    tool.state = PartState::InProgress;
    let launched = facade
        .submit_user_run(session_id, "owner-a", vec![tool], None)
        .await
        .expect("submit launching run");
    let run_id = launched.run_id;
    let tool_part_id = launched.parts[1].part_id;
    let marker_content = json!({
        "operation": {
            "phase": "launched",
            "metadata": {
                "agena.background": {"kind": "shell", "id": "proc_atomic"}
            }
        }
    });

    // Reproduce the original failure shape: this semantic InProgress update
    // enters D10's stream buffer and is visible through the facade, but is not
    // in the durable engine row yet.
    facade
        .update_part(
            session_id,
            "owner-a",
            tool_part_id,
            PartDelta {
                state: Some(PartState::InProgress),
                content: Some(marker_content.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("buffer background marker");
    let before = durable_probe
        .load_session(session_id)
        .await
        .expect("load durable row before checkpoint");
    assert_eq!(
        before
            .parts
            .iter()
            .find(|part| part.part_id == tool_part_id)
            .expect("durable tool part")
            .content["operation"]["phase"],
        "starting",
        "the fixture must prove the marker is still memory-only"
    );

    let mut guard = NewPart::pending(
        "tool_result",
        PartRole::Tool,
        json!({"ok": true, "output": ""}),
    );
    guard.state = PartState::Completed;
    guard.parent_part_id = Some(tool_part_id);
    facade
        .settle_background_run(
            session_id,
            "owner-a",
            run_id,
            Some((tool_part_id, PartState::InProgress, marker_content.clone())),
            vec![guard],
        )
        .await
        .expect("atomically checkpoint launch");

    // Read through a separate engine handle, bypassing the facade overlay:
    // both halves must now be durable and the launching run must stay open.
    let after = durable_probe
        .load_session(session_id)
        .await
        .expect("load durable launch checkpoint");
    let durable_tool = after
        .parts
        .iter()
        .find(|part| part.part_id == tool_part_id)
        .expect("checkpointed tool part");
    assert_eq!(durable_tool.state, PartState::InProgress);
    assert_eq!(durable_tool.finished_at_ms, None);
    assert_eq!(durable_tool.content, marker_content);
    assert!(after.parts.iter().any(|part| {
        part.kind == "tool_result"
            && part.parent_part_id == Some(tool_part_id)
            && part.state == PartState::Completed
    }));
    assert!(
        after
            .parts
            .iter()
            .find(|part| part.part_id == run_id)
            .expect("launching run")
            .state
            .is_in_flight(),
        "an atomic launch checkpoint must not terminalize its run"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_events_are_atomic_idempotent_and_safe_under_out_of_order_concurrency() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let launched = engine
        .submit_user_run(
            session_id,
            "owner-a",
            vec![NewPart::pending(
                "tool_call",
                PartRole::Assistant,
                json!({"operation": {"title": "monitor launch receipt"}}),
            )],
            None,
            1_000_000,
        )
        .await
        .expect("submit launch receipt");
    let tool_part_id = launched.parts[1].part_id;
    let operation_id = format!("bg_{session_id}_{tool_part_id}");
    let created = engine
        .create_background_operation(
            NewBackgroundOperation {
                operation_id: operation_id.clone(),
                session_id,
                launch_run_id: Some(launched.run_id),
                launch_tool_part_id: Some(tool_part_id),
                kind: BackgroundOperationKind::Monitor,
            },
            1_000_001,
        )
        .await
        .expect("create launch intent");
    assert_eq!(created.phase, BackgroundOperationPhase::LaunchRequested);
    let launching = engine
        .transition_background_operation(
            BackgroundOperationTransition {
                operation_id: operation_id.clone(),
                expected_revision: created.revision,
                next_phase: BackgroundOperationPhase::Launching,
                external_id: Some("proc_concurrent".to_owned()),
                outcome: None,
                failure: None,
                owner_id: Some("launch-owner".to_owned()),
                lease_until_ms: Some(1_030_000),
            },
            1_000_002,
        )
        .await
        .expect("transition launching");
    let running = engine
        .transition_background_operation(
            BackgroundOperationTransition {
                operation_id: operation_id.clone(),
                expected_revision: launching.revision,
                next_phase: BackgroundOperationPhase::Running,
                external_id: Some("proc_concurrent".to_owned()),
                outcome: None,
                failure: None,
                owner_id: None,
                lease_until_ms: None,
            },
            1_000_003,
        )
        .await
        .expect("transition running");
    assert_eq!(running.phase, BackgroundOperationPhase::Running);

    let request = |seq: u64| BackgroundEventRequest {
        operation_id: operation_id.clone(),
        event_key: format!("event:{seq}"),
        event_seq: Some(seq),
        next_phase: None,
        outcome: None,
        failure: None,
        notification: {
            let mut part = NewPart::pending(
                "system_notification",
                PartRole::Runtime,
                json!({
                    "operation_id": "proc_concurrent",
                    "operation_kind": "monitor",
                    "status": "event",
                    "event_seq": seq,
                    "summary": format!("event {seq}"),
                    "body": format!("event {seq}")
                }),
            );
            part.state = PartState::Completed;
            part
        },
    };
    let first_engine = engine.clone();
    let second_engine = engine.clone();
    let (seq_two, seq_one) = tokio::join!(
        first_engine.record_background_event(request(2), 1_000_004),
        second_engine.record_background_event(request(1), 1_000_005),
    );
    assert!(seq_two.expect("record seq 2").created);
    assert!(seq_one.expect("record seq 1").created);
    let operation = engine
        .background_operation(&operation_id)
        .await
        .expect("load operation")
        .expect("operation exists");
    assert_eq!(operation.phase, BackgroundOperationPhase::Running);
    assert_eq!(
        operation.last_event_seq, 2,
        "cursor keeps max seen sequence"
    );

    let duplicate = engine
        .record_background_event(request(1), 1_000_006)
        .await
        .expect("replay seq 1");
    assert!(!duplicate.created, "same operation/event key is idempotent");
    let view = engine
        .load_session(session_id)
        .await
        .expect("load transcript");
    let notifications = view
        .parts
        .iter()
        .filter(|part| part.kind == "system_notification")
        .collect::<Vec<_>>();
    assert_eq!(notifications.len(), 2);
    assert!(
        notifications
            .iter()
            .all(|part| part.role == PartRole::Runtime)
    );
    assert!(notifications.iter().all(|part| {
        part.run_id.is_some_and(|run_id| {
            view.parts.iter().any(|marker| {
                marker.part_id == run_id
                    && marker.is_run_marker()
                    && marker.role == PartRole::Runtime
                    && marker.content["run_kind"] == "runtime_ingress"
            })
        })
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_delivery_claim_is_exclusive_expirable_and_retryable() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let operation = engine
        .create_background_operation(
            NewBackgroundOperation {
                operation_id: format!("scheduled:{session_id}:claim-test"),
                session_id,
                launch_run_id: None,
                launch_tool_part_id: None,
                kind: BackgroundOperationKind::ScheduledDelivery,
            },
            1_000_000,
        )
        .await
        .expect("create scheduled operation");
    let launching = engine
        .transition_background_operation(
            BackgroundOperationTransition {
                operation_id: operation.operation_id.clone(),
                expected_revision: operation.revision,
                next_phase: BackgroundOperationPhase::Launching,
                external_id: None,
                outcome: None,
                failure: None,
                owner_id: Some("scheduler".to_owned()),
                lease_until_ms: Some(1_030_000),
            },
            1_000_001,
        )
        .await
        .expect("launching");
    let _running = engine
        .transition_background_operation(
            BackgroundOperationTransition {
                operation_id: operation.operation_id.clone(),
                expected_revision: launching.revision,
                next_phase: BackgroundOperationPhase::Running,
                external_id: Some("claim-test".to_owned()),
                outcome: None,
                failure: None,
                owner_id: None,
                lease_until_ms: None,
            },
            1_000_002,
        )
        .await
        .expect("running");
    let mut notification = NewPart::pending(
        "system_notification",
        PartRole::Runtime,
        json!({
            "operation_id": "claim-test",
            "operation_kind": "scheduled_delivery",
            "status": "submitted",
            "summary": "scheduled",
            "body": "scheduled"
        }),
    );
    notification.state = PartState::Completed;
    let settled = engine
        .record_background_event(
            BackgroundEventRequest {
                operation_id: operation.operation_id,
                event_key: "terminal".to_owned(),
                event_seq: None,
                next_phase: Some(BackgroundOperationPhase::Completed),
                outcome: None,
                failure: None,
                notification,
            },
            1_000_003,
        )
        .await
        .expect("record scheduled terminal");
    assert_eq!(settled.delivery.phase, BackgroundDeliveryPhase::Pending);

    let left = engine.clone();
    let right = engine.clone();
    let delivery_id = settled.delivery.delivery_id.clone();
    let delivery_id_right = delivery_id.clone();
    let (claim_a, claim_b) = tokio::join!(
        left.claim_background_delivery(&delivery_id, "dispatcher-a", 1_000_100, 1_000_010),
        right.claim_background_delivery(&delivery_id_right, "dispatcher-b", 1_000_100, 1_000_010,),
    );
    let claims = [claim_a.expect("claim a"), claim_b.expect("claim b")];
    assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
    let first_owner = claims
        .iter()
        .find_map(|claim| {
            claim
                .as_ref()
                .and_then(|delivery| delivery.claim_owner.clone())
        })
        .expect("one claim owner");
    assert!(
        engine
            .claim_background_delivery(&delivery_id, "too-early", 1_000_200, 1_000_099)
            .await
            .expect("early claim")
            .is_none()
    );
    let reclaimed = engine
        .claim_background_delivery(&delivery_id, "recovery", 1_000_300, 1_000_100)
        .await
        .expect("expired claim")
        .expect("expired claim is reclaimable");
    assert_eq!(reclaimed.claim_owner.as_deref(), Some("recovery"));
    assert_ne!(reclaimed.claim_owner.as_deref(), Some(first_owner.as_str()));
    let pending = engine
        .retry_background_delivery(
            &delivery_id,
            "recovery",
            json!({"message": "wake failed"}),
            1_000_110,
        )
        .await
        .expect("release failed claim");
    assert_eq!(pending.phase, BackgroundDeliveryPhase::Pending);
    let final_claim = engine
        .claim_background_delivery(&delivery_id, "final", 1_000_400, 1_000_120)
        .await
        .expect("final claim")
        .expect("pending delivery claimable");
    let consumed = engine
        .consume_background_delivery(&final_claim.delivery_id, "final", 1_000_130)
        .await
        .expect("consume delivery");
    assert_eq!(consumed.phase, BackgroundDeliveryPhase::Consumed);
}

#[tokio::test]
async fn writes_without_a_fresh_lease_are_refused() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;

    // A different owner is refused outright.
    let held = engine
        .submit_user_run(
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
        .submit_user_run(
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
        LeaseAcquire::Acquired { reconciled_runs, .. } if reconciled_runs == vec![run_id]
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
async fn fork_during_streaming_shares_parent_updates_and_child_diverges_by_append() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let outcome = engine
        .submit_user_run(
            session_id,
            "owner-a",
            vec![NewPart {
                kind: "text".to_owned(),
                role: PartRole::Assistant,
                content: json!({"text": "partial"}),
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
        .expect("start streaming part");
    let run_id = outcome.run_id;
    let streamed_id = outcome.parts[1].part_id;

    let child = engine
        .fork_session(
            session_id,
            streamed_id,
            "streaming fork".to_owned(),
            false,
            1_000_000,
        )
        .await
        .expect("fork while parent part is in progress");
    engine
        .try_acquire_lease(child.id, "owner-child", 1_000_000)
        .await
        .expect("child acquires independent lease");

    engine
        .update_part(
            session_id,
            "owner-a",
            streamed_id,
            agena_storage::store::PartDelta {
                state: Some(PartState::Completed),
                content_text_delta: Some(" complete".to_owned()),
                ..Default::default()
            },
            1_000_001,
        )
        .await
        .expect("parent completes its streamed row");
    let child_after_parent_write = engine.load_session(child.id).await.expect("load child");
    let shared = child_after_parent_write
        .parts
        .iter()
        .find(|part| part.part_id == streamed_id)
        .expect("shared streamed part");
    assert_eq!(shared.state, PartState::Completed);
    assert_eq!(shared.content["text"], "partial complete");

    let mutation_error = engine
        .update_part(
            child.id,
            "owner-child",
            streamed_id,
            agena_storage::store::PartDelta {
                content: Some(json!({"text": "child overwrite"})),
                ..Default::default()
            },
            1_000_002,
        )
        .await
        .expect_err("child cannot mutate a parent-origin shared part");
    assert!(matches!(
        mutation_error,
        agena_storage::store::StoreError::InvalidState(_)
    ));

    let divergence = engine
        .append_parts(
            child.id,
            "owner-child",
            run_id,
            vec![NewPart::pending(
                "text",
                PartRole::Assistant,
                json!({"text": "child continuation"}),
            )],
            1_000_003,
        )
        .await
        .expect("child diverges by appending a child-origin part");
    assert_eq!(divergence.len(), 1);
    assert_eq!(divergence[0].origin_session_id, child.id);
    let child_final = engine
        .load_session(child.id)
        .await
        .expect("load child final");
    assert!(
        child_final
            .parts
            .iter()
            .any(|part| part.content["text"] == "child continuation")
    );
}

#[tokio::test]
async fn retry_transitions_failed_to_in_progress_with_revision_bump_but_not_for_runs() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    // A tool_call created `in_progress` (not `pending`), so
    // in_progress -> failed -> in_progress is the valid retry flow.
    let outcome = engine
        .submit_user_run(
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
async fn retry_history_keeps_the_durable_error_beside_the_successful_result() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let outcome = engine
        .submit_user_run(
            session_id,
            "owner-a",
            vec![NewPart {
                kind: "tool_call".to_owned(),
                role: PartRole::Assistant,
                content: json!({"name": "fs.read", "input": {"path": "missing"}}),
                summary: Some("read missing".to_owned()),
                visibility: PartVisibility::Both,
                rendered_markdown: None,
                parent_part_id: None,
                state: PartState::InProgress,
            }],
            None,
            1_000_000,
        )
        .await
        .expect("start tool operation");
    let run_id = outcome.run_id;
    let tool_id = outcome.parts[1].part_id;

    engine
        .update_part(
            session_id,
            "owner-a",
            tool_id,
            agena_storage::store::PartDelta {
                state: Some(PartState::Failed),
                ..Default::default()
            },
            1_000_001,
        )
        .await
        .expect("first attempt fails");
    engine
        .append_parts(
            session_id,
            "owner-a",
            run_id,
            vec![NewPart {
                kind: "error".to_owned(),
                role: PartRole::Runtime,
                content: json!({
                    "code": "tool.read_failed",
                    "message": "file was temporarily unavailable",
                    "retryable": true,
                    "attempt": 1
                }),
                summary: Some("attempt 1 failed".to_owned()),
                visibility: PartVisibility::Both,
                rendered_markdown: None,
                parent_part_id: Some(tool_id),
                state: PartState::Failed,
            }],
            1_000_002,
        )
        .await
        .expect("append durable error");
    let retried = engine
        .update_part(
            session_id,
            "owner-a",
            tool_id,
            agena_storage::store::PartDelta {
                state: Some(PartState::InProgress),
                ..Default::default()
            },
            1_000_003,
        )
        .await
        .expect("retry same operation part");
    assert_eq!(retried.state, PartState::InProgress);
    assert!(retried.revision >= 3);
    engine
        .append_parts(
            session_id,
            "owner-a",
            run_id,
            vec![NewPart {
                kind: "tool_result".to_owned(),
                role: PartRole::Tool,
                content: json!({"output": "contents", "ok": true}),
                summary: Some("read succeeded".to_owned()),
                visibility: PartVisibility::Both,
                rendered_markdown: Some("`contents`".to_owned()),
                parent_part_id: Some(tool_id),
                state: PartState::Completed,
            }],
            1_000_004,
        )
        .await
        .expect("append successful result");
    engine
        .update_part(
            session_id,
            "owner-a",
            tool_id,
            agena_storage::store::PartDelta {
                state: Some(PartState::Completed),
                ..Default::default()
            },
            1_000_005,
        )
        .await
        .expect("complete retried operation");
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
            1_000_006,
        )
        .await
        .expect("complete run");

    let history = engine
        .load_session(session_id)
        .await
        .expect("reload history");
    let error = history
        .parts
        .iter()
        .find(|part| part.kind == "error")
        .expect("durable error remains");
    let success = history
        .parts
        .iter()
        .find(|part| part.kind == "tool_result")
        .expect("successful result remains");
    assert_eq!(error.parent_part_id, Some(tool_id));
    assert_eq!(error.state, PartState::Failed);
    assert_eq!(success.parent_part_id, Some(tool_id));
    assert_eq!(success.state, PartState::Completed);
    assert_eq!(success.content["output"], "contents");
    assert_eq!(
        history
            .parts
            .iter()
            .find(|part| part.part_id == tool_id)
            .expect("tool operation")
            .state,
        PartState::Completed
    );
}

#[tokio::test]
async fn idempotency_key_deduplicates_user_send() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;

    let first = engine
        .submit_user_run(
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
        .submit_user_run(
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
async fn resume_mid_stream_without_a_lease_reconciles_to_ready() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let facade = SessionFacade::with_clock(
        engine.clone(),
        "owner-a",
        agena_storage::store::MemoryLayer::new(8),
        agena_storage::store::NotificationBus::new(),
        || 1_000_000,
    );
    let run_id = engine
        .submit_user_run(
            session_id,
            "owner-a",
            vec![NewPart {
                kind: "text".to_owned(),
                role: PartRole::Assistant,
                content: json!({"text": "partial"}),
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
        .expect("start stream")
        .run_id;
    assert!(
        engine
            .release_lease(session_id, "owner-a")
            .await
            .expect("release lease")
    );

    let interrupted = facade
        .session_state(session_id)
        .await
        .expect("derive interrupted");
    assert_eq!(
        interrupted.state,
        agena_storage::store::SessionState::Interrupted
    );
    facade
        .reconcile(session_id)
        .await
        .expect("reconcile interrupted stream");
    let ready = facade
        .session_state(session_id)
        .await
        .expect("derive ready");
    assert_eq!(ready.state, agena_storage::store::SessionState::Ready);
    let view = facade
        .load(session_id)
        .await
        .expect("reload reconciled stream");
    let marker = view
        .parts
        .iter()
        .find(|part| part.part_id == run_id)
        .expect("run marker");
    assert_eq!(marker.state, PartState::Failed);
    assert_eq!(marker.content["abort_reason"], "process_restart");
    assert_eq!(
        view.parts
            .iter()
            .find(|part| part.kind == "text")
            .expect("streamed part")
            .state,
        PartState::Cancelled
    );
}

#[tokio::test]
async fn resume_mid_ask_without_a_lease_remains_awaiting_user() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let facade = SessionFacade::with_clock(
        engine.clone(),
        "owner-a",
        agena_storage::store::MemoryLayer::new(8),
        agena_storage::store::NotificationBus::new(),
        || 1_000_000,
    );
    let outcome = engine
        .submit_user_run(
            session_id,
            "owner-a",
            vec![NewPart::pending(
                "interaction",
                PartRole::Runtime,
                json!({"kind": "ask_user", "prompt": "Continue?"}),
            )],
            None,
            1_000_000,
        )
        .await
        .expect("pause run on interaction");
    assert!(
        engine
            .release_lease(session_id, "owner-a")
            .await
            .expect("release lease")
    );

    let awaiting = facade
        .session_state(session_id)
        .await
        .expect("derive awaiting user");
    assert_eq!(
        awaiting.state,
        agena_storage::store::SessionState::AwaitingUser
    );
    let interaction_id = awaiting
        .pending_interaction
        .expect("pending interaction")
        .part_id;
    let view = facade.load(session_id).await.expect("reload paused run");
    assert!(
        view.parts
            .iter()
            .find(|part| part.part_id == outcome.run_id)
            .expect("run marker")
            .state
            .is_in_flight()
    );
    assert_eq!(
        view.parts
            .iter()
            .find(|part| part.part_id == interaction_id)
            .expect("interaction")
            .state,
        PartState::Pending
    );
}

#[tokio::test]
async fn resume_mid_tool_preserves_error_context_and_cancels_the_tool() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let facade = SessionFacade::with_clock(
        engine.clone(),
        "owner-a",
        agena_storage::store::MemoryLayer::new(8),
        agena_storage::store::NotificationBus::new(),
        || 1_000_000,
    );
    let outcome = engine
        .submit_user_run(
            session_id,
            "owner-a",
            vec![NewPart {
                kind: "tool_call".to_owned(),
                role: PartRole::Assistant,
                content: json!({"name": "shell", "input": {"command": "sleep"}}),
                summary: Some("running shell".to_owned()),
                visibility: PartVisibility::Both,
                rendered_markdown: None,
                parent_part_id: None,
                state: PartState::InProgress,
            }],
            None,
            1_000_000,
        )
        .await
        .expect("start tool");
    let tool_id = outcome.parts[1].part_id;
    engine
        .append_parts(
            session_id,
            "owner-a",
            outcome.run_id,
            vec![NewPart {
                kind: "error".to_owned(),
                role: PartRole::Runtime,
                content: json!({
                    "code": "tool.partial_failure",
                    "message": "last durable diagnostic"
                }),
                summary: Some("diagnostic".to_owned()),
                visibility: PartVisibility::User,
                rendered_markdown: None,
                parent_part_id: Some(tool_id),
                state: PartState::Failed,
            }],
            1_000_001,
        )
        .await
        .expect("append durable diagnostic");
    assert!(
        engine
            .release_lease(session_id, "owner-a")
            .await
            .expect("release lease")
    );
    assert_eq!(
        facade
            .session_state(session_id)
            .await
            .expect("derive interrupted tool")
            .state,
        agena_storage::store::SessionState::Interrupted
    );
    facade
        .reconcile(session_id)
        .await
        .expect("reconcile tool run");
    let view = facade
        .load(session_id)
        .await
        .expect("reload reconciled tool");
    assert_eq!(
        view.parts
            .iter()
            .find(|part| part.part_id == tool_id)
            .expect("tool part")
            .state,
        PartState::Cancelled
    );
    let marker = view
        .parts
        .iter()
        .find(|part| part.part_id == outcome.run_id)
        .expect("run marker");
    assert_eq!(marker.state, PartState::Failed);
    assert_eq!(marker.content["abort_reason"], "process_restart");
    let error = view
        .parts
        .iter()
        .find(|part| part.kind == "error")
        .expect("diagnostic remains reconstructible");
    assert_eq!(error.state, PartState::Failed);
    assert_eq!(error.content["message"], "last durable diagnostic");
}

#[tokio::test]
async fn jsonl_round_trip_preserves_ordering_and_references() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db.clone()).await;
    let outcome = engine
        .submit_user_run(
            session_id,
            "owner-a",
            vec![NewPart {
                kind: "tool_call".to_owned(),
                role: PartRole::Assistant,
                content: json!({"name": "fs.read", "input": {"path": "README.md"}}),
                summary: Some("read README".to_owned()),
                visibility: PartVisibility::Both,
                rendered_markdown: None,
                parent_part_id: None,
                state: PartState::InProgress,
            }],
            None,
            1_000_000,
        )
        .await
        .expect("start exportable run");
    let tool_id = outcome.parts[1].part_id;
    engine
        .append_parts(
            session_id,
            "owner-a",
            outcome.run_id,
            vec![NewPart {
                kind: "tool_result".to_owned(),
                role: PartRole::Tool,
                content: json!({"output": "hello", "ok": true}),
                summary: Some("read complete".to_owned()),
                visibility: PartVisibility::Both,
                rendered_markdown: Some("hello".to_owned()),
                parent_part_id: Some(tool_id),
                state: PartState::Completed,
            }],
            1_000_001,
        )
        .await
        .expect("append referenced result");
    engine
        .update_part(
            session_id,
            "owner-a",
            tool_id,
            agena_storage::store::PartDelta {
                state: Some(PartState::Completed),
                ..Default::default()
            },
            1_000_002,
        )
        .await
        .expect("complete tool");
    let provider_state = json!({"response_id": "resp-1", "thought_signature": "sig"});
    engine
        .complete_run(
            session_id,
            "owner-a",
            outcome.run_id,
            RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: None,
                provider_state: Some(provider_state.clone()),
            },
            1_000_003,
        )
        .await
        .expect("complete exportable run");
    let anchors = json!({"anthropic": {"previous_response_id": "resp-1"}});
    engine
        .set_provider_anchors(session_id, Some(anchors.clone()))
        .await
        .expect("persist anchors");

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
    assert_ne!(restored.meta.id, original.meta.id);
    assert_eq!(restored.meta.parent_id, None);
    assert_eq!(restored.meta.relation_kind, SessionRelationKind::Root);
    assert_eq!(restored.meta.cutoff_part_id, None);
    assert_eq!(restored.meta.depth, 0);
    assert_eq!(restored.meta.root_id, restored.meta.id);
    assert_eq!(restored.meta.provider_anchors_json, Some(anchors));
    for (left, right) in restored.parts.iter().zip(original.parts.iter()) {
        assert_ne!(left.part_id, right.part_id, "imports allocate fresh ids");
        assert_eq!(left.kind, right.kind);
        assert_eq!(left.role, right.role);
        assert_eq!(left.state, right.state);
        assert_eq!(left.content, right.content);
        assert_eq!(left.provider_state, right.provider_state);
    }
    assert!(restored.parts[0].is_run_marker());
    let restored_marker = restored.parts[0].part_id;
    let restored_tool = restored
        .parts
        .iter()
        .find(|part| part.kind == "tool_call")
        .expect("restored tool call");
    let restored_result = restored
        .parts
        .iter()
        .find(|part| part.kind == "tool_result")
        .expect("restored tool result");
    assert_eq!(restored.parts[0].run_id, None, "marker stays a root");
    assert_eq!(restored.parts[0].provider_state, Some(provider_state));
    assert_eq!(restored_tool.run_id, Some(restored_marker));
    assert_eq!(restored_result.run_id, Some(restored_marker));
    assert_eq!(restored_result.parent_part_id, Some(restored_tool.part_id));
    assert!(
        restored
            .parts
            .windows(2)
            .all(|pair| (pair[0].created_at_ms, pair[0].part_id)
                <= (pair[1].created_at_ms, pair[1].part_id)),
        "import preserves canonical ordering"
    );
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

#[tokio::test]
async fn usage_query_shapes_use_their_covering_range_indexes() {
    let db = in_memory_db().await;
    initialize_schema(&db).await.expect("schema");

    async fn plan_details(db: &sea_orm::DatabaseConnection, sql: &str) -> Vec<String> {
        db.query_all(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!("EXPLAIN QUERY PLAN {sql}"),
        ))
        .await
        .expect("explain query plan")
        .into_iter()
        .map(|row| row.try_get("", "detail").expect("plan detail"))
        .collect()
    }

    let aggregate = "SELECT provider_id, model_id, COUNT(*) AS calls, \
                     SUM(input_tokens), SUM(output_tokens), SUM(reasoning_tokens), \
                     SUM(cache_write_tokens), SUM(cache_read_tokens), SUM(total_cost_micros) \
                     FROM agena_usage";
    let session = plan_details(
        &db,
        &format!(
            "{aggregate} WHERE session_id = 1 AND created_at_ms >= 100 AND created_at_ms < 200 \
             GROUP BY provider_id, model_id ORDER BY provider_id, model_id"
        ),
    )
    .await;
    assert!(
        session
            .iter()
            .any(|detail| detail.contains("idx_agena_usage_session")),
        "session/time plan must use idx_agena_usage_session: {session:?}"
    );

    let workspace = plan_details(
        &db,
        &format!(
            "{aggregate} WHERE workspace_id = 1 AND created_at_ms >= 100 AND created_at_ms < 200 \
             GROUP BY provider_id, model_id ORDER BY provider_id, model_id"
        ),
    )
    .await;
    assert!(
        workspace
            .iter()
            .any(|detail| detail.contains("idx_agena_usage_ws_time")),
        "workspace/time plan must use idx_agena_usage_ws_time: {workspace:?}"
    );

    let provider_model = plan_details(
        &db,
        &format!(
            "{aggregate} WHERE provider_id = 'anthropic' AND model_id = 'sonnet' \
             AND created_at_ms >= 100 AND created_at_ms < 200 \
             GROUP BY provider_id, model_id ORDER BY provider_id, model_id"
        ),
    )
    .await;
    assert!(
        provider_model
            .iter()
            .any(|detail| detail.contains("idx_agena_usage_provider_model")),
        "provider/model/time plan must use idx_agena_usage_provider_model: {provider_model:?}"
    );
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
        LeaseAcquire::Acquired { reconciled_runs, .. } if reconciled_runs.is_empty()
    ));
    let run_id = engine_a
        .submit_user_run(
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
        LeaseAcquire::Acquired { reconciled_runs, .. } if reconciled_runs == vec![run_id]
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
/// cache is invalidated on the next read for both append and in-place updates
/// (including a shared fork prefix) — explicit snapshot catch-up through the
/// facade via session version + newest-member cursor (gate 5, 14.4).
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
        .submit_user_run(
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

    // Cache the three-part position, then update the existing newest part in
    // place from process B. The cursor does not move; sessions.version must be
    // what invalidates process A's cache.
    let updated_part_id = refreshed
        .parts
        .iter()
        .find(|part| part.content["text"] == "second")
        .expect("second part")
        .part_id;
    facade_b
        .update_part(
            session_id,
            "owner-a",
            updated_part_id,
            agena_storage::store::PartDelta {
                state: Some(PartState::InProgress),
                content: Some(json!({"text": "second, updated"})),
                ..Default::default()
            },
        )
        .await
        .expect("process B updates existing part");
    let refreshed_in_place = facade_a
        .load(session_id)
        .await
        .expect("reload after in-place update");
    assert_eq!(refreshed_in_place.parts.len(), 3);
    let updated = refreshed_in_place
        .parts
        .iter()
        .find(|part| part.part_id == updated_part_id)
        .expect("updated part");
    assert_eq!(updated.content["text"], "second, updated");
    assert_eq!(updated.revision, 2);

    // A fork caches the shared marker. Completing that marker through process
    // B must advance the child session's version as well as the origin's.
    let child_id = facade_a
        .fork(session_id, run_id, "shared cache".to_owned())
        .await
        .expect("fork");
    let child_cached = facade_a.load(child_id).await.expect("cache child");
    assert_eq!(child_cached.parts[0].state, PartState::Pending);
    facade_b
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
        )
        .await
        .expect("complete shared marker");
    let child_refreshed = facade_a.load(child_id).await.expect("reload child");
    assert_eq!(child_refreshed.parts[0].state, PartState::Completed);
    assert_eq!(child_refreshed.parts[0].revision, 2);
}

#[tokio::test]
async fn every_sqlite_part_mutation_bumps_version_but_idempotency_replay_does_not() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    assert_eq!(engine.session_meta(session_id).await.unwrap().version, 1);

    let submitted = engine
        .submit_user_run(
            session_id,
            "owner-a",
            vec![text_part("hello")],
            Some("send-1".to_owned()),
            1_000_000,
        )
        .await
        .expect("submit");
    assert_eq!(engine.session_meta(session_id).await.unwrap().version, 2);
    let replay = engine
        .submit_user_run(
            session_id,
            "owner-a",
            vec![text_part("ignored")],
            Some("send-1".to_owned()),
            1_000_001,
        )
        .await
        .expect("replay");
    assert!(!replay.created);
    assert_eq!(engine.session_meta(session_id).await.unwrap().version, 2);

    let appended = engine
        .append_parts(
            session_id,
            "owner-a",
            submitted.run_id,
            vec![NewPart::pending(
                "interaction",
                PartRole::Assistant,
                json!({"kind": "ask_user", "prompt": "Continue?"}),
            )],
            1_000_002,
        )
        .await
        .expect("append interaction");
    assert_eq!(engine.session_meta(session_id).await.unwrap().version, 3);

    engine
        .update_part(
            session_id,
            "owner-a",
            submitted.parts[1].part_id,
            PartDelta {
                state: Some(PartState::InProgress),
                ..Default::default()
            },
            1_000_003,
        )
        .await
        .expect("update");
    assert_eq!(engine.session_meta(session_id).await.unwrap().version, 4);

    engine
        .answer_interaction(
            session_id,
            "owner-a",
            appended[0].part_id,
            NewPart::pending("text", PartRole::User, json!({"text": "yes"})),
            1_000_004,
        )
        .await
        .expect("answer");
    assert_eq!(engine.session_meta(session_id).await.unwrap().version, 5);

    engine
        .complete_run(
            session_id,
            "owner-a",
            submitted.run_id,
            RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: None,
                provider_state: None,
            },
            1_000_005,
        )
        .await
        .expect("complete");
    assert_eq!(engine.session_meta(session_id).await.unwrap().version, 6);

    let cancelled = engine
        .start_run(
            session_id,
            "owner-a",
            "continue",
            json!({}),
            None,
            1_000_006,
        )
        .await
        .expect("start cancel run");
    assert_eq!(engine.session_meta(session_id).await.unwrap().version, 7);
    engine
        .cancel_run(session_id, "owner-a", cancelled.run_id, 1_000_007)
        .await
        .expect("cancel");
    assert_eq!(engine.session_meta(session_id).await.unwrap().version, 8);

    engine
        .start_run(
            session_id,
            "owner-a",
            "continue",
            json!({}),
            None,
            1_000_008,
        )
        .await
        .expect("start interrupted run");
    assert_eq!(engine.session_meta(session_id).await.unwrap().version, 9);
    engine
        .release_lease(session_id, "owner-a")
        .await
        .expect("release");
    engine
        .reconcile(session_id, 1_000_009)
        .await
        .expect("reconcile");
    assert_eq!(engine.session_meta(session_id).await.unwrap().version, 10);

    let workspace_id = engine
        .session_meta(session_id)
        .await
        .expect("meta")
        .workspace_id;
    let bundle = engine
        .export_session_jsonl(session_id)
        .await
        .expect("export");
    let imported_id = engine
        .import_session_jsonl(workspace_id, &bundle, 1_000_010)
        .await
        .expect("import");
    assert_eq!(engine.session_meta(imported_id).await.unwrap().version, 2);
}

#[tokio::test]
async fn sqlite_fork_cannot_answer_a_shared_interaction_in_place() {
    let db = in_memory_db().await;
    let (engine, session_id) = setup(db).await;
    let outcome = engine
        .submit_user_run(
            session_id,
            "owner-a",
            vec![NewPart::pending(
                "interaction",
                PartRole::Assistant,
                json!({"kind": "ask_user", "prompt": "Continue?"}),
            )],
            None,
            1_000_000,
        )
        .await
        .expect("submit interaction");
    let interaction_id = outcome.parts[1].part_id;
    let child = engine
        .fork_session(
            session_id,
            interaction_id,
            "fork".to_owned(),
            false,
            1_000_000,
        )
        .await
        .expect("fork");
    engine
        .try_acquire_lease(child.id, "child-owner", 1_000_000)
        .await
        .expect("child lease");

    let error = engine
        .answer_interaction(
            child.id,
            "child-owner",
            interaction_id,
            NewPart::pending("text", PartRole::User, json!({"text": "yes"})),
            1_000_000,
        )
        .await
        .expect_err("shared interaction is origin-owned");
    assert!(matches!(
        error,
        agena_storage::store::StoreError::InvalidState(_)
    ));
    let parent = engine.load_session(session_id).await.expect("parent");
    assert_eq!(
        parent
            .parts
            .iter()
            .find(|part| part.part_id == interaction_id)
            .expect("interaction")
            .state,
        PartState::Pending
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
        .submit_user_run(
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

#[tokio::test]
async fn list_exclude_subagents_filters_task_children_only() {
    let db = in_memory_db().await;
    let (engine, parent_id) = setup(db).await;
    let workspace_id = engine
        .session_meta(parent_id)
        .await
        .expect("parent meta")
        .workspace_id;
    // A task child (relation_kind = 'subagent').
    engine
        .create_subagent_session(
            parent_id,
            "task-9".to_owned(),
            "sub task".to_owned(),
            1_000_000,
        )
        .await
        .expect("create subagent");
    // A regular user child must survive the filter.
    engine
        .create_session(NewSession {
            workspace_id,
            parent_id: Some(parent_id),
            relation_kind: SessionRelationKind::Child,
            cutoff_part_id: None,
            title: "user child".to_owned(),
            task_id: None,
            config_json: None,
            provider_anchors_json: None,
        })
        .await
        .expect("create child");

    let all = engine
        .list_session_summaries(SessionListQuery {
            workspace_id: Some(workspace_id),
            parent_id: None,
            roots_only: false,
            exclude_subagents: false,
            search: None,
            limit: None,
            before: None,
        })
        .await
        .expect("list all");
    assert_eq!(all.len(), 3, "without the filter every session is listed");

    let parents_only = engine
        .list_session_summaries(SessionListQuery {
            workspace_id: Some(workspace_id),
            parent_id: None,
            roots_only: false,
            exclude_subagents: true,
            search: None,
            limit: None,
            before: None,
        })
        .await
        .expect("list excluding subagents");
    let titles: Vec<&str> = parents_only
        .iter()
        .map(|summary| summary.title.as_str())
        .collect();
    assert!(
        !titles.iter().any(|title| *title == "sub task"),
        "task child must be hidden: {titles:?}"
    );
    assert_eq!(
        parents_only.len(),
        2,
        "root + user child remain, task child hidden: {titles:?}"
    );
}
