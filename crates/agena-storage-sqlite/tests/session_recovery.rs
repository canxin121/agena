use agena_domain::{AgentStatus, ComposerDocument, ComposerNode, SessionRelationKind};
use agena_storage::kernel::*;
use agena_storage_sqlite::open_kernel_runtime;
use sea_orm::sqlx::Connection;

#[tokio::test]
async fn interrupted_agent_recovery_preserves_queued_compaction_without_resuming_its_task() {
    let directory = tempfile::tempdir().unwrap();
    let source_url = format!(
        "sqlite://{}?mode=rwc",
        directory.path().join("source.db").display()
    );
    let runtime = open_kernel_runtime(&source_url).await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/agent-maintenance-recovery")
        .await
        .unwrap();
    let parent = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Parent".into(),
            Default::default(),
        )
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
            "Agent".into(),
            SessionConfiguration {
                parent_session_id: Some(parent),
                relation_kind: SessionRelationKind::Subagent,
                agent: Some(SessionAgentConfiguration {
                    name: "worker".into(),
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
        .id;
    let current = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            child,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Running delegated work".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = runtime
        .ports
        .executor
        .claim_next(RequestId::new(), child, current.id)
        .await
        .unwrap()
        .value;
    let queued = runtime
        .ports
        .sessions
        .send(
            RequestId::new(),
            child,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Queued delegated followup".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let compact = runtime
        .ports
        .sessions
        .start_control_execution(RequestId::new(), child, ControlExecutionPurpose::Compaction)
        .await
        .unwrap()
        .value;
    let previous_epoch = runtime.ports.reader.epoch();
    let image = directory.path().join("recovery.db");
    sea_orm::sqlx::query("VACUUM INTO ?")
        .bind(image.to_string_lossy().as_ref())
        .execute(runtime.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    assert!(std::fs::metadata(&image).unwrap().len() > 0);
    let copy_url = format!("sqlite://{}?mode=ro", image.display());
    let mut copied = sea_orm::sqlx::SqliteConnection::connect(&copy_url)
        .await
        .unwrap();
    assert_eq!(
        sea_orm::sqlx::query_scalar::<_, i64>("SELECT count(*) FROM agena_sessions")
            .fetch_one(&mut copied)
            .await
            .unwrap(),
        2
    );
    copied.close().await.unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
    drop((attempt, runtime));

    let reopened = open_kernel_runtime(&format!("sqlite://{}?mode=rwc", image.display()))
        .await
        .unwrap();
    assert_ne!(reopened.ports.reader.epoch(), previous_epoch);
    let recovered_attempt = reopened
        .ports
        .reader
        .execution_attempt(child, current.id, current.generation)
        .await
        .unwrap();
    assert_eq!(recovered_attempt.generation, current.generation);
    assert_eq!(recovered_attempt.message_id, current.message_id);
    assert_eq!(recovered_attempt.phase, ExecutionPhase::Interrupted);
    assert_eq!(recovered_attempt.owner_epoch, Some(previous_epoch));
    assert!(
        matches!(
            reopened
                .ports
                .reader
                .execution_attempt(parent, current.id, current.generation)
                .await,
            Err(KernelError::NotFound(_)),
        ),
        "a readable child transcript must not grant the parent's execution ownership"
    );
    drop(recovered_attempt);
    let snapshot = reopened
        .ports
        .reader
        .control_snapshot(child, 128)
        .await
        .unwrap();
    assert_eq!(
        snapshot.configuration.agent.as_ref().unwrap().status,
        AgentStatus::Interrupted
    );
    assert_eq!(snapshot.summary.queued_executions, 1);
    assert_eq!(snapshot.summary.running_executions, 0);
    assert_eq!(snapshot.active_executions.executions.len(), 1);
    assert_eq!(snapshot.active_executions.executions[0].id, compact.id);
    assert_eq!(
        snapshot.active_executions.executions[0].purpose,
        ExecutionPurpose::Compaction
    );
    assert_eq!(
        snapshot.active_executions.executions[0].phase,
        ExecutionPhase::Queued
    );
    drop(snapshot);
    for execution in [current.id, queued.id] {
        assert_eq!(
            reopened
                .ports
                .reader
                .execution(child, execution)
                .await
                .unwrap()
                .phase,
            ExecutionPhase::Interrupted
        );
    }
    // The maintenance attempt remains independently claimable even though
    // task recovery correctly interrupted both delegated message attempts.
    let maintenance = reopened
        .ports
        .executor
        .claim_next(RequestId::new(), child, compact.id)
        .await
        .unwrap()
        .value;
    maintenance
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let settled = reopened
        .ports
        .reader
        .control_snapshot(child, 128)
        .await
        .unwrap();
    assert_eq!(settled.summary.queued_executions, 0);
    assert_eq!(settled.summary.running_executions, 0);
    assert_eq!(
        settled.configuration.agent.as_ref().unwrap().status,
        AgentStatus::Interrupted
    );
    assert!(
        reopened
            .ports
            .reader
            .pending_deliveries(parent, None, 16)
            .await
            .unwrap()
            .is_empty()
    );
    reopened.ports.clone().shutdown().await.unwrap();
}
