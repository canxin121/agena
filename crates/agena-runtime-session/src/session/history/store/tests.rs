use super::*;

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use super::*;
    use crate::session::history::AssistantMessageFinished;
    use agena_domain::FinishReason;
    use agena_storage::WorkspaceRepository;
    use sea_orm::{ActiveModelTrait, Database, EntityTrait, PaginatorTrait, Set};

    fn provider_execution_problem() -> agena_failure::UserProblem {
        agena_failure::Failure::new(
            agena_failure::FailureCode::new("provider.unavailable"),
            agena_failure::FailureCategory::DependencyUnavailable,
            agena_failure::FailureResponsibility::Dependency,
            agena_failure::RetryDirective::Backoff,
            agena_failure::RecoveryDirective::Retry,
            agena_failure::FailureImpact::OperationFailed,
            agena_failure::UserPresentation::new(
                "provider-unavailable",
                "The provider is temporarily unavailable.",
            ),
        )
        .into()
    }

    #[tokio::test]
    async fn database_projection_fence_serializes_independent_callers() {
        let directory = tempfile::tempdir().expect("temporary database directory");
        let path = directory.path().join("projection-fence.sqlite");
        let db = Database::connect(format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .expect("file database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test/projection-fence")
            .await
            .expect("workspace");
        let session =
            crate::db::crud::session::create_session(&db, workspace_id, None, "projection fence")
                .await
                .expect("session");

        let first = db.begin().await.expect("first transaction");
        acquire_projection_fence(&first, session.id)
            .await
            .expect("first fence");

        let second_db = db.clone();
        let session_id = session.id;
        let second = tokio::spawn(async move {
            let transaction = second_db.begin().await.expect("second transaction");
            acquire_projection_fence(&transaction, session_id)
                .await
                .expect("second fence");
            let watermark = transcript_projection_state::Entity::find_by_id(session_id)
                .one(&transaction)
                .await
                .expect("read fenced watermark")
                .expect("projection state")
                .last_seq_global;
            transaction.commit().await.expect("commit second");
            watermark
        });
        tokio::task::yield_now().await;
        assert!(
            !second.is_finished(),
            "the second independent caller must wait for the database fence"
        );

        first
            .execute(Statement::from_sql_and_values(
                first.get_database_backend(),
                "UPDATE agena_transcript_projection_states SET last_seq_global = 10 WHERE session_id = ?"
                    .to_owned(),
                [session.id.into()],
            ))
            .await
            .expect("advance watermark");
        first.commit().await.expect("commit first");

        let observed = tokio::time::timeout(std::time::Duration::from_secs(5), second)
            .await
            .expect("second caller released")
            .expect("second task");
        assert_eq!(observed, 10, "stale caller must re-read winner's watermark");
    }

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

    #[tokio::test]
    async fn executions_project_owned_responses_without_system_messages() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test/response")
            .await
            .expect("workspace");
        let session = crate::db::crud::session::create_session(&db, workspace_id, None, "response")
            .await
            .expect("session");
        let execution_id = agena_domain::ExecutionId::new();
        let turn_id = agena_domain::TurnId::new();
        let response_id = agena_domain::ResponseId::new();
        let writer = RuntimeProjectionPartWriter;
        project_execution_started(
            &db,
            &writer,
            &ExecutionStartedEvent {
                session_id: session.id,
                execution_id,
                turn_id,
                response_id,
                source: agena_domain::ExecutionSource::User,
                ts_ms: 10,
            },
            1,
        )
        .await
        .expect("started response");

        assert_eq!(
            transcript_message::Entity::find()
                .count(&db)
                .await
                .expect("count transcript messages"),
            0,
            "execution lifecycle must not synthesize a transcript message"
        );
        let started = db
            .query_one(Statement::from_sql_and_values(
                db.get_database_backend(),
                "SELECT turn_id, execution_id, status, revision_seq, finished_at_ms FROM agena_responses WHERE response_id = ?",
                [response_id.to_string().into()],
            ))
            .await
            .expect("query response")
            .expect("started response");
        assert_eq!(
            started.try_get::<String>("", "turn_id").unwrap(),
            turn_id.to_string()
        );
        assert_eq!(
            started.try_get::<String>("", "execution_id").unwrap(),
            execution_id.to_string()
        );
        assert_eq!(
            started.try_get::<String>("", "status").unwrap(),
            "in_progress"
        );
        assert_eq!(started.try_get::<i64>("", "revision_seq").unwrap(), 1);
        assert_eq!(
            started
                .try_get::<Option<i64>>("", "finished_at_ms")
                .unwrap(),
            None
        );

        project_execution_finished(
            &db,
            &writer,
            &ExecutionFinishedEvent {
                session_id: session.id,
                execution_id,
                response_id,
                outcome: ExecutionOutcome::Failed {
                    failure: provider_execution_problem(),
                },
                ts_ms: 20,
            },
            2,
        )
        .await
        .expect("failed response");

        project_execution_finished(
            &db,
            &writer,
            &ExecutionFinishedEvent {
                session_id: session.id,
                execution_id,
                response_id,
                outcome: ExecutionOutcome::Failed {
                    failure: provider_execution_problem(),
                },
                ts_ms: 20,
            },
            2,
        )
        .await
        .expect("replaying the same terminal event is idempotent");

        let failed = db
            .query_one(Statement::from_sql_and_values(
                db.get_database_backend(),
                "SELECT status, revision_seq, finished_at_ms FROM agena_responses WHERE response_id = ?",
                [response_id.to_string().into()],
            ))
            .await
            .expect("query failed response")
            .expect("failed response");
        assert_eq!(failed.try_get::<String>("", "status").unwrap(), "failed");
        assert_eq!(failed.try_get::<i64>("", "revision_seq").unwrap(), 2);
        assert_eq!(
            failed.try_get::<Option<i64>>("", "finished_at_ms").unwrap(),
            Some(20)
        );
        assert_eq!(
            transcript_message::Entity::find()
                .count(&db)
                .await
                .expect("count terminal transcript messages"),
            0,
            "terminal execution state belongs to the response, not a duplicate system record"
        );
    }

    #[tokio::test]
    async fn mixed_user_content_projects_to_one_canonical_ordered_document() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test/mixed-content")
            .await
            .expect("workspace");
        let session =
            crate::db::crud::session::create_session(&db, workspace_id, None, "mixed content")
                .await
                .expect("session");
        let execution_id = agena_domain::ExecutionId::new();
        let turn_id = agena_domain::TurnId::new();
        let response_id = agena_domain::ResponseId::new();
        let writer = RuntimeProjectionPartWriter;
        project_execution_started(
            &db,
            &writer,
            &ExecutionStartedEvent {
                session_id: session.id,
                execution_id,
                turn_id,
                response_id,
                source: agena_domain::ExecutionSource::User,
                ts_ms: 1,
            },
            1,
        )
        .await
        .expect("response owner");

        let now = Utc::now();
        let mut first = MessagePart::from_content_with_index(
            1,
            1,
            0,
            now,
            ExecutionStatus::Completed,
            PartContent::text("hi "),
        );
        let first_segment_id = first.segment_id.expect("first text identity");
        let skill_id = agena_domain::ActivityId::new();
        let mut skill = MessagePart::from_content_with_index(
            2,
            1,
            1,
            now,
            ExecutionStatus::Completed,
            PartContent::Activity(crate::message::RuntimeActivity::SkillReference(
                crate::message::SkillReferencePart {
                    skills: vec![crate::message::SkillReference {
                        name: "batch".to_owned(),
                        description: "delegate independent work".to_owned(),
                        instructions: "Use isolated tasks.".to_owned(),
                        content_hash: "sha256:batch".to_owned(),
                        source: "test".to_owned(),
                        aliases: Vec::new(),
                    }],
                },
            )),
        );
        skill.activity_id = Some(skill_id);
        let mut second = MessagePart::from_content_with_index(
            3,
            1,
            2,
            now,
            ExecutionStatus::Completed,
            PartContent::text(" hi "),
        );
        let second_segment_id = second.segment_id.expect("second text identity");
        let directory_id = agena_domain::ActivityId::new();
        let mut directory = MessagePart::from_content_with_index(
            4,
            1,
            3,
            now,
            ExecutionStatus::Completed,
            PartContent::attachments(vec![crate::message::AttachmentItem {
                kind: crate::message::AttachmentKind::File,
                mime: "inode/directory".to_owned(),
                source: crate::message::AttachmentSource::LocalPath {
                    path: "apps".to_owned(),
                },
                filename: Some("apps".to_owned()),
                title: None,
                size_bytes: None,
                sha256: None,
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            }]),
        );
        directory.activity_id = Some(directory_id);

        for (revision, part) in [&mut first, &mut skill, &mut second, &mut directory]
            .into_iter()
            .enumerate()
        {
            project_part_content(&db, execution_id, Role::User, part, revision as i64 + 2)
                .await
                .expect("project canonical content node");
        }

        let rows = db
            .query_all(Statement::from_sql_and_values(
                db.get_database_backend(),
                "SELECT 'text' AS kind, segment_id AS id, position, NULL AS payload_json \
                 FROM agena_text_segments WHERE owner_kind = 'turn_input' AND owner_id = ? \
                 UNION ALL \
                 SELECT 'activity' AS kind, activity_id AS id, position, payload_json \
                 FROM agena_activities WHERE owner_kind = 'turn_input' AND owner_id = ? \
                 ORDER BY position",
                [turn_id.to_string().into(), turn_id.to_string().into()],
            ))
            .await
            .expect("canonical content rows");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].try_get::<String>("", "kind").unwrap(), "text");
        assert_eq!(
            rows[0].try_get::<String>("", "id").unwrap(),
            first_segment_id.to_string()
        );
        assert_eq!(
            rows[1].try_get::<String>("", "id").unwrap(),
            skill_id.to_string()
        );
        assert_eq!(
            rows[2].try_get::<String>("", "id").unwrap(),
            second_segment_id.to_string()
        );
        assert_eq!(
            rows[3].try_get::<String>("", "id").unwrap(),
            directory_id.to_string()
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.try_get::<i64>("", "position").unwrap())
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        let directory_payload: serde_json::Value = rows[3]
            .try_get("", "payload_json")
            .expect("directory payload");
        assert_eq!(directory_payload["activity_type"], "resource");
        assert_eq!(directory_payload["kind"], "directory");
        assert_eq!(
            directory_payload["reference"]["reference_type"],
            "workspace_path"
        );
        assert_eq!(directory_payload["reference"]["path"], "apps");
    }

    #[tokio::test]
    async fn compaction_is_one_runtime_activity_in_the_owning_response_document() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        agena_storage_sqlite::initialize_schema(&db)
            .await
            .expect("schema");
        let workspace_id = agena_storage_sqlite::SeaWorkspaceRepository::new(Arc::new(db.clone()))
            .ensure_id("/test/compaction-activity")
            .await
            .expect("workspace");
        let session =
            crate::db::crud::session::create_session(&db, workspace_id, None, "compaction")
                .await
                .expect("session");
        let execution_id = agena_domain::ExecutionId::new();
        let turn_id = agena_domain::TurnId::new();
        let response_id = agena_domain::ResponseId::new();
        let writer = RuntimeProjectionPartWriter;
        project_execution_started(
            &db,
            &writer,
            &ExecutionStartedEvent {
                session_id: session.id,
                execution_id,
                turn_id,
                response_id,
                source: agena_domain::ExecutionSource::Compaction,
                ts_ms: 10,
            },
            1,
        )
        .await
        .expect("start compaction response");

        let text = MessagePart::from_content_with_index(
            1,
            1,
            0,
            Utc::now(),
            ExecutionStatus::Completed,
            PartContent::text("continuation record"),
        );
        project_part_content(&db, execution_id, Role::Assistant, &text, 2)
            .await
            .expect("project response text before compaction activity");

        let activity_id = agena_domain::ActivityId::new();
        let event = PromptCompactionCompletedEvent {
            session_id: session.id,
            execution_id,
            activity_id,
            activity: agena_domain::PromptCompactionActivity {
                checkpoint_id: "checkpoint-1".to_owned(),
                generation: 1,
                compacted_through_message_id: 42,
                trigger: agena_domain::PromptCompactionTrigger::Manual,
                strategy: agena_domain::PromptCompactionStrategy::LocalSummary,
                before_tokens: 1_000,
                after_tokens: 400,
            },
            ts_ms: 20,
        };
        project_compaction_completed(&db, &writer, &event, 3)
            .await
            .expect("project compaction activity");

        assert_eq!(
            transcript_message::Entity::find()
                .count(&db)
                .await
                .expect("count transcript messages"),
            0,
            "compaction must not synthesize a System message"
        );
        let row = db
            .query_one(Statement::from_sql_and_values(
                db.get_database_backend(),
                "SELECT activity_id, owner_kind, owner_id, actor, payload_json, state, \
                        position, revision_seq, started_at_ms, finished_at_ms \
                 FROM agena_activities WHERE activity_id = ?",
                [activity_id.to_string().into()],
            ))
            .await
            .expect("query compaction activity")
            .expect("compaction activity");
        assert_eq!(
            row.try_get::<String>("", "activity_id").unwrap(),
            activity_id.to_string()
        );
        assert_eq!(row.try_get::<String>("", "owner_kind").unwrap(), "response");
        assert_eq!(
            row.try_get::<String>("", "owner_id").unwrap(),
            response_id.to_string()
        );
        assert_eq!(row.try_get::<String>("", "actor").unwrap(), "runtime");
        assert_eq!(row.try_get::<String>("", "state").unwrap(), "completed");
        assert_eq!(row.try_get::<i64>("", "position").unwrap(), 1);
        assert_eq!(row.try_get::<i64>("", "revision_seq").unwrap(), 3);
        assert_eq!(row.try_get::<i64>("", "started_at_ms").unwrap(), 20);
        assert_eq!(
            row.try_get::<Option<i64>>("", "finished_at_ms").unwrap(),
            Some(20)
        );
        let payload: serde_json::Value = row.try_get("", "payload_json").unwrap();
        assert_eq!(payload["activity_type"], "maintenance");
        assert_eq!(payload["maintenance_type"], "compaction");
        assert_eq!(payload["activity"]["checkpoint_id"], "checkpoint-1");
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
        let row = transcript_message::Model {
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
        };

        upsert_message_projection(&db, row.clone())
            .await
            .expect("project message");
        let stored = transcript_message::Entity::find_by_id(41)
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
    async fn terminal_projection_preserves_immutable_creation_time() {
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
        let checkpoint_created_at_ms = terminal_created_at.timestamp_millis();
        let metadata = crate::message::MessageMetadata {
            turn_id: Some(41),
            source: MessageSource::Assistant,
            ..Default::default()
        };

        transcript_message::ActiveModel {
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
        .expect("project terminal event with stable identity");

        let projected = transcript_message::Entity::find_by_id(41)
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

        transcript_message::ActiveModel {
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
        }
        .insert(&db)
        .await
        .expect("message");

        let mut operation_part = MessagePart::from_content(
            51,
            41,
            created_at,
            ExecutionStatus::Pending,
            crate::message::PartContent::operation(crate::message::OperationPart::pending(
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

        let projected = transcript_part::Entity::find()
            .filter(transcript_part::Column::MessageId.eq(41))
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

        let projected = transcript_part::Entity::find()
            .filter(transcript_part::Column::MessageId.eq(41))
            .all(&db)
            .await
            .expect("completed parts");
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].part_id, 51);
        assert_eq!(projected[0].status, StoredExecutionStatus::Completed);
    }

    #[tokio::test]
    async fn operation_correlation_can_span_distinct_activities() {
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

        transcript_message::ActiveModel {
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
        }
        .insert(&db)
        .await
        .expect("message");

        let operation =
            crate::message::PartContent::operation(crate::message::OperationPart::pending(
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
        upsert_part_projection(&db, session.id, &conflicting)
            .await
            .expect("a correlation id may be shared by separate activities");
        let projected = transcript_part::Entity::find()
            .filter(transcript_part::Column::MessageId.eq(41))
            .all(&db)
            .await
            .expect("projected parts");
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].part_id, 51);
        assert_eq!(projected[1].part_id, 52);
        assert_ne!(projected[0].activity_id, projected[1].activity_id);
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
        let response_id = agena_domain::ResponseId::new();
        let run_id = RunId::new();
        let metadata = crate::message::MessageMetadata {
            turn_id: Some(41),
            ..Default::default()
        };

        transcript_message::ActiveModel {
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
        }
        .insert(&db)
        .await
        .expect("message");
        transcript_part::ActiveModel {
            part_id: Set(51),
            message_id: Set(41),
            part_index: Set(0),
            status: Set(StoredExecutionStatus::InProgress),
            kind: Set(StoredPartKind::Text),
            name: Set(None),
            summary: Set(None),
            has_detail: Set(false),
            activity_id: Set(None),
            segment_id: Set(None),
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
                turn_id: agena_domain::TurnId::new(),
                response_id,
                source: agena_domain::ExecutionSource::User,
                ts_ms: 1,
            },
            1,
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
                    response_id,
                    outcome: ExecutionOutcome::Completed,
                    ts_ms: Utc::now().timestamp_millis(),
                }),
            }],
        )
        .await
        .expect("terminalize");

        let terminal_message = transcript_message::Entity::find_by_id(41)
            .one(&db)
            .await
            .expect("query terminal message")
            .expect("message exists");
        let terminal_part = transcript_part::Entity::find_by_id(51)
            .one(&db)
            .await
            .expect("query terminal part")
            .expect("part exists");
        assert_eq!(terminal_message.state, StoredExecutionStatus::Failed);
        assert_eq!(terminal_part.status, StoredExecutionStatus::Failed);

        // Model a terminal assistant whose tool part was closed by the
        // execution boundary. Parent state alone must not let a delayed part
        // checkpoint reopen that tool.
        let mut terminal_message_update: transcript_message::ActiveModel = terminal_message.into();
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
                turn_id: None,
                response_id: None,
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

        let message = transcript_message::Entity::find_by_id(41)
            .one(&db)
            .await
            .expect("query message")
            .expect("message exists");
        let part = transcript_part::Entity::find_by_id(51)
            .one(&db)
            .await
            .expect("query part")
            .expect("part exists");
        assert_eq!(message.state, StoredExecutionStatus::Completed);
        assert_eq!(part.status, StoredExecutionStatus::Failed);
    }
}
