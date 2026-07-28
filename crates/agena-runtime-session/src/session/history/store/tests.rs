use super::*;

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::*;
    use crate::session::history::AssistantMessageFinished;
    use agena_domain::FinishReason;
    use agena_storage::WorkspaceRepository;
    use sea_orm::{ActiveModelTrait, Database, EntityTrait, Set};

    #[test]
    fn projected_header_decodes_storage_record_and_rejects_inconsistent_turn() {
        let record = MessageProjectionHeaderRecord {
            message_id: 41,
            turn_id: Some(7),
            role: Role::Assistant,
            state: ExecutionStatus::Completed,
            created_at_ms: 1,
            metadata: serde_json::to_value(crate::message::MessageMetadata {
                turn_id: Some(7),
                ..Default::default()
            })
            .expect("serialize metadata"),
            provider_state: None,
            usage: Some(
                serde_json::to_value(agena_provider::CompletionUsage {
                    output_tokens: 3,
                    ..Default::default()
                })
                .expect("serialize usage"),
            ),
            part_count: 2,
        };
        let header = projected_message_header_from_record(record.clone()).expect("header");
        assert_eq!(header.id, 41);
        assert_eq!(header.metadata.turn_id, Some(7));
        assert_eq!(header.usage.expect("usage").output_tokens, 3);
        assert_eq!(header.part_count, 2);

        let mut inconsistent = record;
        inconsistent.turn_id = Some(8);
        assert!(
            projected_message_header_from_record(inconsistent)
                .expect_err("turn identity mismatch")
                .to_string()
                .contains("inconsistent turn identity")
        );
    }

    #[test]
    fn compaction_activity_is_a_distinct_non_conversation_part() {
        let created_at = Utc::now();
        let activity = agena_domain::PromptCompactionActivity {
            checkpoint_id: "checkpoint-1".to_owned(),
            generation: 2,
            compacted_through_message_id: 40,
            trigger: agena_domain::PromptCompactionTrigger::Manual,
            strategy: agena_domain::PromptCompactionStrategy::LocalSummary,
            before_tokens: 10_000,
            after_tokens: 2_500,
        };
        let execution_id = agena_domain::ExecutionId::new();
        let mut projected = crate::message::ActivityPart::execution(
            execution_id,
            agena_domain::ExecutionSource::Compaction,
            created_at.timestamp_millis(),
        );
        projected.apply_compaction(execution_id, activity.clone());
        projected.complete_execution(created_at.timestamp_millis());
        let mut message = crate::message::Message::prompt_parts(
            Role::System,
            vec![crate::message::PartContent::Activity(projected)],
        );
        message.id = 41;
        message.metadata.source = MessageSource::System;
        message.parts[0].id = 51;
        message.parts[0].message_id = 41;

        assert_eq!(message.id, 41);
        assert_eq!(message.metadata.source, MessageSource::System);
        assert!(message.is_activity());
        let part = message.parts.into_iter().next().expect("activity part");
        assert_eq!(part.id, 51);
        assert_eq!(part.message_id, 41);
        let Some(crate::message::PartContent::Activity(projected)) = part.content else {
            panic!("expected compaction activity")
        };
        let crate::message::ActivityKind::Compaction {
            activity: projected_activity,
            ..
        } = projected.kind
        else {
            panic!("expected typed compaction activity")
        };
        assert_eq!(projected_activity, activity);
    }

    #[tokio::test]
    async fn execution_activity_updates_in_place_and_never_projects_to_a_model() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test/activity")
            .await
            .expect("workspace");
        let session = crate::db::crud::session::create_session(&db, workspace_id, None, "activity")
            .await
            .expect("session");
        let execution_id = agena_domain::ExecutionId::new();
        let writer = RuntimeProjectionPartWriter;
        project_execution_started(
            &db,
            &writer,
            &ExecutionStartedEvent {
                session_id: session.id,
                execution_id,
                activity_message_id: agena_domain::MessageId(41),
                activity_part_id: agena_domain::PartId(51),
                source: agena_domain::ExecutionSource::User,
                ts_ms: 10,
            },
        )
        .await
        .expect("started activity");

        let started = activity_part::Entity::find_by_id(51)
            .one(&db)
            .await
            .expect("query started")
            .expect("started part");
        assert_eq!(started.status, StoredExecutionStatus::InProgress);
        assert!(
            activity_message::Entity::find_by_id(41)
                .one(&db)
                .await
                .expect("query started message")
                .expect("started message")
                .is_hidden,
            "normal submit activity stays latent while the optimistic user message is visible"
        );
        let Some(PartContent::Activity(started_activity)) = started.content else {
            panic!("expected typed activity")
        };
        let mut message = crate::message::Message::prompt_parts(
            Role::System,
            vec![PartContent::Activity(started_activity)],
        );
        message.id = 41;
        assert!(crate::provider::project_completion_input(&message).is_none());

        project_execution_finished(
            &db,
            &writer,
            &ExecutionFinishedEvent {
                session_id: session.id,
                execution_id,
                outcome: ExecutionOutcome::Failed {
                    failure_kind: ExecutionFailureKind::Provider,
                    message: "provider unavailable".to_owned(),
                },
                ts_ms: 20,
            },
        )
        .await
        .expect("failed activity");

        let failed = activity_part::Entity::find_by_id(51)
            .one(&db)
            .await
            .expect("query failed")
            .expect("failed part");
        assert_eq!(failed.status, StoredExecutionStatus::Failed);
        assert!(
            !activity_message::Entity::find_by_id(41)
                .one(&db)
                .await
                .expect("query failed message")
                .expect("failed message")
                .is_hidden,
            "terminal failures must become visible transcript evidence"
        );
        let Some(PartContent::Activity(failed_activity)) = failed.content else {
            panic!("expected typed activity")
        };
        assert_eq!(failed_activity.activity_id, execution_id.to_string());
        assert_eq!(
            failed_activity.error.expect("activity error").message,
            "provider unavailable"
        );
    }

    #[tokio::test]
    async fn manual_compaction_enriches_the_execution_activity_in_place() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test/compact-activity")
            .await
            .expect("workspace");
        let session =
            crate::db::crud::session::create_session(&db, workspace_id, None, "compact activity")
                .await
                .expect("session");
        let execution_id = agena_domain::ExecutionId::new();
        let writer = RuntimeProjectionPartWriter;
        project_execution_started(
            &db,
            &writer,
            &ExecutionStartedEvent {
                session_id: session.id,
                execution_id,
                activity_message_id: agena_domain::MessageId(61),
                activity_part_id: agena_domain::PartId(62),
                source: agena_domain::ExecutionSource::Compaction,
                ts_ms: 10,
            },
        )
        .await
        .expect("started activity");
        let compacted = agena_domain::PromptCompactionActivity {
            checkpoint_id: "checkpoint".to_owned(),
            generation: 2,
            compacted_through_message_id: 40,
            trigger: agena_domain::PromptCompactionTrigger::Manual,
            strategy: agena_domain::PromptCompactionStrategy::LocalSummary,
            before_tokens: 10_000,
            after_tokens: 2_500,
        };
        project_compaction_completed(
            &db,
            &writer,
            &agena_domain::PromptCompactionCompletedEvent {
                session_id: session.id,
                execution_id,
                standalone_message_id: None,
                standalone_part_id: None,
                activity: compacted.clone(),
                ts_ms: 20,
            },
        )
        .await
        .expect("compaction details");
        project_execution_finished(
            &db,
            &writer,
            &ExecutionFinishedEvent {
                session_id: session.id,
                execution_id,
                outcome: ExecutionOutcome::Completed,
                ts_ms: 21,
            },
        )
        .await
        .expect("completed activity");

        assert!(
            activity_message::Entity::find_by_id(71)
                .one(&db)
                .await
                .expect("query unused message")
                .is_none(),
            "manual compaction must update the execution activity instead of duplicating it"
        );
        let part = activity_part::Entity::find_by_id(62)
            .one(&db)
            .await
            .expect("query compact activity")
            .expect("compact activity");
        assert_eq!(part.status, StoredExecutionStatus::Completed);
        let Some(PartContent::Activity(activity)) = part.content else {
            panic!("expected typed activity")
        };
        let ActivityKind::Compaction {
            activity: projected,
            ..
        } = activity.kind
        else {
            panic!("expected compaction details")
        };
        assert_eq!(projected, compacted);
    }

    #[tokio::test]
    async fn automatic_compaction_creates_a_separate_activity_from_the_outer_execution() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test/automatic-compact-activity")
            .await
            .expect("workspace");
        let session = crate::db::crud::session::create_session(
            &db,
            workspace_id,
            None,
            "automatic compact activity",
        )
        .await
        .expect("session");
        let execution_id = agena_domain::ExecutionId::new();
        let writer = RuntimeProjectionPartWriter;
        project_execution_started(
            &db,
            &writer,
            &ExecutionStartedEvent {
                session_id: session.id,
                execution_id,
                activity_message_id: agena_domain::MessageId(81),
                activity_part_id: agena_domain::PartId(82),
                source: agena_domain::ExecutionSource::User,
                ts_ms: 10,
            },
        )
        .await
        .expect("outer execution activity");
        let compacted = agena_domain::PromptCompactionActivity {
            checkpoint_id: "automatic-checkpoint".to_owned(),
            generation: 3,
            compacted_through_message_id: 40,
            trigger: agena_domain::PromptCompactionTrigger::Auto,
            strategy: agena_domain::PromptCompactionStrategy::LocalSummary,
            before_tokens: 12_000,
            after_tokens: 3_000,
        };

        project_compaction_completed(
            &db,
            &writer,
            &agena_domain::PromptCompactionCompletedEvent {
                session_id: session.id,
                execution_id,
                standalone_message_id: Some(agena_domain::MessageId(91)),
                standalone_part_id: Some(agena_domain::PartId(92)),
                activity: compacted.clone(),
                ts_ms: 20,
            },
        )
        .await
        .expect("automatic compaction activity");

        let outer = activity_part::Entity::find_by_id(82)
            .one(&db)
            .await
            .expect("query outer activity")
            .expect("outer activity");
        let Some(PartContent::Activity(outer)) = outer.content else {
            panic!("expected typed outer activity")
        };
        assert!(matches!(
            outer.kind,
            ActivityKind::Execution {
                source: agena_domain::ExecutionSource::User,
                ..
            }
        ));

        let standalone_message = activity_message::Entity::find_by_id(91)
            .one(&db)
            .await
            .expect("query standalone message")
            .expect("standalone message");
        assert_eq!(standalone_message.state, StoredExecutionStatus::Completed);
        assert!(!standalone_message.is_hidden);
        let standalone = activity_part::Entity::find_by_id(92)
            .one(&db)
            .await
            .expect("query standalone activity")
            .expect("standalone activity");
        assert_eq!(standalone.status, StoredExecutionStatus::Completed);
        assert_eq!(
            standalone.operation_id.as_deref(),
            Some("compaction:automatic-checkpoint")
        );
        let Some(PartContent::Activity(standalone_activity)) = standalone.content else {
            panic!("expected typed standalone activity")
        };
        let ActivityKind::Compaction { ref activity, .. } = standalone_activity.kind else {
            panic!("expected compaction details")
        };
        assert_eq!(activity, &compacted);

        let mut provider_message = crate::message::Message::prompt_parts(
            Role::System,
            vec![PartContent::Activity(standalone_activity)],
        );
        provider_message.id = 91;
        assert!(crate::provider::project_completion_input(&provider_message).is_none());
    }

    #[tokio::test]
    async fn message_turn_and_execution_identity_are_persisted_and_immutable() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test")
            .await
            .expect("workspace");
        let session = crate::db::crud::session::create_session(&db, workspace_id, None, "test")
            .await
            .expect("session");
        let metadata = crate::message::MessageMetadata {
            turn_id: Some(7),
            ..Default::default()
        };
        let row = activity_message::Model {
            message_id: 41,
            session_id: session.id,
            turn_id: Some(7),
            execution_id: Some("execution-1".to_owned()),
            run_id: Some("run-1".to_owned()),
            role: Role::Assistant.into(),
            state: StoredExecutionStatus::Completed,
            created_at_ms: 1,
            updated_at_ms: 1,
            metadata,
            provider_state: None,
            usage: None,
            part_count: 0,
            is_hidden: false,
        };

        upsert_message_projection(&db, row.clone())
            .await
            .expect("project message");
        let stored = activity_message::Entity::find_by_id(41)
            .one(&db)
            .await
            .expect("query message")
            .expect("stored message");
        assert_eq!(stored.turn_id, Some(7));
        assert_eq!(stored.metadata.turn_id, Some(7));
        assert_eq!(stored.execution_id.as_deref(), Some("execution-1"));
        assert_eq!(stored.run_id.as_deref(), Some("run-1"));

        let mut changed = row.clone();
        changed.turn_id = Some(8);
        changed.metadata.turn_id = Some(8);
        let error = upsert_message_projection(&db, changed)
            .await
            .expect_err("turn identity must be immutable");
        assert!(error.to_string().contains("turn identity is immutable"));

        let mut inconsistent = row;
        inconsistent.turn_id = Some(8);
        let error = upsert_message_projection(&db, inconsistent)
            .await
            .expect_err("column and metadata must agree");
        assert!(error.to_string().contains("inconsistent turn identity"));
    }

    #[tokio::test]
    async fn terminal_projection_preserves_checkpoint_creation_time() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test")
            .await
            .expect("workspace");
        let session = crate::db::crud::session::create_session(&db, workspace_id, None, "test")
            .await
            .expect("session");
        let execution_id = agena_domain::ExecutionId::new();
        let run_id = RunId::new();
        let terminal_created_at = Utc::now();
        let checkpoint_created_at_ms = terminal_created_at.timestamp_millis() - 1;
        let metadata = crate::message::MessageMetadata {
            turn_id: Some(41),
            source: MessageSource::Assistant,
            ..Default::default()
        };

        activity_message::ActiveModel {
            message_id: Set(41),
            session_id: Set(session.id),
            turn_id: Set(Some(41)),
            execution_id: Set(Some(execution_id.to_string())),
            run_id: Set(Some(run_id.to_string())),
            role: Set(Role::Assistant.into()),
            state: Set(StoredExecutionStatus::InProgress),
            created_at_ms: Set(checkpoint_created_at_ms),
            updated_at_ms: Set(checkpoint_created_at_ms),
            metadata: Set(metadata.clone()),
            provider_state: Set(None),
            usage: Set(None),
            part_count: Set(0),
            is_hidden: Set(false),
        }
        .insert(&db)
        .await
        .expect("checkpoint projection");

        apply_projection_events_on_connection(
            &db,
            &RuntimeProjectionPartWriter,
            session.id,
            &[DomainEvent {
                meta: agena_domain::EventMeta {
                    id: uuid::Uuid::new_v4(),
                    seq_global: 1,
                    seq_session: Some(1),
                    session_id: Some(session.id),
                    workspace_id: Some(workspace_id),
                    created_at: terminal_created_at,
                    causation_id: None,
                    correlation_id: None,
                    envelope_schema: agena_domain::EVENT_ENVELOPE_SCHEMA_VERSION,
                },
                kind: EventKind::AssistantMessageFinished(AssistantMessageFinished {
                    execution_id,
                    message_id: agena_domain::MessageId(41),
                    run_id,
                    created_at: terminal_created_at,
                    content: Default::default(),
                    status: ExecutionStatus::Completed,
                    parts: Vec::new(),
                    usage: None,
                    finish_reason: FinishReason::Stop,
                    metadata,
                    provider_state: None,
                }),
            }],
        )
        .await
        .expect("project terminal event with legacy timestamp drift");

        let projected = activity_message::Entity::find_by_id(41)
            .one(&db)
            .await
            .expect("query message")
            .expect("projected message");
        assert_eq!(projected.created_at_ms, checkpoint_created_at_ms);
        assert_eq!(projected.state, StoredExecutionStatus::Completed);
    }

    #[tokio::test]
    async fn tool_lifecycle_preserves_the_assistant_operation_part_identity() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test")
            .await
            .expect("workspace");
        let session = crate::db::crud::session::create_session(&db, workspace_id, None, "test")
            .await
            .expect("session");
        let created_at = Utc::now();
        let run_id = RunId::new();
        let call_id = agena_domain::ToolCallId::new("call_1");

        activity_message::ActiveModel {
            message_id: Set(41),
            session_id: Set(session.id),
            turn_id: Set(None),
            execution_id: Set(None),
            run_id: Set(Some(run_id.to_string())),
            role: Set(StoredRole::Assistant),
            state: Set(StoredExecutionStatus::Completed),
            created_at_ms: Set(created_at.timestamp_millis()),
            updated_at_ms: Set(created_at.timestamp_millis()),
            metadata: Set(Default::default()),
            provider_state: Set(None),
            usage: Set(None),
            part_count: Set(1),
            is_hidden: Set(false),
        }
        .insert(&db)
        .await
        .expect("message");

        let mut operation_part = MessagePart::from_content(
            51,
            41,
            created_at,
            ExecutionStatus::Pending,
            crate::message::PartContent::Operation(crate::message::OperationPart::pending(
                1,
                agena_domain::ToolInvocation::new(
                    "tools_list",
                    agena_domain::StructuredObject::default(),
                ),
                "tools_list",
                agena_domain::TimeRange::default(),
            )),
        );
        operation_part.operation_id = Some(call_id.to_string());
        upsert_part_projection(&db, session.id, &operation_part)
            .await
            .expect("original operation part");

        let part_writer = RuntimeProjectionPartWriter;
        project_tool_call_issued(
            &db,
            &part_writer,
            session.id,
            &crate::session::history::ToolCallIssued {
                message_id: agena_domain::MessageId(41),
                run_id,
                call_id: call_id.clone(),
                name: "tools_list".into(),
                arguments: serde_json::json!({}),
                created_at,
            },
        )
        .await
        .expect("project issued call");

        let projected = activity_part::Entity::find()
            .filter(activity_part::Column::MessageId.eq(41))
            .all(&db)
            .await
            .expect("projected parts");
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].part_id, 51);
        assert_eq!(projected[0].operation_id.as_deref(), Some("call_1"));

        let mut completed_part = operation_part.clone();
        completed_part.status = ExecutionStatus::Completed;
        update_tool_result_projection(
            &db,
            &part_writer,
            session.id,
            &crate::session::history::ToolCallCompleted {
                message_id: agena_domain::MessageId(41),
                call_id,
                run_id,
                tool_name: "tools_list".into(),
                part: completed_part,
                completed_at: Utc::now(),
            },
        )
        .await
        .expect("project completed call");

        let projected = activity_part::Entity::find()
            .filter(activity_part::Column::MessageId.eq(41))
            .all(&db)
            .await
            .expect("completed parts");
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].part_id, 51);
        assert_eq!(projected[0].status, StoredExecutionStatus::Completed);
    }

    #[tokio::test]
    async fn operation_identity_cannot_be_rebound_to_another_part() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test")
            .await
            .expect("workspace");
        let session = crate::db::crud::session::create_session(&db, workspace_id, None, "test")
            .await
            .expect("session");
        let created_at = Utc::now();

        activity_message::ActiveModel {
            message_id: Set(41),
            session_id: Set(session.id),
            turn_id: Set(None),
            execution_id: Set(None),
            run_id: Set(None),
            role: Set(Role::Assistant.into()),
            state: Set(StoredExecutionStatus::Completed),
            created_at_ms: Set(created_at.timestamp_millis()),
            updated_at_ms: Set(created_at.timestamp_millis()),
            metadata: Set(Default::default()),
            provider_state: Set(None),
            usage: Set(None),
            part_count: Set(1),
            is_hidden: Set(false),
        }
        .insert(&db)
        .await
        .expect("message");

        let operation =
            crate::message::PartContent::Operation(crate::message::OperationPart::pending(
                1,
                agena_domain::ToolInvocation::new(
                    "tools_list",
                    agena_domain::StructuredObject::default(),
                ),
                "tools_list",
                agena_domain::TimeRange::default(),
            ));
        let mut original = MessagePart::from_content(
            51,
            41,
            created_at,
            ExecutionStatus::Pending,
            operation.clone(),
        );
        original.operation_id = Some("call_1".to_owned());
        upsert_part_projection(&db, session.id, &original)
            .await
            .expect("original operation");

        let mut conflicting =
            MessagePart::from_content(52, 41, created_at, ExecutionStatus::Pending, operation);
        conflicting.operation_id = Some("call_1".to_owned());
        let error = upsert_part_projection(&db, session.id, &conflicting)
            .await
            .expect_err("operation identity must be immutable");

        assert!(error.to_string().contains("already bound to part 51"));
        let projected = activity_part::Entity::find()
            .filter(activity_part::Column::MessageId.eq(41))
            .all(&db)
            .await
            .expect("projected parts");
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].part_id, 51);

        let database_error = activity_part::ActiveModel {
            part_id: Set(52),
            message_id: Set(41),
            part_index: Set(1),
            status: Set(conflicting.status.into()),
            kind: Set(conflicting.kind.into()),
            name: Set(conflicting.name.clone()),
            summary: Set(conflicting.summary.clone()),
            has_detail: Set(conflicting.has_detail),
            operation_id: Set(conflicting.operation_id.clone()),
            created_at_ms: Set(conflicting.created_at.timestamp_millis()),
            content: Set(conflicting.content.clone()),
        }
        .insert(&db)
        .await
        .expect_err("database must enforce operation identity uniqueness");
        assert!(
            database_error
                .to_string()
                .contains("UNIQUE constraint failed")
        );
    }

    #[tokio::test]
    async fn execution_finish_closes_open_artifacts_and_late_checkpoint_cannot_reopen_them() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test")
            .await
            .expect("workspace");
        let session = crate::db::crud::session::create_session(&db, workspace_id, None, "test")
            .await
            .expect("session");
        let execution_id = agena_domain::ExecutionId::new();
        let run_id = RunId::new();
        let metadata = crate::message::MessageMetadata {
            turn_id: Some(41),
            ..Default::default()
        };

        activity_message::ActiveModel {
            message_id: Set(41),
            session_id: Set(session.id),
            turn_id: Set(Some(41)),
            execution_id: Set(Some(execution_id.to_string())),
            run_id: Set(Some(run_id.to_string())),
            role: Set(Role::Assistant.into()),
            state: Set(StoredExecutionStatus::InProgress),
            created_at_ms: Set(1),
            updated_at_ms: Set(1),
            metadata: Set(metadata),
            provider_state: Set(None),
            usage: Set(None),
            part_count: Set(1),
            is_hidden: Set(false),
        }
        .insert(&db)
        .await
        .expect("message");
        activity_part::ActiveModel {
            part_id: Set(51),
            message_id: Set(41),
            part_index: Set(0),
            status: Set(StoredExecutionStatus::InProgress),
            kind: Set(StoredPartKind::Text),
            name: Set(None),
            summary: Set(None),
            has_detail: Set(false),
            operation_id: Set(None),
            created_at_ms: Set(1),
            content: Set(None),
        }
        .insert(&db)
        .await
        .expect("part");

        let part_writer = RuntimeProjectionPartWriter;
        project_execution_started(
            &db,
            &part_writer,
            &ExecutionStartedEvent {
                session_id: session.id,
                execution_id,
                activity_message_id: agena_domain::MessageId(61),
                activity_part_id: agena_domain::PartId(62),
                source: agena_domain::ExecutionSource::User,
                ts_ms: 1,
            },
        )
        .await
        .expect("project execution activity");
        apply_projection_events_on_connection(
            &db,
            &part_writer,
            session.id,
            &[DomainEvent {
                meta: agena_domain::EventMeta {
                    id: uuid::Uuid::new_v4(),
                    seq_global: 1,
                    seq_session: Some(1),
                    session_id: Some(session.id),
                    workspace_id: Some(workspace_id),
                    created_at: Utc::now(),
                    causation_id: None,
                    correlation_id: None,
                    envelope_schema: agena_domain::EVENT_ENVELOPE_SCHEMA_VERSION,
                },
                kind: EventKind::ExecutionFinished(ExecutionFinishedEvent {
                    session_id: session.id,
                    execution_id,
                    outcome: ExecutionOutcome::Completed,
                    ts_ms: Utc::now().timestamp_millis(),
                }),
            }],
        )
        .await
        .expect("terminalize");

        let terminal_message = activity_message::Entity::find_by_id(41)
            .one(&db)
            .await
            .expect("query terminal message")
            .expect("message exists");
        let terminal_part = activity_part::Entity::find_by_id(51)
            .one(&db)
            .await
            .expect("query terminal part")
            .expect("part exists");
        assert_eq!(terminal_message.state, StoredExecutionStatus::Failed);
        assert_eq!(terminal_part.status, StoredExecutionStatus::Failed);

        // Model a terminal assistant whose tool part was closed by the
        // execution boundary. Parent state alone must not let a delayed part
        // checkpoint reopen that tool.
        let mut terminal_message_update: activity_message::ActiveModel = terminal_message.into();
        terminal_message_update.state = Set(StoredExecutionStatus::Completed);
        terminal_message_update
            .update(&db)
            .await
            .expect("set completed parent");

        let mut late_part = MessagePart::from_content(
            51,
            41,
            Utc::now(),
            ExecutionStatus::InProgress,
            crate::message::PartContent::text("late checkpoint"),
        );
        late_part.part_index = 0;
        apply_message_part_update_on_connection(
            &db,
            &part_writer,
            &MessagePartCheckpointedEvent {
                session_id: session.id,
                execution_id: Some(execution_id),
                run_id: Some(run_id),
                message_id: 41,
                message_role: Role::Assistant,
                message_state: ExecutionStatus::Completed,
                message_created_at: Utc::now(),
                message_metadata: crate::message::MessageMetadata {
                    turn_id: Some(41),
                    ..Default::default()
                },
                part: late_part,
                ts_ms: Utc::now().timestamp_millis(),
            },
        )
        .await
        .expect("ignore stale checkpoint");

        let message = activity_message::Entity::find_by_id(41)
            .one(&db)
            .await
            .expect("query message")
            .expect("message exists");
        let part = activity_part::Entity::find_by_id(51)
            .one(&db)
            .await
            .expect("query part")
            .expect("part exists");
        assert_eq!(message.state, StoredExecutionStatus::Completed);
        assert_eq!(part.status, StoredExecutionStatus::Failed);
    }
}
