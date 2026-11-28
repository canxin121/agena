use agena_domain::{
    AgentStatus, ComposerDocument, ComposerNode, SessionRelationKind, SessionStateKind,
};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};

async fn send(runtime: &SqliteKernel, session: SessionId, text: &str) -> ExecutionView {
    runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text { text: text.into() }]),
        )
        .await
        .unwrap()
        .value
}

async fn seed_agent(runtime: &SqliteKernel, parent: SessionId, request: RequestId) -> SessionId {
    runtime
        .ports
        .sessions
        .create_seeded_agent(
            request,
            parent,
            "Context worker".into(),
            SessionConfiguration {
                parent_session_id: Some(parent),
                relation_kind: SessionRelationKind::Subagent,
                agent: Some(SessionAgentConfiguration {
                    origin_message_id: None,
                    workflow: None,
                    cordis: None,
                    name: "context_worker".into(),
                    status: AgentStatus::Running,
                    started_at_ms: Some(1),
                    finished_at_ms: None,
                    failure: None,
                    limits: Default::default(),
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id
}

#[tokio::test]
async fn seeded_agent_freezes_completed_turns_across_interleaved_queue_entries() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("seed.db").display()
    );
    let runtime = open_kernel_runtime(&url).await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/agent-seed-tests")
        .await
        .unwrap();
    let parent = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Parent".into())
        .await
        .unwrap()
        .value
        .id;
    let first = completed_input(&runtime, parent).await;
    let second = send(&runtime, parent, "Previous completed question").await;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), parent, second.id)
        .await
        .unwrap()
        .value;
    let inflight = send(
        &runtime,
        parent,
        "Future question admitted before the response",
    )
    .await;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let answer = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    answer.append("Frozen parent answer").await.unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let running = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), parent, inflight.id)
        .await
        .unwrap()
        .value;
    let pending = send(&runtime, parent, "Future queued question").await;
    let cancelled = send(&runtime, parent, "Never showed this to the model").await;
    runtime
        .ports
        .sessions
        .cancel_queued(RequestId::new(), parent, cancelled.id)
        .await
        .unwrap();
    let current_round = running
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let partial = current_round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    partial.append("Unfinished answer").await.unwrap();

    let request = RequestId::new();
    let child = seed_agent(&runtime, parent, request).await;
    let child_state = runtime
        .ports
        .reader
        .control_snapshot(child, 1)
        .await
        .unwrap();
    assert_eq!(child_state.configuration.parent_session_id, Some(parent));
    assert_eq!(
        child_state.configuration.relation_kind,
        SessionRelationKind::Subagent
    );
    assert_eq!(child_state.summary.queued_executions, 0);
    assert_eq!(child_state.summary.running_executions, 0);
    let own = send(&runtime, child, "Child instruction").await;
    let child_attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), child, own.id)
        .await
        .unwrap()
        .value;
    let child_round = child_attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let expected = [
        first.message_id,
        second.message_id,
        answer.entry_id(),
        own.message_id,
    ];
    let prompt = child_round.prompt().await.unwrap();
    assert_eq!(
        prompt
            .entries
            .iter()
            .map(|entry| entry.entry.id)
            .collect::<Vec<_>>(),
        expected
    );
    drop(prompt);

    current_round
        .finish(RequestId::new(), None, true)
        .await
        .unwrap();
    running
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    runtime
        .ports
        .sessions
        .cancel_queued(RequestId::new(), parent, pending.id)
        .await
        .unwrap();
    runtime
        .ports
        .sessions
        .clear(RequestId::new(), parent, ClearMode::History)
        .await
        .unwrap();
    assert_eq!(seed_agent(&runtime, parent, request).await, child);
    child_round
        .finish(RequestId::new(), None, true)
        .await
        .unwrap();
    child_attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();

    let content = answer.id();
    runtime.ports.clone().shutdown().await.unwrap();
    drop((
        runtime,
        attempt,
        round,
        answer,
        running,
        current_round,
        partial,
        child_attempt,
        child_round,
    ));
    let restored = open_kernel_runtime(&url).await.unwrap();
    let history = restored
        .ports
        .reader
        .history(child, None, 16)
        .await
        .unwrap();
    assert_eq!(
        history
            .entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        expected
    );
    let text = restored
        .ports
        .reader
        .text_range(child, content, None, 1024)
        .await
        .unwrap();
    assert_eq!(text.bytes, b"Frozen parent answer");
    assert!(
        restored
            .ports
            .reader
            .execution_text_sources(child, own.id, Some(own.generation))
            .await
            .unwrap()
            .is_empty()
    );
    restored.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn seeded_agent_pins_a_completed_retry_generation_and_its_prompt_checkpoint() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let parent = runtime
        .ports
        .sessions
        .create_global(RequestId::new(), "Parent".into(), Default::default())
        .await
        .unwrap()
        .value
        .id;
    let concealed = completed_input(&runtime, parent).await;
    runtime
        .ports
        .sessions
        .clear(RequestId::new(), parent, ClearMode::Context)
        .await
        .unwrap();
    let retained = send(&runtime, parent, "Question after context reset").await;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), parent, retained.id)
        .await
        .unwrap()
        .value;
    let compact = attempt
        .begin_compaction(RequestId::new(), 8)
        .await
        .unwrap()
        .value;
    let summary = compact
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    summary.append("Compacted retained context").await.unwrap();
    let checkpoint = compact
        .commit_compaction(RequestId::new(), summary.id(), retained.message_id, None)
        .await
        .unwrap()
        .value;
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    runtime
        .ports
        .sessions
        .retry(RequestId::new(), parent, retained.id)
        .await
        .unwrap();

    let child = seed_agent(&runtime, parent, RequestId::new()).await;
    let version = runtime
        .ports
        .reader
        .entry(child, retained.message_id)
        .await
        .unwrap();
    assert_eq!(version.execution_phase, ExecutionPhase::Succeeded);
    assert_eq!(version.execution_generation, retained.generation);
    assert!(
        runtime
            .ports
            .reader
            .execution_text_sources(child, retained.id, Some(retained.generation))
            .await
            .unwrap()
            .is_empty(),
        "a prompt summary is not an assistant answer"
    );
    assert!(matches!(
        runtime
            .ports
            .reader
            .entry(child, concealed.message_id)
            .await,
        Err(KernelError::NotFound(_))
    ));
    let input = send(&runtime, child, "Child instruction").await;
    let own = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), child, input.id)
        .await
        .unwrap()
        .value;
    let round = own
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let prompt = round.prompt().await.unwrap();
    assert_eq!(
        prompt
            .entries
            .iter()
            .map(|item| item.entry.id)
            .collect::<Vec<_>>(),
        [checkpoint, input.message_id]
    );
    round.finish(RequestId::new(), None, true).await.unwrap();
    own.finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
}

async fn pause(runtime: &SqliteKernel, session: SessionId) {
    runtime
        .ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                message_queue_paused: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

async fn assert_new_branch_runs(runtime: &SqliteKernel, session: SessionId) {
    let snapshot = runtime
        .ports
        .reader
        .control_snapshot(session, 8)
        .await
        .unwrap();
    assert!(!snapshot.configuration.message_queue_paused);
    assert_eq!(snapshot.summary.state, SessionStateKind::Ready);
    assert_eq!(snapshot.summary.queued_executions, 0);
    assert_eq!(snapshot.summary.running_executions, 0);
    let execution = send(runtime, session, "Run in this branch").await;
    runtime
        .ports
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap();
    runtime
        .ports
        .sessions
        .cancel(RequestId::new(), session, execution.id)
        .await
        .unwrap();
}

#[tokio::test]
async fn forks_nested_forks_and_rewinds_start_with_an_independent_unpaused_queue() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime.workspaces.ensure_id("/branch-tests").await.unwrap();
    let parent = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Parent".into())
        .await
        .unwrap()
        .value
        .id;
    let source = send(&runtime, parent, "Inherited context").await;
    runtime
        .ports
        .sessions
        .cancel(RequestId::new(), parent, source.id)
        .await
        .unwrap();
    pause(&runtime, parent).await;

    let fork = runtime
        .ports
        .sessions
        .fork(RequestId::new(), parent, source.message_id)
        .await
        .unwrap()
        .value
        .id;
    assert_new_branch_runs(&runtime, fork).await;
    pause(&runtime, fork).await;
    let nested = runtime
        .ports
        .sessions
        .fork(RequestId::new(), fork, source.message_id)
        .await
        .unwrap()
        .value
        .id;
    let rewind = runtime
        .ports
        .sessions
        .rewind(RequestId::new(), parent, source.message_id)
        .await
        .unwrap()
        .value
        .session
        .id;
    for (session, expected_parent, relation, depth) in [
        (nested, fork, SessionRelationKind::Fork, 2),
        (rewind, parent, SessionRelationKind::Rewind, 1),
    ] {
        assert_new_branch_runs(&runtime, session).await;
        let snapshot = runtime
            .ports
            .reader
            .control_snapshot(session, 8)
            .await
            .unwrap();
        assert_eq!(
            snapshot.configuration.parent_session_id,
            Some(expected_parent)
        );
        assert_eq!(snapshot.summary.root_session_id, parent);
        assert_eq!(snapshot.summary.depth, depth);
        assert_eq!(snapshot.configuration.relation_kind, relation);
    }
    for session in [parent, fork] {
        assert!(
            runtime
                .ports
                .reader
                .configuration(session)
                .await
                .unwrap()
                .message_queue_paused
        );
    }
    runtime.ports.clone().shutdown().await.unwrap();
}

async fn completed_input(runtime: &SqliteKernel, session: SessionId) -> ExecutionView {
    let input = send(runtime, session, "Context inherited by the branch").await;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), session, input.id)
        .await
        .unwrap()
        .value;
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap()
        .value
}

#[tokio::test]
async fn inherited_context_control_operations_own_their_queue_and_attempts() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/control-tests")
        .await
        .unwrap();
    let parent = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Parent".into())
        .await
        .unwrap()
        .value
        .id;
    let source = completed_input(&runtime, parent).await;
    let fork = runtime
        .ports
        .sessions
        .fork(RequestId::new(), parent, source.message_id)
        .await
        .unwrap()
        .value
        .id;
    let branch = runtime
        .ports
        .sessions
        .fork(RequestId::new(), fork, source.message_id)
        .await
        .unwrap()
        .value
        .id;
    let request = RequestId::new();
    let compact = runtime
        .ports
        .sessions
        .start_control_execution(request, branch, ControlExecutionPurpose::Compaction)
        .await
        .unwrap();
    let replay = runtime
        .ports
        .sessions
        .start_control_execution(request, branch, ControlExecutionPurpose::Compaction)
        .await
        .unwrap();
    assert_eq!(compact.position, replay.position);
    assert_eq!(compact.value, replay.value);
    let compact = compact.value;
    assert_eq!(compact.purpose, ExecutionPurpose::Compaction);
    assert_eq!(compact.session_id, branch);
    assert_ne!(compact.id, source.id);
    assert!(compact.message_id > source.message_id);
    let continuation = runtime
        .ports
        .sessions
        .continue_execution(RequestId::new(), branch, source.message_id)
        .await
        .unwrap()
        .value;
    assert_eq!(continuation.purpose, ExecutionPurpose::Continuation);
    assert_ne!(continuation.id, source.id);
    assert_ne!(continuation.id, compact.id);
    assert!(continuation.message_id > compact.message_id);
    assert!(matches!(
        runtime
            .ports
            .executor
            .claim_next(RequestId::new(), branch, continuation.id)
            .await,
        Err(KernelError::QueueOutOfOrder)
    ));
    let compact_attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), branch, compact.id)
        .await
        .unwrap()
        .value;
    let compact_round = compact_attempt
        .begin_compaction(RequestId::new(), 8)
        .await
        .unwrap()
        .value;
    let prompt = compact_round.prompt().await.unwrap();
    assert!(
        prompt
            .entries
            .iter()
            .any(|entry| entry.entry.id == source.message_id)
    );
    assert!(
        !prompt
            .entries
            .iter()
            .any(|entry| entry.entry.id == compact.message_id)
    );
    assert!(
        !prompt
            .entries
            .iter()
            .any(|entry| entry.entry.id == continuation.message_id)
    );
    drop(prompt);
    compact_round
        .finish(RequestId::new(), None, false)
        .await
        .unwrap();
    compact_attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let continuation_attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), branch, continuation.id)
        .await
        .unwrap()
        .value;
    let round = continuation_attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let prompt = round.prompt().await.unwrap();
    assert!(
        prompt
            .entries
            .iter()
            .any(|entry| entry.entry.id == continuation.message_id)
    );
    drop(prompt);
    round.finish(RequestId::new(), None, true).await.unwrap();
    continuation_attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    assert!(matches!(
        runtime
            .ports
            .sessions
            .cancel(RequestId::new(), parent, continuation.id)
            .await,
        Err(KernelError::Ownership)
    ));
    assert_eq!(
        *runtime
            .ports
            .reader
            .execution(parent, source.id)
            .await
            .unwrap(),
        source
    );
    let history = runtime
        .ports
        .reader
        .history(branch, None, 32)
        .await
        .unwrap();
    assert_eq!(
        history
            .entries
            .iter()
            .filter(|entry| matches!(entry.payload, EntryPayload::User { .. }))
            .count(),
        1
    );
    assert_eq!(
        runtime
            .ports
            .reader
            .summary(branch)
            .await
            .unwrap()
            .user_message_count,
        1
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn queued_compaction_purpose_survives_reopen_and_cannot_be_edited_as_user_input() {
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("controls.db").display()
    );
    let runtime = open_kernel_runtime(&url).await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/persisted-controls")
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Control".into())
        .await
        .unwrap()
        .value
        .id;
    let source = completed_input(&runtime, session).await;
    pause(&runtime, session).await;
    let compact = runtime
        .ports
        .sessions
        .start_control_execution(
            RequestId::new(),
            session,
            ControlExecutionPurpose::Compaction,
        )
        .await
        .unwrap()
        .value;
    assert!(matches!(
        runtime
            .ports
            .executor
            .claim_next(RequestId::new(), session, compact.id)
            .await,
        Err(KernelError::QueuePaused)
    ));
    let anchor = runtime
        .ports
        .reader
        .entry(session, compact.message_id)
        .await
        .unwrap();
    assert!(matches!(anchor.payload, EntryPayload::Notice { .. }));
    assert_eq!(anchor.id, anchor.message_id);
    assert!(matches!(
        runtime
            .ports
            .sessions
            .edit_queued_with_blobs(
                RequestId::new(),
                session,
                compact.id,
                anchor.revision,
                ComposerDocument(vec![ComposerNode::Text {
                    text: "Must not replace control state".into()
                }]),
                &[]
            )
            .await,
        Err(KernelError::Invalid(_))
    ));
    drop(anchor);
    runtime.ports.clone().shutdown().await.unwrap();
    drop(runtime);

    let runtime = open_kernel_runtime(&url).await.unwrap();
    let snapshot = runtime
        .ports
        .reader
        .control_snapshot(session, 8)
        .await
        .unwrap();
    assert!(snapshot.configuration.message_queue_paused);
    assert_eq!(snapshot.active_executions.executions.len(), 1);
    assert_eq!(snapshot.active_executions.executions[0], compact);
    assert_eq!(snapshot.summary.user_message_count, 1);
    drop(snapshot);
    runtime
        .ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                message_queue_paused: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), session, compact.id)
        .await
        .unwrap()
        .value;
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    // A caller still holding the original user input must also advance past
    // compact's newer control anchor, rather than retrying the old boundary.
    let resumed = runtime
        .ports
        .sessions
        .continue_execution(RequestId::new(), session, source.message_id)
        .await
        .unwrap()
        .value;
    assert_eq!(resumed.purpose, ExecutionPurpose::Continuation);
    assert_ne!(resumed.id, compact.id);
    assert_eq!(
        runtime
            .ports
            .reader
            .execution(session, compact.id)
            .await
            .unwrap()
            .generation,
        AttemptGeneration::INITIAL
    );
    assert_eq!(
        *runtime
            .ports
            .reader
            .execution(session, source.id)
            .await
            .unwrap(),
        source
    );
    runtime
        .ports
        .sessions
        .cancel(RequestId::new(), session, resumed.id)
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn an_existing_family_cannot_be_reparented_into_a_cycle_or_stale_ancestry() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime.workspaces.ensure_id("/family-tests").await.unwrap();
    let root = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Root".into())
        .await
        .unwrap()
        .value
        .id;
    let child = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Child".into(),
            SessionConfiguration {
                parent_session_id: Some(root),
                relation_kind: SessionRelationKind::Child,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let other = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Other".into())
        .await
        .unwrap()
        .value
        .id;
    for parent in [child, other] {
        assert!(matches!(
            runtime
                .ports
                .sessions
                .configure(
                    RequestId::new(),
                    root,
                    SessionConfigurationChange {
                        parent_session_id: Some(parent),
                        relation_kind: Some(SessionRelationKind::Child),
                        ..Default::default()
                    }
                )
                .await,
            Err(KernelError::Conflict(_))
        ));
    }
    assert_eq!(
        runtime
            .ports
            .reader
            .configuration(root)
            .await
            .unwrap()
            .parent_session_id,
        None
    );
    let summary = runtime.ports.reader.summary(child).await.unwrap();
    assert_eq!(summary.root_session_id, root);
    assert_eq!(summary.depth, 1);
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn archive_publication_releases_failed_commit_and_respects_allocated_segment_ids() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/archive-publication")
        .await
        .unwrap();
    let source = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Source".into())
        .await
        .unwrap()
        .value
        .id;
    send(&runtime, source, "Keep this entry").await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("publication.jsonl");
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    runtime
        .ports
        .reader
        .export_jsonl(source, &mut file)
        .await
        .unwrap();
    drop(file);

    let db = runtime.database.get_sqlite_connection_pool();
    let collected: i64 = sea_orm::sqlx::query_scalar(
        "INSERT INTO agena_history_segments DEFAULT VALUES RETURNING id",
    )
    .fetch_one(db)
    .await
    .unwrap();
    sea_orm::sqlx::query("DELETE FROM agena_history_segments WHERE id=?")
        .bind(collected)
        .execute(db)
        .await
        .unwrap();

    // The staging graph is valid. Reject only the final publication commit.
    sea_orm::sqlx::query(
        "CREATE TRIGGER reject_publication AFTER INSERT ON agena_sessions
         BEGIN
             INSERT INTO agena_executions(id,session_id,message_id,current_generation,created_at_ms)
             VALUES('failed-publication',NEW.id,-1,1,0);
         END",
    )
    .execute(db)
    .await
    .unwrap();
    let request = RequestId::new();
    let error = runtime
        .ports
        .sessions
        .import_archive(request, workspace, &path)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        KernelError::Invalid("archive publication has unresolved references".into())
    );
    let published: i64 = sea_orm::sqlx::query_scalar("SELECT count(*) FROM agena_sessions")
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(published, 1);
    sea_orm::sqlx::query("DROP TRIGGER reject_publication")
        .execute(db)
        .await
        .unwrap();
    let receipt = runtime
        .ports
        .sessions
        .import_archive(request, workspace, &path)
        .await
        .unwrap();
    let segment: i64 = sea_orm::sqlx::query_scalar(
        "SELECT segment_id FROM agena_session_segments WHERE session_id=?",
    )
    .bind(receipt.value.id.get())
    .fetch_one(db)
    .await
    .unwrap();
    assert!(segment > collected);
    assert_eq!(
        runtime
            .ports
            .sessions
            .import_archive(request, workspace, &path)
            .await
            .unwrap(),
        receipt
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn archives_restore_control_purpose_and_the_original_inherited_content_cut() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("source.db");
    let runtime = open_kernel_runtime(&format!("sqlite://{}?mode=rwc", database.display()))
        .await
        .unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/archive-branches")
        .await
        .unwrap();
    let parent = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Source".into())
        .await
        .unwrap()
        .value
        .id;
    let input = send(&runtime, parent, "A streaming answer").await;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), parent, input.id)
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
    output.append("before-fork").await.unwrap();
    let branch = runtime
        .ports
        .sessions
        .fork(RequestId::new(), parent, output.entry_id())
        .await
        .unwrap()
        .value
        .id;
    output.append("-after-fork").await.unwrap();
    // A second fork checkpoints the source's later bytes while capture stays
    // open; the first branch must still retain its earlier content cut.
    runtime
        .ports
        .sessions
        .fork(RequestId::new(), parent, output.entry_id())
        .await
        .unwrap();
    let before = runtime
        .ports
        .reader
        .text_range(branch, output.id(), None, 1024)
        .await
        .unwrap();
    assert_eq!(before.bytes, b"before-fork");
    let frozen_epoch = before.stream.epoch;
    drop(before);
    let compact = runtime
        .ports
        .sessions
        .start_control_execution(
            RequestId::new(),
            branch,
            ControlExecutionPurpose::Compaction,
        )
        .await
        .unwrap()
        .value;
    let compact_attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), branch, compact.id)
        .await
        .unwrap()
        .value;
    let compact_round = compact_attempt
        .begin_compaction(RequestId::new(), 8)
        .await
        .unwrap()
        .value;
    compact_round
        .finish(RequestId::new(), None, false)
        .await
        .unwrap();
    compact_attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let image = directory.path().join("recovery.db");
    let source_sessions: i64 = sea_orm::sqlx::query_scalar("SELECT COUNT(*) FROM agena_sessions")
        .fetch_one(runtime.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    assert!(source_sessions >= 3);
    sea_orm::sqlx::query("VACUUM INTO ?")
        .bind(image.to_string_lossy().as_ref())
        .execute(runtime.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    assert!(std::fs::metadata(&image).unwrap().len() >= 4096);
    let source_content = output.id();
    runtime.ports.clone().shutdown().await.unwrap();
    drop((
        output,
        round,
        attempt,
        compact_round,
        compact_attempt,
        runtime,
    ));
    let runtime = open_kernel_runtime(&format!("sqlite://{}?mode=rwc", image.display()))
        .await
        .unwrap();
    assert_eq!(
        sea_orm::sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM agena_sessions")
            .fetch_one(runtime.database.get_sqlite_connection_pool())
            .await
            .unwrap(),
        source_sessions
    );
    assert_ne!(
        runtime
            .ports
            .reader
            .text_range(parent, source_content, None, 1024)
            .await
            .unwrap()
            .stream
            .epoch,
        frozen_epoch
    );
    assert_eq!(
        runtime
            .ports
            .reader
            .text_range(branch, source_content, None, 1024)
            .await
            .unwrap()
            .bytes,
        b"before-fork"
    );
    let path = directory.path().join("branch.jsonl");
    let mut target = tokio::fs::File::create(&path).await.unwrap();
    runtime
        .ports
        .reader
        .export_jsonl(branch, &mut target)
        .await
        .unwrap();
    target.sync_all().await.unwrap();
    drop(target);
    let restored = runtime
        .ports
        .sessions
        .import_archive(RequestId::new(), workspace, &path)
        .await
        .unwrap()
        .value
        .id;
    let history = runtime
        .ports
        .reader
        .history(restored, None, 32)
        .await
        .unwrap();
    let content = history
        .entries
        .iter()
        .find_map(|entry| match entry.payload {
            EntryPayload::Text { content_id, .. } => Some(content_id),
            _ => None,
        })
        .unwrap();
    let anchor = history
        .entries
        .iter()
        .find(|entry| {
            matches!(entry.payload, EntryPayload::Notice { .. }) && entry.id == entry.message_id
        })
        .unwrap();
    let restored_control = runtime
        .ports
        .reader
        .execution(restored, anchor.execution_id)
        .await
        .unwrap();
    assert_eq!(restored_control.purpose, ExecutionPurpose::Compaction);
    assert_ne!(restored_control.id, compact.id);
    let restored_rounds = runtime
        .ports
        .reader
        .rounds(restored, None, 32)
        .await
        .unwrap();
    let archived_round = restored_rounds
        .iter()
        .find(|round| round.execution_id == restored_control.id)
        .unwrap();
    let archived_prompt = runtime
        .ports
        .reader
        .prompt(restored, archived_round.id)
        .await
        .unwrap();
    let counted: bool = sea_orm::sqlx::query_scalar(
        "SELECT (input_source_count,input_source_bytes)=(
             SELECT count(*),sum(head_bytes+256) FROM agena_round_input_sources WHERE round_id=r.id
         ) FROM agena_rounds r WHERE r.id=?",
    )
    .bind(archived_round.id.get())
    .fetch_one(runtime.database.get_sqlite_connection_pool())
    .await
    .unwrap();
    assert!(counted);
    let archived_source = archived_prompt
        .sources
        .iter()
        .find(|head| head.id == content)
        .unwrap();
    assert_eq!(archived_source.committed, b"before-fork".len() as u64);
    assert_ne!(archived_source.epoch, frozen_epoch);
    assert_eq!(archived_source.state, CaptureState::Open);
    drop((archived_prompt, restored_rounds));
    let restored_anchor = anchor.id;
    drop(restored_control);
    drop(history);
    assert_eq!(
        runtime
            .ports
            .reader
            .text_range(restored, content, None, 1024)
            .await
            .unwrap()
            .bytes,
        b"before-fork"
    );
    let continued = runtime
        .ports
        .sessions
        .continue_execution(RequestId::new(), restored, restored_anchor)
        .await
        .unwrap()
        .value;
    assert_eq!(continued.purpose, ExecutionPurpose::Continuation);
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), restored, continued.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let prompt = round.prompt().await.unwrap();
    let frozen = prompt
        .sources
        .iter()
        .find(|head| head.id == content)
        .unwrap();
    assert_eq!(
        runtime
            .ports
            .reader
            .text_range_at(restored, frozen, None, 1024)
            .await
            .unwrap()
            .bytes,
        b"before-fork"
    );
    drop(prompt);
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn in_place_truncation_keeps_one_session_and_drops_the_retracted_tail() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/truncate-tests")
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Session".into())
        .await
        .unwrap()
        .value
        .id;
    let first = completed_input(&runtime, session).await;
    let second = completed_input(&runtime, session).await;

    let result = runtime
        .ports
        .sessions
        .truncate(RequestId::new(), session, second.message_id)
        .await
        .unwrap()
        .value;
    // No branch is created; the same session keeps its history before the cut.
    assert_eq!(result.session.id, session);
    assert_eq!(result.document.text(), "Context inherited by the branch");

    let history = runtime
        .ports
        .reader
        .history(session, None, 32)
        .await
        .unwrap();
    let ids = history
        .entries
        .iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    assert!(ids.contains(&first.message_id));
    assert!(!ids.contains(&second.message_id));
    assert_eq!(
        runtime
            .ports
            .reader
            .summary(session)
            .await
            .unwrap()
            .user_message_count,
        1
    );
    // The same session can still accept a new turn after the cut.
    let replacement = completed_input(&runtime, session).await;
    assert_ne!(replacement.message_id, second.message_id);
    let fork = runtime
        .ports
        .sessions
        .fork(RequestId::new(), session, replacement.message_id)
        .await
        .unwrap()
        .value
        .id;
    let summary = runtime.ports.reader.summary(fork).await.unwrap();
    assert_eq!((summary.entry_count, summary.user_message_count), (2, 2));
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn archive_and_history_cuts_keep_exact_counts_and_timestamp_maxima() {
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/history-summary")
        .await
        .unwrap();
    let source = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Summary source".into())
        .await
        .unwrap()
        .value
        .id;
    let first = send(&runtime, source, "First").await;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), source, first.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let response = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    response.append("An answer").await.unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    completed_input(&runtime, source).await;
    completed_input(&runtime, source).await;

    // Restore valid facts whose clocks do not follow membership order. Import
    // uses the newest user timestamp; cuts use the newest retained entry.
    let mut original = Vec::new();
    let mut report = runtime
        .ports
        .reader
        .export_jsonl(source, &mut original)
        .await
        .unwrap();
    let latest_entry = 8_000_000_000_000i64;
    let mut times = [20, latest_entry, -100, 10].into_iter();
    let mut archive = Vec::new();
    for line in original
        .split_inclusive(|byte| *byte == b'\n')
        .take(report.records as usize)
    {
        let mut record: Value = serde_json::from_slice(line).unwrap();
        if record["kind"] == "entry" {
            let timestamp = times.next().unwrap();
            record["data"]["facts"]["created_at_ms"] = json!(timestamp);
            record["data"]["facts"]["entry"]["created_at_ms"] = json!(timestamp);
        }
        serde_json::to_writer(&mut archive, &record).unwrap();
        archive.push(b'\n');
    }
    assert!(times.next().is_none());
    report.bytes = archive.len() as u64;
    report.sha256 = Sha256::digest(&archive).into();
    serde_json::to_writer(&mut archive, &json!({"kind":"footer","data":report})).unwrap();
    archive.push(b'\n');
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("summary.jsonl");
    tokio::fs::write(&path, archive).await.unwrap();
    let restored = runtime
        .ports
        .sessions
        .import_archive(RequestId::new(), workspace, &path)
        .await
        .unwrap()
        .value
        .id;
    let summary = runtime.ports.reader.summary(restored).await.unwrap();
    assert_eq!(
        (
            summary.entry_count,
            summary.user_message_count,
            summary.last_message_at_ms
        ),
        (4, 3, Some(20))
    );

    let history = runtime
        .ports
        .reader
        .history(restored, None, 8)
        .await
        .unwrap();
    let at_time = |timestamp| {
        history
            .entries
            .iter()
            .find(|entry| entry.created_at_ms == timestamp)
            .unwrap()
            .id
    };
    let branch = runtime
        .ports
        .sessions
        .fork(RequestId::new(), restored, at_time(-100))
        .await
        .unwrap()
        .value
        .id;
    runtime
        .ports
        .sessions
        .truncate(RequestId::new(), restored, at_time(10))
        .await
        .unwrap();
    drop(history);
    for session in [restored, branch] {
        let summary = runtime.ports.reader.summary(session).await.unwrap();
        assert_eq!(
            (
                summary.entry_count,
                summary.user_message_count,
                summary.last_message_at_ms
            ),
            (3, 2, Some(latest_entry))
        );
    }

    // The branch still owns the old segment. Appending to the truncated
    // session copies its visible prefix and extends that prefix's summary.
    let next = completed_input(&runtime, restored).await;
    let extended = runtime
        .ports
        .sessions
        .fork(RequestId::new(), restored, next.message_id)
        .await
        .unwrap()
        .value
        .id;
    let summary = runtime.ports.reader.summary(extended).await.unwrap();
    assert_eq!(
        (
            summary.entry_count,
            summary.user_message_count,
            summary.last_message_at_ms
        ),
        (4, 3, Some(latest_entry))
    );
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn truncated_rounds_are_no_longer_selected_as_the_latest_provider_round() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/truncate-rounds")
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create(RequestId::new(), workspace, "Rounds".into())
        .await
        .unwrap()
        .value
        .id;
    async fn round(
        runtime: &SqliteKernel,
        session: SessionId,
        text: &str,
    ) -> (ExecutionView, RoundId) {
        let input = send(runtime, session, text).await;
        let attempt = runtime
            .ports
            .executor
            .claim_next(RequestId::new(), session, input.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let id = round.id();
        round
            .finish(
                RequestId::new(),
                Some(serde_json::json!({"usage": {"input_tokens": 1, "output_tokens": 1}})),
                true,
            )
            .await
            .unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        (input, id)
    }
    let (first, first_round) = round(&runtime, session, "first").await;
    let (second, second_round) = round(&runtime, session, "second").await;
    let latest = runtime
        .ports
        .reader
        .latest_round(session)
        .await
        .unwrap()
        .clone();
    assert_eq!(latest.as_ref().map(|round| round.id), Some(second_round));
    assert_eq!(
        runtime
            .ports
            .reader
            .latest_usage_round(session)
            .await
            .unwrap()
            .as_ref()
            .map(|round| round.id),
        Some(second_round)
    );

    runtime
        .ports
        .sessions
        .truncate(RequestId::new(), session, second.message_id)
        .await
        .unwrap();

    // The retracted round left this session's membership; selecting it would
    // make the projection return an internal error.
    let latest = runtime
        .ports
        .reader
        .latest_round(session)
        .await
        .unwrap()
        .clone();
    assert_eq!(latest.as_ref().map(|round| round.id), Some(first_round));
    assert_eq!(
        runtime
            .ports
            .reader
            .latest_usage_round(session)
            .await
            .unwrap()
            .as_ref()
            .map(|round| round.id),
        Some(first_round)
    );

    // Truncating the whole history must not surface a removed round either.
    runtime
        .ports
        .sessions
        .truncate(RequestId::new(), session, first.message_id)
        .await
        .unwrap();
    assert!(
        runtime
            .ports
            .reader
            .latest_round(session)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        runtime
            .ports
            .reader
            .latest_usage_round(session)
            .await
            .unwrap()
            .is_none()
    );
    runtime.ports.clone().shutdown().await.unwrap();
}
