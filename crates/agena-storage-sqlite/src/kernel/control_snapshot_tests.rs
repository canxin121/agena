use super::{tests::document, *};

fn assert_phase(snapshot: &SessionControlSnapshot, expected: agena_domain::ExecutionPhase) {
    assert!(matches!(
        snapshot.active_execution,
        Some(agena_domain::ExecutionLifecycle::Active { phase, .. }) if phase == expected
    ));
}

#[tokio::test]
async fn control_snapshot_is_shared_on_warm_reads_and_tracks_durable_execution_phases() {
    for memory in [true, false] {
        let diagnostics = KernelDiagnostics::default();
        let (ports, _admin, _root, _url) =
            tests::fixture_with_diagnostics(memory, diagnostics.clone()).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "coherent status".into())
            .await
            .unwrap()
            .value
            .id;
        let execution = ports
            .sessions
            .send(RequestId::new(), session, document("work"))
            .await
            .unwrap()
            .value;
        let cold = ports.reader.control_snapshot(session, 128).await.unwrap();
        assert_phase(&cold, agena_domain::ExecutionPhase::Starting);
        assert_eq!(cold.metadata.title, cold.summary.title);
        assert_eq!(
            cold.metadata.metadata_version,
            cold.summary.metadata_version
        );
        assert_eq!(cold.active_executions.executions[0].id, execution.id);
        let before_warm = diagnostics.snapshot();
        let budget = diagnostics.read_budget();
        let warm = ports.reader.control_snapshot(session, 128).await.unwrap();
        assert!(cold.ptr_eq(&warm));
        assert_eq!(diagnostics.read_budget().bytes, budget.bytes);
        assert_eq!(diagnostics.read_budget().allocations, budget.allocations);
        let after_warm = diagnostics.snapshot();
        assert_eq!(after_warm.metadata_queries, before_warm.metadata_queries);
        assert_eq!(after_warm.summary_queries, before_warm.summary_queries);
        assert_eq!(after_warm.activity_queries, before_warm.activity_queries);
        assert_eq!(
            after_warm.read_transactions_started,
            before_warm.read_transactions_started
        );

        let attempt = ports
            .executor
            .claim(RequestId::new(), session, execution.id)
            .await
            .unwrap()
            .value;
        let preparing = ports.reader.control_snapshot(session, 128).await.unwrap();
        assert!(!cold.ptr_eq(&preparing));
        assert!(preparing.control_version > cold.control_version);
        assert_phase(&preparing, agena_domain::ExecutionPhase::PreparingModel);
        let round = attempt
            .begin_round(RequestId::new(), vec![execution.message_id])
            .await
            .unwrap()
            .value;
        let output = round
            .begin_text(RequestId::new(), true)
            .await
            .unwrap()
            .value;
        output
            .append(&"model content ".repeat(16_384))
            .await
            .unwrap();
        let before_status = diagnostics.snapshot();
        let streaming = ports.reader.control_snapshot(session, 128).await.unwrap();
        assert_phase(&streaming, agena_domain::ExecutionPhase::StreamingModel);
        assert_eq!(
            diagnostics.snapshot().history_queries,
            before_status.history_queries
        );
        assert_eq!(
            diagnostics.snapshot().detail_queries,
            before_status.detail_queries
        );
        assert_eq!(
            diagnostics.snapshot().history_entry_body_bytes,
            before_status.history_entry_body_bytes
        );
        output.finish(RequestId::new()).await.unwrap();
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "status-tool".into(),
                    name: "fixture".into(),
                    input: serde_json::json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        let executing = ports.reader.control_snapshot(session, 128).await.unwrap();
        assert_phase(&executing, agena_domain::ExecutionPhase::ExecutingTools);
        let interaction = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: "status permission".into(),
                    description: "await one durable reply".into(),
                },
            )
            .await
            .unwrap()
            .value;
        let waiting = ports.reader.control_snapshot(session, 128).await.unwrap();
        assert_phase(&waiting, agena_domain::ExecutionPhase::AwaitingInteraction);
        assert_eq!(waiting.pending_interactions.interactions[0].id, interaction);
        ports
            .sessions
            .reply(
                RequestId::new(),
                session,
                interaction,
                tests::permission_decision(interaction, true, None),
                None,
            )
            .await
            .unwrap();
        let resumed = ports.reader.control_snapshot(session, 128).await.unwrap();
        assert!(resumed.pending_interactions.interactions.is_empty());
        assert_phase(&resumed, agena_domain::ExecutionPhase::ExecutingTools);
        ports
            .sessions
            .cancel(RequestId::new(), session, execution.id)
            .await
            .unwrap();
        let terminal = ports.reader.control_snapshot(session, 128).await.unwrap();
        assert!(terminal.active_execution.is_none());
        assert!(terminal.active_executions.executions.is_empty());
        assert!(terminal.control_version > waiting.control_version);
        let end = diagnostics.snapshot();
        assert_eq!(
            end.read_transactions_started,
            end.read_transactions_completed
        );
        assert_eq!(end.read_transactions_abandoned, 0);
        assert_eq!(end.read_transaction_active_scopes, 0);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn control_snapshot_prefers_running_execution_outside_the_first_active_page() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = tests::fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "running before queued".into())
            .await
            .unwrap()
            .value
            .id;
        let first = ports
            .sessions
            .send(RequestId::new(), session, document("first queued message"))
            .await
            .unwrap()
            .value;
        let second = ports
            .sessions
            .send(RequestId::new(), session, document("second queued message"))
            .await
            .unwrap()
            .value;
        let (queued, running) = if first.id.to_string() < second.id.to_string() {
            (first, second)
        } else {
            (second, first)
        };
        let _attempt = ports
            .executor
            .claim(RequestId::new(), session, running.id)
            .await
            .unwrap()
            .value;
        let current = ports.reader.control_snapshot(session, 1).await.unwrap();
        assert_eq!(current.active_executions.executions.len(), 1);
        assert_eq!(current.active_executions.executions[0].id, queued.id);
        assert_eq!(
            current.active_executions.executions[0].phase,
            ExecutionPhase::Queued
        );
        assert!(current.active_executions.has_more);
        assert_eq!(current.active_executions.next.unwrap().after, queued.id);
        assert_eq!(
            current.active_execution,
            Some(agena_domain::ExecutionLifecycle::Active {
                execution_id: running.id,
                phase: agena_domain::ExecutionPhase::PreparingModel,
            })
        );
        ports
            .sessions
            .cancel(RequestId::new(), session, running.id)
            .await
            .unwrap();
        let remaining = ports.reader.control_snapshot(session, 1).await.unwrap();
        assert_eq!(remaining.active_executions.executions.len(), 1);
        assert_eq!(remaining.active_executions.executions[0].id, queued.id);
        assert!(!remaining.active_executions.has_more);
        assert_eq!(
            remaining.active_execution,
            Some(agena_domain::ExecutionLifecycle::Active {
                execution_id: queued.id,
                phase: agena_domain::ExecutionPhase::Starting,
            })
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn control_snapshot_reads_one_sqlite_version_across_permission_completion_and_metadata_edits()
{
    let root = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        root.path().join("control.db").display()
    );
    let database = KernelDatabase::open(&url).await.unwrap();
    let reader = database.reader.get_sqlite_connection_pool().clone();
    let admin = database.writer.clone();
    let diagnostics = KernelDiagnostics::default();
    let ports = open_database(database, diagnostics.clone()).await.unwrap();
    tests::sql(&admin, "INSERT INTO agena_workspaces (id,path,created_at_ms,updated_at_ms) VALUES (1,'coherent control fixture',0,0)").await;
    let (session, round, tool) = event_tests::tool(&ports).await;
    let interaction = tool
        .request_interaction(
            RequestId::new(),
            InteractionRequest::Permission {
                action_key: "snapshot permission".into(),
                description: "commit during a pinned control read".into(),
            },
        )
        .await
        .unwrap()
        .value;
    let original = ports.reader.control_snapshot(session, 128).await.unwrap();
    let execution = original.active_executions.executions[0].id;
    let mut transaction = reader.begin().await.unwrap();
    // A real first SELECT pins SQLite's read version. The kernel writer then
    // commits every mutation before the remaining snapshot fields are read.
    let version: i64 = sqlx::query_scalar("SELECT control_version FROM agena_sessions WHERE id=?")
        .bind(session.get())
        .fetch_one(&mut *transaction)
        .await
        .unwrap();
    ports
        .sessions
        .update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                title: Some("new metadata".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    ports
        .sessions
        .update_selection(
            RequestId::new(),
            session,
            SelectionChange::ThinkingMode(Some("new selection".into())),
        )
        .await
        .unwrap();
    ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange {
                effective_workspace_root: Some(Some(root.path().join("new workspace"))),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    ports
        .sessions
        .reply(
            RequestId::new(),
            session,
            interaction,
            tests::permission_decision(interaction, true, None),
            None,
        )
        .await
        .unwrap();
    ports
        .sessions
        .cancel(RequestId::new(), session, execution)
        .await
        .unwrap();

    let pinned = control_snapshot::read_on(&mut transaction, session, 128, &diagnostics)
        .await
        .unwrap();
    assert_eq!(pinned.control_version, version);
    assert_eq!(pinned.metadata, original.metadata);
    assert_eq!(pinned.summary, original.summary);
    assert_eq!(pinned.configuration, original.configuration);
    assert_eq!(
        pinned.active_executions.executions,
        original.active_executions.executions
    );
    assert_eq!(pinned.pending_interactions.interactions[0].id, interaction);
    assert_eq!(pinned.active_execution, original.active_execution);
    transaction.rollback().await.unwrap();

    let current = ports.reader.control_snapshot(session, 128).await.unwrap();
    assert!(current.control_version > version);
    assert_eq!(current.metadata.title, "new metadata");
    assert_eq!(current.summary.title, "new metadata");
    assert_ne!(current.metadata.selection, original.metadata.selection);
    assert_eq!(
        current.configuration.effective_workspace_root,
        Some(root.path().join("new workspace"))
    );
    assert!(current.pending_interactions.interactions.is_empty());
    assert!(current.active_executions.executions.is_empty());
    assert!(current.active_execution.is_none());
    drop(round);
    ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn control_snapshot_pages_stay_bounded_and_retained_views_release_sql_and_admission() {
    for memory in [true, false] {
        let diagnostics = KernelDiagnostics::default();
        let (ports, _admin, _root, _url) =
            tests::fixture_with_diagnostics(memory, diagnostics.clone()).await;
        let (session, _round, tool) = event_tests::tool(&ports).await;
        for index in 0..3 {
            tool.request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: format!("permission-{index}"),
                    description: "bounded status page".into(),
                },
            )
            .await
            .unwrap();
            ports
                .sessions
                .send(
                    RequestId::new(),
                    session,
                    document("queued behind current work"),
                )
                .await
                .unwrap();
        }
        let one = ports.reader.control_snapshot(session, 1).await.unwrap();
        let zero = ports.reader.control_snapshot(session, 0).await.unwrap();
        assert!(one.ptr_eq(&zero));
        assert_eq!(one.pending_interactions.interactions.len(), 1);
        assert!(one.pending_interactions.has_more);
        assert!(one.pending_interactions.next.is_some());
        assert_eq!(one.active_executions.executions.len(), 1);
        assert!(one.active_executions.has_more);
        assert!(one.active_executions.next.is_some());
        let all = ports
            .reader
            .control_snapshot(session, usize::MAX)
            .await
            .unwrap();
        assert!(all.ptr_eq(&ports.reader.control_snapshot(session, 128).await.unwrap()));
        assert_eq!(all.pending_interactions.interactions.len(), 3);
        assert!(!all.pending_interactions.has_more);
        assert_eq!(all.active_executions.executions.len(), 4);
        let mut retained = vec![one, zero, all];
        tokio::time::timeout(Duration::from_secs(10), async {
            for limit in 2..14 {
                retained.push(ports.reader.control_snapshot(session, limit).await.unwrap());
            }
            ports.sessions.update_metadata(RequestId::new(), session, MetadataChange {
                title: Some("views retain no SQL transaction".into()),
                ..Default::default()
            }).await.unwrap();
        }).await.expect("more than eight retained views must not retain the control lane or singleton SQL connection");
        let end = diagnostics.snapshot();
        assert_eq!(
            end.read_transactions_started,
            end.read_transactions_completed
        );
        assert_eq!(end.read_transaction_active_scopes, 0);
        assert_eq!(retained[0].metadata.title, "events");
        assert_eq!(
            ports
                .reader
                .control_snapshot(session, 128)
                .await
                .unwrap()
                .metadata
                .title,
            "views retain no SQL transaction"
        );
        let missing = SessionId::try_from(i64::MAX).unwrap();
        let before_error = diagnostics.snapshot();
        assert!(matches!(
            ports.reader.control_snapshot(missing, 128).await,
            Err(KernelError::NotFound(_))
        ));
        let after_error = diagnostics.snapshot();
        assert_eq!(
            after_error.read_transactions_started,
            before_error.read_transactions_started + 1
        );
        assert_eq!(
            after_error.read_transactions_abandoned,
            before_error.read_transactions_abandoned + 1
        );
        assert_eq!(
            after_error.read_transactions_completed,
            before_error.read_transactions_completed
        );
        assert_eq!(
            after_error.read_transaction_duration_us_log2,
            before_error.read_transaction_duration_us_log2
        );
        assert_eq!(after_error.read_transaction_active_scopes, 0);
        ports.shutdown().await.unwrap();
    }
}
